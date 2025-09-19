use chrono::DateTime;
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde::Serialize;
use std::env;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use codex_protocol::mcp_protocol::AuthMode;

use crate::token_data::PlanType;
use crate::token_data::TokenData;
use crate::token_data::parse_id_token;

#[derive(Debug, Clone)]
pub struct CodexAuth {
    pub mode: AuthMode,

    pub(crate) api_key: Option<String>,
    pub(crate) active_account_index: Option<usize>,
    pub(crate) auth_dot_json: Arc<Mutex<Option<AuthDotJson>>>,
    pub(crate) auth_file: PathBuf,
    pub(crate) client: reqwest::Client,
}

impl PartialEq for CodexAuth {
    fn eq(&self, other: &Self) -> bool {
        self.mode == other.mode && self.active_account_index == other.active_account_index
    }
}

impl CodexAuth {
    pub async fn refresh_token(&self) -> Result<String, std::io::Error> {
        let token_data = self
            .get_current_token_data()
            .ok_or(std::io::Error::other("Token data is not available."))?;
        let token = token_data.refresh_token;

        let refresh_response = try_refresh_token(token, &self.client)
            .await
            .map_err(std::io::Error::other)?;

        let updated = update_tokens(
            &self.auth_file,
            self.resolve_account_index(),
            refresh_response.id_token,
            refresh_response.access_token,
            refresh_response.refresh_token,
        )
        .await?;

        if let Ok(mut auth_lock) = self.auth_dot_json.lock() {
            *auth_lock = Some(updated.clone());
        }
        let access = updated
            .active_account()
            .and_then(|account| account.tokens.as_ref())
            .map(|t| t.access_token.clone())
            .ok_or_else(|| std::io::Error::other("Token data is not available after refresh."))?;
        Ok(access)
    }

    /// Loads the available auth information from the auth.json.
    pub fn from_codex_home(codex_home: &Path) -> std::io::Result<Option<CodexAuth>> {
        load_auth(codex_home)
    }

    pub async fn get_token_data(&self) -> Result<TokenData, std::io::Error> {
        let auth_dot_json = self
            .get_current_auth_json()
            .ok_or(std::io::Error::other("Token data is not available."))?;

        let account_index = self.resolve_account_index_with_fallback(&auth_dot_json);

        let account = auth_dot_json
            .accounts
            .get(account_index)
            .cloned()
            .ok_or(std::io::Error::other("Token data is not available."))?;

        let mut tokens = account
            .tokens
            .clone()
            .ok_or(std::io::Error::other("Token data is not available."))?;
        let last_refresh = account
            .last_refresh
            .ok_or(std::io::Error::other("Token data is not available."))?;

        if last_refresh < Utc::now() - chrono::Duration::days(28) {
            let refresh_response = tokio::time::timeout(
                Duration::from_secs(60),
                try_refresh_token(tokens.refresh_token.clone(), &self.client),
            )
            .await
            .map_err(|_| std::io::Error::other("timed out while refreshing OpenAI API key"))?
            .map_err(std::io::Error::other)?;

            let updated_auth_dot_json = update_tokens(
                &self.auth_file,
                Some(account_index),
                refresh_response.id_token,
                refresh_response.access_token,
                refresh_response.refresh_token,
            )
            .await?;

            tokens = updated_auth_dot_json
                .accounts
                .get(account_index)
                .and_then(|acct| acct.tokens.clone())
                .ok_or(std::io::Error::other(
                    "Token data is not available after refresh.",
                ))?;

            #[expect(clippy::unwrap_used)]
            let mut auth_lock = self.auth_dot_json.lock().unwrap();
            *auth_lock = Some(updated_auth_dot_json);
        }

        Ok(tokens)
    }

    pub async fn get_token(&self) -> Result<String, std::io::Error> {
        match self.mode {
            AuthMode::ApiKey => Ok(self.api_key.clone().unwrap_or_default()),
            AuthMode::ChatGPT => {
                let id_token = self.get_token_data().await?.access_token;
                Ok(id_token)
            }
        }
    }

    pub fn get_account_id(&self) -> Option<String> {
        self.get_current_token_data().and_then(|t| t.account_id)
    }

    pub(crate) fn get_plan_type(&self) -> Option<PlanType> {
        self.get_current_token_data()
            .and_then(|t| t.id_token.chatgpt_plan_type)
    }

    pub fn account_pool_summary(&self) -> AccountPoolSummary {
        let state = account_pool_state();
        AccountPoolSummary {
            total_accounts: state.total_accounts,
            active_index: state.active_index,
            rotation_enabled: state.rotation_enabled,
            available_accounts: state.available_accounts,
            cooldown_accounts: state.cooldown_accounts,
            inactive_accounts: state.inactive_accounts,
            next_available_at: state.next_available_at,
            rate_limited_accounts: state.rate_limited_accounts,
            last_rate_limit_at: state.last_rate_limit_at,
            last_rotation_at: state.last_rotation_at,
        }
    }

    fn get_current_auth_json(&self) -> Option<AuthDotJson> {
        #[expect(clippy::unwrap_used)]
        self.auth_dot_json.lock().unwrap().clone()
    }

    fn get_current_token_data(&self) -> Option<TokenData> {
        let auth = self.get_current_auth_json()?;
        if auth.accounts.is_empty() {
            return auth.tokens;
        }
        let idx = self.resolve_account_index_with_fallback(&auth);
        auth.accounts.get(idx).and_then(|acct| acct.tokens.clone())
    }

    fn resolve_account_index(&self) -> Option<usize> {
        self.active_account_index
            .or_else(|| self.get_current_auth_json().map(|auth| auth.active_index()))
    }

    fn resolve_account_index_with_fallback(&self, auth: &AuthDotJson) -> usize {
        self.active_account_index
            .or(auth.current_account_index)
            .unwrap_or_else(|| auth.active_index())
    }

    /// Consider this private to integration tests.
    pub fn create_dummy_chatgpt_auth_for_testing() -> Self {
        let account = AuthAccount {
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: Default::default(),
                access_token: "Access Token".to_string(),
                refresh_token: "test".to_string(),
                account_id: Some("account_id".to_string()),
            }),
            last_refresh: Some(Utc::now()),
            rate_limit_reset: None,
        };

        let mut auth_dot_json = AuthDotJson {
            openai_api_key: None,
            tokens: None,
            last_refresh: None,
            accounts: vec![account],
            current_account_index: Some(0),
            rotation_enabled: Some(false),
        };
        auth_dot_json.normalize();

        let auth_dot_json = Arc::new(Mutex::new(Some(auth_dot_json)));
        Self {
            api_key: None,
            mode: AuthMode::ChatGPT,
            active_account_index: Some(0),
            auth_file: PathBuf::new(),
            auth_dot_json,
            client: crate::default_client::create_client(),
        }
    }

    fn from_api_key_with_client(api_key: &str, client: reqwest::Client) -> Self {
        Self {
            api_key: Some(api_key.to_owned()),
            mode: AuthMode::ApiKey,
            active_account_index: None,
            auth_file: PathBuf::new(),
            auth_dot_json: Arc::new(Mutex::new(None)),
            client,
        }
    }

    pub fn from_api_key(api_key: &str) -> Self {
        Self::from_api_key_with_client(api_key, crate::default_client::create_client())
    }
}

pub const OPENAI_API_KEY_ENV_VAR: &str = "OPENAI_API_KEY";

pub fn read_openai_api_key_from_env() -> Option<String> {
    env::var(OPENAI_API_KEY_ENV_VAR)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn auth_pool_path(codex_home: &Path) -> PathBuf {
    codex_home.join("auth_pool.json")
}

fn legacy_auth_path(codex_home: &Path) -> PathBuf {
    codex_home.join("auth.json")
}

/// Returns the canonical path Codex uses to persist authentication state.
/// The new default is `$CODEX_HOME/auth_pool.json` (multi-account aware).
pub fn get_auth_file(codex_home: &Path) -> PathBuf {
    auth_pool_path(codex_home)
}

/// Returns the best available authentication file for reading:
/// prefers `auth_pool.json` and falls back to legacy `auth.json` if needed.
pub fn find_auth_file(codex_home: &Path) -> Option<PathBuf> {
    let pool = auth_pool_path(codex_home);
    if pool.exists() {
        Some(pool)
    } else {
        let legacy = legacy_auth_path(codex_home);
        if legacy.exists() { Some(legacy) } else { None }
    }
}

/// Delete the auth.json file inside `codex_home` if it exists. Returns `Ok(true)`
/// if a file was removed, `Ok(false)` if no auth file was present.
pub fn logout(codex_home: &Path) -> std::io::Result<bool> {
    let mut removed = false;
    for path in [auth_pool_path(codex_home), legacy_auth_path(codex_home)] {
        match std::fs::remove_file(&path) {
            Ok(_) => removed = true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(removed)
}

/// Writes an `auth.json` that contains only the API key.
pub fn login_with_api_key(codex_home: &Path, api_key: &str) -> std::io::Result<()> {
    let auth_dot_json = AuthDotJson {
        openai_api_key: Some(api_key.to_string()),
        tokens: None,
        last_refresh: None,
        accounts: Vec::new(),
        current_account_index: None,
        rotation_enabled: None,
    };
    write_auth_json(&get_auth_file(codex_home), &auth_dot_json)
}

fn load_auth(codex_home: &Path) -> std::io::Result<Option<CodexAuth>> {
    let client = crate::default_client::create_client();
    let canonical_file = get_auth_file(codex_home);

    let source_file = match find_auth_file(codex_home) {
        Some(path) => path,
        None => return Ok(None),
    };

    let mut auth_dot_json = match try_read_auth_json(&source_file) {
        Ok(auth) => auth,
        Err(e) => {
            return Err(e);
        }
    };

    if source_file != canonical_file {
        // Migrate legacy auth.json into the canonical auth_pool.json.
        write_auth_json(&canonical_file, &auth_dot_json)?;
        auth_dot_json = try_read_auth_json(&canonical_file)?;
    }

    if let Some(account) = auth_dot_json.active_account() {
        if let Some(api_key) = &account.openai_api_key {
            return Ok(Some(CodexAuth::from_api_key_with_client(api_key, client)));
        }
    }

    if let Some(api_key) = &auth_dot_json.openai_api_key {
        return Ok(Some(CodexAuth::from_api_key_with_client(api_key, client)));
    }

    if auth_dot_json
        .active_account()
        .and_then(|account| account.tokens.as_ref())
        .is_none()
    {
        return Ok(None);
    }

    auth_dot_json.normalize();
    refresh_account_pool_snapshot(&auth_dot_json);
    let active_account_index = auth_dot_json.current_account_index;

    Ok(Some(CodexAuth {
        api_key: None,
        mode: AuthMode::ChatGPT,
        auth_file: canonical_file,
        active_account_index,
        auth_dot_json: Arc::new(Mutex::new(Some(auth_dot_json))),
        client,
    }))
}

/// Attempt to read and refresh the `auth.json` file in the given `CODEX_HOME` directory.
/// Returns the full AuthDotJson structure after refreshing if necessary.
pub fn try_read_auth_json(auth_file: &Path) -> std::io::Result<AuthDotJson> {
    let mut file = File::open(auth_file)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let mut auth_dot_json: AuthDotJson = serde_json::from_str(&contents)?;
    auth_dot_json.normalize();

    Ok(auth_dot_json)
}

pub fn write_auth_json(auth_file: &Path, auth_dot_json: &AuthDotJson) -> std::io::Result<()> {
    let mut normalized = auth_dot_json.clone();
    normalized.normalize();
    refresh_account_pool_snapshot(&normalized);
    let json_data = serde_json::to_string_pretty(&normalized)?;
    let mut options = OpenOptions::new();
    options.truncate(true).write(true).create(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(auth_file)?;
    file.write_all(json_data.as_bytes())?;
    file.flush()?;
    Ok(())
}

async fn update_tokens(
    auth_file: &Path,
    account_index: Option<usize>,
    id_token: String,
    access_token: Option<String>,
    refresh_token: Option<String>,
) -> std::io::Result<AuthDotJson> {
    let mut auth_dot_json = try_read_auth_json(auth_file)?;

    let idx = account_index.unwrap_or_else(|| auth_dot_json.active_index());
    let account = auth_dot_json
        .accounts
        .get_mut(idx)
        .ok_or_else(|| std::io::Error::other("Account index out of bounds"))?;

    let tokens = account.tokens.get_or_insert_with(TokenData::default);
    tokens.id_token = parse_id_token(&id_token).map_err(std::io::Error::other)?;
    if let Some(access_token) = access_token {
        tokens.access_token = access_token;
    }
    if let Some(refresh_token) = refresh_token {
        tokens.refresh_token = refresh_token;
    }
    account.last_refresh = Some(Utc::now());
    account.clear_rate_limit();
    auth_dot_json.set_active_index(idx);

    write_auth_json(auth_file, &auth_dot_json)?;
    let mut updated = auth_dot_json.clone();
    updated.normalize();
    Ok(updated)
}

async fn try_refresh_token(
    refresh_token: String,
    client: &reqwest::Client,
) -> std::io::Result<RefreshResponse> {
    let refresh_request = RefreshRequest {
        client_id: CLIENT_ID,
        grant_type: "refresh_token",
        refresh_token,
        scope: "openid profile email",
    };

    // Use shared client factory to include standard headers
    let response = client
        .post("https://auth.openai.com/oauth/token")
        .header("Content-Type", "application/json")
        .json(&refresh_request)
        .send()
        .await
        .map_err(std::io::Error::other)?;

    if response.status().is_success() {
        let refresh_response = response
            .json::<RefreshResponse>()
            .await
            .map_err(std::io::Error::other)?;
        Ok(refresh_response)
    } else {
        Err(std::io::Error::other(format!(
            "Failed to refresh token: {}",
            response.status()
        )))
    }
}

#[derive(Serialize)]
struct RefreshRequest {
    client_id: &'static str,
    grant_type: &'static str,
    refresh_token: String,
    scope: &'static str,
}

#[derive(Deserialize, Clone)]
struct RefreshResponse {
    id_token: String,
    access_token: Option<String>,
    refresh_token: Option<String>,
}

/// Represents the serialized auth.json document. Legacy (single-account) files
/// populate only the top-level fields. When account rotation is enabled the
/// `accounts` list stores individual entries together with rotation metadata.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct AuthDotJson {
    #[serde(
        rename = "OPENAI_API_KEY",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub openai_api_key: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenData>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<DateTime<Utc>>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<AuthAccount>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_account_index: Option<usize>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_enabled: Option<bool>,
}

/// Single account entry inside the auth pool. The legacy single-account format
/// is mapped to a vector that contains exactly one element of this struct.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct AuthAccount {
    #[serde(
        rename = "OPENAI_API_KEY",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub openai_api_key: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenData>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<DateTime<Utc>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_reset: Option<DateTime<Utc>>,
}

impl AuthDotJson {
    fn normalize(&mut self) {
        if self.accounts.is_empty() {
            self.accounts.push(AuthAccount {
                openai_api_key: self.openai_api_key.clone(),
                tokens: self.tokens.clone(),
                last_refresh: self.last_refresh,
                rate_limit_reset: None,
            });
        }

        if self.current_account_index.is_none() {
            self.current_account_index = Some(0);
        }

        let len = self.accounts.len();
        if len == 0 {
            self.current_account_index = Some(0);
        } else if let Some(idx) = self.current_account_index {
            if idx >= len {
                self.current_account_index = Some(0);
            }
        }

        if self.rotation_enabled.is_none() {
            let enabled = self.accounts.len() > 1;
            self.rotation_enabled = Some(enabled);
        }

        let idx = self.active_index();
        if let Some(account) = self.accounts.get(idx) {
            self.openai_api_key = account.openai_api_key.clone();
            self.tokens = account.tokens.clone();
            self.last_refresh = account.last_refresh;
        }
    }

    pub fn active_index(&self) -> usize {
        let len = self.accounts.len();
        if len == 0 {
            return 0;
        }
        self.current_account_index.unwrap_or(0).min(len - 1)
    }

    fn active_account(&self) -> Option<&AuthAccount> {
        if self.accounts.is_empty() {
            return None;
        }
        let idx = self.active_index();
        self.accounts.get(idx)
    }

    pub fn rotation_enabled(&self) -> bool {
        self.rotation_enabled.unwrap_or(false)
    }

    fn set_active_index(&mut self, idx: usize) {
        if self.accounts.is_empty() {
            self.accounts.push(AuthAccount {
                openai_api_key: self.openai_api_key.clone(),
                tokens: self.tokens.clone(),
                last_refresh: self.last_refresh,
                rate_limit_reset: None,
            });
        }
        let len = self.accounts.len();
        let bounded = if len == 0 { 0 } else { idx.min(len - 1) };
        self.current_account_index = Some(bounded);
        if let Some(account) = self.accounts.get(bounded) {
            self.openai_api_key = account.openai_api_key.clone();
            self.tokens = account.tokens.clone();
            self.last_refresh = account.last_refresh;
        }
    }

    fn mark_rate_limited(&mut self, idx: usize, reset_at: Option<DateTime<Utc>>) {
        if let Some(account) = self.accounts.get_mut(idx) {
            account.rate_limit_reset = reset_at;
        }
    }
}

impl AuthAccount {
    fn is_available(&self, now: DateTime<Utc>) -> bool {
        match self.rate_limit_reset {
            Some(reset) => reset <= now,
            None => true,
        }
    }

    fn clear_rate_limit(&mut self) {
        self.rate_limit_reset = None;
    }
}

// Shared constant for token refresh (client id used for oauth token refresh flow)
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

use std::sync::RwLock;

/// Internal cached auth state.
#[derive(Clone, Debug)]
struct CachedAuth {
    auth: Option<CodexAuth>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_data::IdTokenInfo;
    use crate::token_data::KnownPlan;
    use crate::token_data::PlanType;
    use base64::Engine;
    use pretty_assertions::assert_eq;
    use serde::Serialize;
    use serde_json::json;
    use tempfile::tempdir;

    const LAST_REFRESH: &str = "2025-08-06T20:41:36.232376Z";
    const SAMPLE_JWT: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.e30.c2ln";

    #[tokio::test]
    async fn roundtrip_auth_dot_json() {
        let codex_home = tempdir().unwrap();
        let _ = write_auth_file(
            AuthFileParams {
                openai_api_key: None,
                chatgpt_plan_type: "pro".to_string(),
            },
            codex_home.path(),
        )
        .expect("failed to write auth file");

        let file = get_auth_file(codex_home.path());
        let auth_dot_json = try_read_auth_json(&file).unwrap();
        write_auth_json(&file, &auth_dot_json).unwrap();

        let same_auth_dot_json = try_read_auth_json(&file).unwrap();
        assert_eq!(auth_dot_json, same_auth_dot_json);
    }

    #[test]
    fn login_with_api_key_overwrites_existing_auth_json() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        let stale_auth = json!({
            "OPENAI_API_KEY": "sk-old",
            "tokens": {
                "id_token": "stale.header.payload",
                "access_token": "stale-access",
                "refresh_token": "stale-refresh",
                "account_id": "stale-acc"
            }
        });
        std::fs::write(
            &auth_path,
            serde_json::to_string_pretty(&stale_auth).unwrap(),
        )
        .unwrap();

        super::login_with_api_key(dir.path(), "sk-new").expect("login_with_api_key should succeed");

        let canonical = get_auth_file(dir.path());
        assert!(canonical.exists(), "canonical auth file should be created");
        let auth = super::try_read_auth_json(&canonical).expect("auth file should parse");
        assert_eq!(auth.openai_api_key.as_deref(), Some("sk-new"));
        assert!(auth.tokens.is_none(), "tokens should be cleared");
    }

    #[tokio::test]
    async fn pro_account_with_no_api_key_uses_chatgpt_auth() {
        let codex_home = tempdir().unwrap();
        let fake_jwt = write_auth_file(
            AuthFileParams {
                openai_api_key: None,
                chatgpt_plan_type: "pro".to_string(),
            },
            codex_home.path(),
        )
        .expect("failed to write auth file");

        let CodexAuth {
            api_key,
            mode,
            auth_dot_json,
            auth_file: _,
            ..
        } = super::load_auth(codex_home.path()).unwrap().unwrap();
        assert_eq!(None, api_key);
        assert_eq!(AuthMode::ChatGPT, mode);

        let guard = auth_dot_json.lock().unwrap();
        let auth_dot_json = guard.as_ref().expect("AuthDotJson should exist");
        assert_eq!(
            &AuthDotJson {
                openai_api_key: None,
                tokens: Some(TokenData {
                    id_token: IdTokenInfo {
                        email: Some("user@example.com".to_string()),
                        chatgpt_plan_type: Some(PlanType::Known(KnownPlan::Pro)),
                        raw_jwt: fake_jwt.clone(),
                    },
                    access_token: "test-access-token".to_string(),
                    refresh_token: "test-refresh-token".to_string(),
                    account_id: None,
                }),
                last_refresh: Some(
                    DateTime::parse_from_rfc3339(LAST_REFRESH)
                        .unwrap()
                        .with_timezone(&Utc)
                ),
                accounts: vec![AuthAccount {
                    openai_api_key: None,
                    tokens: Some(TokenData {
                        id_token: IdTokenInfo {
                            email: Some("user@example.com".to_string()),
                            chatgpt_plan_type: Some(PlanType::Known(KnownPlan::Pro)),
                            raw_jwt: fake_jwt,
                        },
                        access_token: "test-access-token".to_string(),
                        refresh_token: "test-refresh-token".to_string(),
                        account_id: None,
                    }),
                    last_refresh: Some(
                        DateTime::parse_from_rfc3339(LAST_REFRESH)
                            .unwrap()
                            .with_timezone(&Utc)
                    ),
                    rate_limit_reset: None,
                }],
                current_account_index: Some(0),
                rotation_enabled: Some(false),
            },
            auth_dot_json
        )
    }

    #[tokio::test]
    async fn loads_api_key_from_auth_json() {
        let dir = tempdir().unwrap();
        let auth_file = dir.path().join("auth.json");
        std::fs::write(
            auth_file,
            r#"{"OPENAI_API_KEY":"sk-test-key","tokens":null,"last_refresh":null}"#,
        )
        .unwrap();

        let auth = super::load_auth(dir.path()).unwrap().unwrap();
        assert_eq!(auth.mode, AuthMode::ApiKey);
        assert_eq!(auth.api_key, Some("sk-test-key".to_string()));

        assert!(auth.get_token_data().await.is_err());
    }

    #[test]
    fn logout_removes_auth_file() -> Result<(), std::io::Error> {
        let dir = tempdir()?;
        let auth_dot_json = AuthDotJson {
            openai_api_key: Some("sk-test-key".to_string()),
            tokens: None,
            last_refresh: None,
            accounts: Vec::new(),
            current_account_index: None,
            rotation_enabled: None,
        };
        let canonical = get_auth_file(dir.path());
        write_auth_json(&canonical, &auth_dot_json)?;
        assert!(canonical.exists());
        // Simulate legacy file lingering on disk.
        let legacy = legacy_auth_path(dir.path());
        std::fs::write(&legacy, "legacy").unwrap();
        let removed = logout(dir.path())?;
        assert!(removed);
        assert!(!canonical.exists());
        assert!(!legacy.exists());
        Ok(())
    }

    #[test]
    fn rotates_to_next_available_account() {
        let dir = tempdir().unwrap();
        let auth_file = get_auth_file(dir.path());

        let account0 = AuthAccount {
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: IdTokenInfo {
                    email: None,
                    chatgpt_plan_type: None,
                    raw_jwt: SAMPLE_JWT.to_string(),
                },
                access_token: "access0".to_string(),
                refresh_token: "refresh0".to_string(),
                account_id: Some("account0".to_string()),
            }),
            last_refresh: Some(Utc::now()),
            rate_limit_reset: None,
        };

        let account1 = AuthAccount {
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: IdTokenInfo {
                    email: None,
                    chatgpt_plan_type: None,
                    raw_jwt: SAMPLE_JWT.to_string(),
                },
                access_token: "access1".to_string(),
                refresh_token: "refresh1".to_string(),
                account_id: Some("account1".to_string()),
            }),
            last_refresh: Some(Utc::now()),
            rate_limit_reset: None,
        };

        let auth = AuthDotJson {
            openai_api_key: None,
            tokens: None,
            last_refresh: None,
            accounts: vec![account0, account1],
            current_account_index: Some(0),
            rotation_enabled: Some(true),
        };
        write_auth_json(&auth_file, &auth).unwrap();

        let manager = AuthManager::shared(dir.path().to_path_buf());
        let initial = manager.auth().unwrap();
        assert_eq!(initial.active_account_index, Some(0));

        manager.mark_current_rate_limited(Some(120)).unwrap();
        assert!(manager.switch_to_next_account().unwrap());

        let rotated = manager.auth().unwrap();
        assert_eq!(rotated.active_account_index, Some(1));

        let persisted = try_read_auth_json(&auth_file).unwrap();
        assert_eq!(persisted.current_account_index, Some(1));
        assert!(persisted.accounts[0].rate_limit_reset.is_some());
        assert!(persisted.accounts[1].rate_limit_reset.is_none());
        assert_eq!(
            persisted.accounts[1].tokens.as_ref().unwrap().access_token,
            "access1"
        );
    }

    #[test]
    fn rotation_respects_disabled_flag() {
        let dir = tempdir().unwrap();
        let auth_file = get_auth_file(dir.path());

        let account = AuthAccount {
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: IdTokenInfo {
                    email: None,
                    chatgpt_plan_type: None,
                    raw_jwt: SAMPLE_JWT.to_string(),
                },
                access_token: "access".to_string(),
                refresh_token: "refresh".to_string(),
                account_id: Some("account".to_string()),
            }),
            last_refresh: Some(Utc::now()),
            rate_limit_reset: None,
        };

        let account_b = AuthAccount {
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: IdTokenInfo {
                    email: None,
                    chatgpt_plan_type: None,
                    raw_jwt: SAMPLE_JWT.to_string(),
                },
                access_token: "access-b".to_string(),
                refresh_token: "refresh-b".to_string(),
                account_id: Some("account-b".to_string()),
            }),
            last_refresh: Some(Utc::now()),
            rate_limit_reset: None,
        };

        let auth = AuthDotJson {
            openai_api_key: None,
            tokens: None,
            last_refresh: None,
            accounts: vec![account, account_b],
            current_account_index: Some(0),
            rotation_enabled: Some(false),
        };
        write_auth_json(&auth_file, &auth).unwrap();

        let manager = AuthManager::shared(dir.path().to_path_buf());
        manager.mark_current_rate_limited(Some(30)).unwrap();
        assert!(!manager.switch_to_next_account().unwrap());
    }

    struct AuthFileParams {
        openai_api_key: Option<String>,
        chatgpt_plan_type: String,
    }

    fn write_auth_file(params: AuthFileParams, codex_home: &Path) -> std::io::Result<String> {
        let auth_file = get_auth_file(codex_home);
        // Create a minimal valid JWT for the id_token field.
        #[derive(Serialize)]
        struct Header {
            alg: &'static str,
            typ: &'static str,
        }
        let header = Header {
            alg: "none",
            typ: "JWT",
        };
        let payload = serde_json::json!({
            "email": "user@example.com",
            "email_verified": true,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "bc3618e3-489d-4d49-9362-1561dc53ba53",
                "chatgpt_plan_type": params.chatgpt_plan_type,
                "chatgpt_user_id": "user-12345",
                "user_id": "user-12345",
            }
        });
        let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
        let header_b64 = b64(&serde_json::to_vec(&header)?);
        let payload_b64 = b64(&serde_json::to_vec(&payload)?);
        let signature_b64 = b64(b"sig");
        let fake_jwt = format!("{header_b64}.{payload_b64}.{signature_b64}");

        let auth_json_data = json!({
            "OPENAI_API_KEY": params.openai_api_key,
            "tokens": {
                "id_token": fake_jwt,
                "access_token": "test-access-token",
                "refresh_token": "test-refresh-token"
            },
            "last_refresh": LAST_REFRESH,
        });
        let auth_json = serde_json::to_string_pretty(&auth_json_data)?;
        std::fs::write(auth_file, auth_json)?;
        Ok(fake_jwt)
    }
}

/// Central manager providing a single source of truth for auth.json derived
/// authentication data. It loads once (or on preference change) and then
/// hands out cloned `CodexAuth` values so the rest of the program has a
/// consistent snapshot.
///
/// External modifications to `auth.json` will NOT be observed until
/// `reload()` is called explicitly. This matches the design goal of avoiding
/// different parts of the program seeing inconsistent auth data mid‑run.
#[derive(Debug)]
pub struct AuthManager {
    codex_home: PathBuf,
    inner: RwLock<CachedAuth>,
}

impl AuthManager {
    /// Create a new manager loading the initial auth using the provided
    /// preferred auth method. Errors loading auth are swallowed; `auth()` will
    /// simply return `None` in that case so callers can treat it as an
    /// unauthenticated state.
    pub fn new(codex_home: PathBuf) -> Self {
        let auth = CodexAuth::from_codex_home(&codex_home).ok().flatten();
        Self {
            codex_home,
            inner: RwLock::new(CachedAuth { auth }),
        }
    }

    /// Create an AuthManager with a specific CodexAuth, for testing only.
    pub fn from_auth_for_testing(auth: CodexAuth) -> Arc<Self> {
        let cached = CachedAuth { auth: Some(auth) };
        Arc::new(Self {
            codex_home: PathBuf::new(),
            inner: RwLock::new(cached),
        })
    }

    /// Current cached auth (clone). May be `None` if not logged in or load failed.
    pub fn auth(&self) -> Option<CodexAuth> {
        self.inner.read().ok().and_then(|c| c.auth.clone())
    }

    /// Force a reload of the auth information from auth.json. Returns
    /// whether the auth value changed.
    pub fn reload(&self) -> bool {
        let new_auth = CodexAuth::from_codex_home(&self.codex_home).ok().flatten();
        if let Ok(mut guard) = self.inner.write() {
            let changed = !AuthManager::auths_equal(&guard.auth, &new_auth);
            guard.auth = new_auth;
            changed
        } else {
            false
        }
    }

    fn auths_equal(a: &Option<CodexAuth>, b: &Option<CodexAuth>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// Convenience constructor returning an `Arc` wrapper.
    pub fn shared(codex_home: PathBuf) -> Arc<Self> {
        Arc::new(Self::new(codex_home))
    }

    /// Attempt to refresh the current auth token (if any). On success, reload
    /// the auth state from disk so other components observe refreshed token.
    pub async fn refresh_token(&self) -> std::io::Result<Option<String>> {
        let auth = match self.auth() {
            Some(a) => a,
            None => return Ok(None),
        };
        match auth.refresh_token().await {
            Ok(token) => {
                // Reload to pick up persisted changes.
                self.reload();
                Ok(Some(token))
            }
            Err(e) => Err(e),
        }
    }

    /// Log out by deleting the on‑disk auth.json (if present). Returns Ok(true)
    /// if a file was removed, Ok(false) if no auth file existed. On success,
    /// reloads the in‑memory auth cache so callers immediately observe the
    /// unauthenticated state.
    pub fn logout(&self) -> std::io::Result<bool> {
        let removed = super::auth::logout(&self.codex_home)?;
        // Always reload to clear any cached auth (even if file absent).
        self.reload();
        Ok(removed)
    }

    pub fn mark_current_rate_limited(&self, resets_in_seconds: Option<u64>) -> std::io::Result<()> {
        let auth_file = get_auth_file(&self.codex_home);
        let mut auth_dot_json = try_read_auth_json(&auth_file)?;
        if auth_dot_json.accounts.is_empty() {
            return Ok(());
        }

        let secs = resets_in_seconds.unwrap_or(60);
        let capped_secs = std::cmp::min(secs, i64::MAX as u64) as i64;
        let reset_at = Utc::now() + chrono::Duration::seconds(capped_secs);

        let idx = auth_dot_json.active_index();
        auth_dot_json.mark_rate_limited(idx, Some(reset_at));
        write_auth_json(&auth_file, &auth_dot_json)?;
        self.reload();
        update_account_pool_state(|state| {
            state.last_rate_limit_at = Some(Utc::now());
        });
        Ok(())
    }

    pub fn switch_to_next_account(&self) -> std::io::Result<bool> {
        use tracing::debug;
        use tracing::warn;

        let auth_file = get_auth_file(&self.codex_home);
        let mut auth_dot_json = try_read_auth_json(&auth_file)?;

        if !auth_dot_json.rotation_enabled() {
            debug!("Account rotation disabled");
            return Ok(false);
        }

        let total = auth_dot_json.accounts.len();
        if total <= 1 {
            debug!("Only {} account(s) available, cannot rotate", total);
            return Ok(false);
        }

        let current = auth_dot_json.active_index();
        let now = Utc::now();
        debug!(
            "Current account index: {}, total accounts: {}",
            current, total
        );

        for offset in 1..total {
            let candidate = (current + offset) % total;
            let account = match auth_dot_json.accounts.get(candidate) {
                Some(acc) => acc,
                None => {
                    warn!("Account at index {} not found", candidate);
                    continue;
                }
            };

            // Check if account has valid tokens
            let has_valid_tokens = account
                .tokens
                .as_ref()
                .map(|t| !t.access_token.is_empty() && !t.refresh_token.is_empty())
                .unwrap_or(false);

            if !has_valid_tokens {
                debug!("Account {} has no valid tokens, skipping", candidate);
                continue;
            }

            if !account.is_available(now) {
                if let Some(reset) = account.rate_limit_reset {
                    debug!("Account {} is rate limited until {}", candidate, reset);
                }
                continue;
            }

            debug!(
                "Switching from account {} to account {}",
                current, candidate
            );
            auth_dot_json.set_active_index(candidate);
            if let Some(account) = auth_dot_json.accounts.get_mut(candidate) {
                account.clear_rate_limit();
            }
            write_auth_json(&auth_file, &auth_dot_json)?;
            self.reload();
            update_account_pool_state(|state| {
                state.last_rotation_at = Some(Utc::now());
            });
            debug!("Successfully switched to account {}", candidate);
            return Ok(true);
        }

        warn!(
            "No available accounts found for rotation. All {} accounts are either rate limited or have invalid tokens",
            total
        );
        Ok(false)
    }
}
#[derive(Debug, Clone)]
pub struct AccountPoolState {
    pub total_accounts: usize,
    pub active_index: Option<usize>,
    pub rotation_enabled: bool,
    pub available_accounts: usize,
    pub cooldown_accounts: usize,
    pub inactive_accounts: usize,
    pub next_available_at: Option<DateTime<Utc>>,
    pub rate_limited_accounts: usize,
    pub last_rate_limit_at: Option<DateTime<Utc>>,
    pub last_rotation_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct AccountPoolSummary {
    pub total_accounts: usize,
    pub active_index: Option<usize>,
    pub rotation_enabled: bool,
    pub available_accounts: usize,
    pub cooldown_accounts: usize,
    pub inactive_accounts: usize,
    pub next_available_at: Option<DateTime<Utc>>,
    pub rate_limited_accounts: usize,
    pub last_rate_limit_at: Option<DateTime<Utc>>,
    pub last_rotation_at: Option<DateTime<Utc>>,
}

impl Default for AccountPoolState {
    fn default() -> Self {
        Self {
            total_accounts: 0,
            active_index: None,
            rotation_enabled: false,
            available_accounts: 0,
            cooldown_accounts: 0,
            inactive_accounts: 0,
            next_available_at: None,
            rate_limited_accounts: 0,
            last_rate_limit_at: None,
            last_rotation_at: None,
        }
    }
}

static ACCOUNT_POOL_STATE: Lazy<RwLock<AccountPoolState>> =
    Lazy::new(|| RwLock::new(AccountPoolState::default()));

pub fn account_pool_state() -> AccountPoolState {
    ACCOUNT_POOL_STATE
        .read()
        .map(|state| state.clone())
        .unwrap_or_default()
}

fn update_account_pool_state<F>(update: F)
where
    F: FnOnce(&mut AccountPoolState),
{
    if let Ok(mut guard) = ACCOUNT_POOL_STATE.write() {
        update(&mut guard);
    }
}

pub fn set_account_pool_state_for_testing(state: AccountPoolState) {
    if let Ok(mut guard) = ACCOUNT_POOL_STATE.write() {
        *guard = state;
    }
}

fn refresh_account_pool_snapshot(auth: &AuthDotJson) {
    let total_accounts = auth.accounts.len();
    let active_index = if total_accounts == 0 {
        None
    } else {
        Some(auth.active_index())
    };
    let rotation_enabled = auth.rotation_enabled();
    let now = Utc::now();
    let mut available_accounts = 0usize;
    let mut cooldown_accounts = 0usize;
    let mut inactive_accounts = 0usize;
    let mut next_available_at: Option<DateTime<Utc>> = None;

    for account in &auth.accounts {
        let has_tokens = account
            .tokens
            .as_ref()
            .map(|tokens| !tokens.access_token.is_empty() && !tokens.refresh_token.is_empty())
            .unwrap_or(false);

        if !has_tokens {
            inactive_accounts += 1;
            continue;
        }

        if let Some(reset_at) = account.rate_limit_reset {
            if reset_at > now {
                cooldown_accounts += 1;
                next_available_at = match next_available_at {
                    Some(current) if current <= reset_at => Some(current),
                    _ => Some(reset_at),
                };
                continue;
            }
        }

        available_accounts += 1;
    }

    let rate_limited_accounts = cooldown_accounts;

    update_account_pool_state(|state| {
        state.total_accounts = total_accounts;
        state.active_index = active_index;
        state.rotation_enabled = rotation_enabled;
        state.available_accounts = available_accounts;
        state.cooldown_accounts = cooldown_accounts;
        state.inactive_accounts = inactive_accounts;
        state.next_available_at = next_available_at;
        state.rate_limited_accounts = rate_limited_accounts;
    });
}
