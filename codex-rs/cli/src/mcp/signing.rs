//! Secure Command Signing helpers for the MCP wizard apply path.
//!
//! Trace: REQ-SEC-01 (#9, #74) — enforce signed command envelopes before
//! persisting changes.

use std::convert::TryInto;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use ed25519_dalek::Signature;
use ed25519_dalek::Verifier;
use ed25519_dalek::VerifyingKey;

use crate::mcp::cli::WizardArgs;
use crate::mcp::wizard::WizardOutcome;

const ENVELOPE_CONTEXT: &str = "stellar.mcp.wizard";
const MIN_NONCE_LENGTH: usize = 8;

/// Parsed Secure Command Signing envelope.
#[derive(Debug, Clone)]
pub struct CommandSignatureEnvelope {
    verifying_key: VerifyingKey,
    signature: Signature,
    signed_at: DateTime<Utc>,
    nonce: String,
}

impl CommandSignatureEnvelope {
    /// Build an envelope from CLI arguments. Returns `Ok(None)` when no signing
    /// fields are provided, and errors on partial input.
    pub fn from_args(args: &WizardArgs) -> Result<Option<Self>> {
        let Some(key) = args.signing_key.as_ref() else {
            ensure_no_partial_inputs(args)?;
            return Ok(None);
        };
        let signature = args.signature.as_ref().ok_or_else(|| {
            anyhow!("--signature is required when --signing-key is provided (REQ-SEC-01)")
        })?;
        let signed_at = args.signed_at.as_ref().ok_or_else(|| {
            anyhow!("--signed-at is required when --signing-key is provided (REQ-SEC-01)")
        })?;
        let nonce = args.nonce.as_ref().ok_or_else(|| {
            anyhow!("--nonce is required when --signing-key is provided (REQ-SEC-01)")
        })?;

        if nonce.trim().len() < MIN_NONCE_LENGTH {
            bail!(
                "nonce must be at least {MIN_NONCE_LENGTH} characters for replay resistance (REQ-SEC-01)"
            );
        }

        let verifying_key = decode_verifying_key(key)?;
        let signature = decode_signature(signature)?;
        let signed_at = parse_timestamp(signed_at)?;

        Ok(Some(Self {
            verifying_key,
            signature,
            signed_at,
            nonce: nonce.clone(),
        }))
    }

    /// Verify the envelope against the wizard outcome.
    pub fn verify(&self, outcome: &WizardOutcome) -> Result<()> {
        enforce_timestamp_bounds(self.signed_at)?;
        let message = build_envelope_message(outcome, &self.nonce, self.signed_at)?;
        self.verifying_key
            .verify(&message, &self.signature)
            .with_context(
                || "Secure Command Signing failed: signature mismatch (REQ-SEC-01, #74)",
            )?;
        Ok(())
    }

    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    pub fn signed_at(&self) -> DateTime<Utc> {
        self.signed_at
    }

    pub fn verifying_key_b64(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.verifying_key.to_bytes())
    }
}

fn ensure_no_partial_inputs(args: &WizardArgs) -> Result<()> {
    if args.signature.is_some() || args.signed_at.is_some() || args.nonce.is_some() {
        bail!(
            "Secure Command Signing requires --signing-key, --signature, --signed-at, and --nonce together (REQ-SEC-01)"
        );
    }
    Ok(())
}

fn decode_verifying_key(value: &str) -> Result<VerifyingKey> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value.trim())
        .with_context(|| "failed to decode signing key: expected base64url (REQ-SEC-01)")?;
    let array: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("signing key must decode to 32 bytes (Ed25519)"))?;
    VerifyingKey::from_bytes(&array).with_context(|| "invalid Ed25519 verifying key material")
}

fn decode_signature(value: &str) -> Result<Signature> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value.trim())
        .with_context(|| "failed to decode signature: expected base64url (REQ-SEC-01)")?;
    let array: [u8; 64] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("signature must decode to 64 bytes (Ed25519)"))?;
    Ok(Signature::from_bytes(&array))
}

fn parse_timestamp(raw: &str) -> Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("invalid RFC3339 timestamp '{raw}' for --signed-at"))?;
    Ok(parsed.with_timezone(&Utc))
}

fn enforce_timestamp_bounds(signed_at: DateTime<Utc>) -> Result<()> {
    let now = Utc::now();
    let max_age = Duration::minutes(10);
    let max_future_skew = Duration::seconds(60);
    if now - signed_at > max_age {
        bail!(
            "signed command is older than {} minutes; re-authorise the action (REQ-SEC-01)",
            max_age.num_minutes()
        );
    }
    if signed_at - now > max_future_skew {
        bail!(
            "signed command is {} seconds in the future; check system clocks (REQ-SEC-01)",
            max_future_skew.num_seconds()
        );
    }
    Ok(())
}

fn build_envelope_message(
    outcome: &WizardOutcome,
    nonce: &str,
    signed_at: DateTime<Utc>,
) -> Result<Vec<u8>> {
    let summary_json = serde_json::to_string(&outcome.summary())
        .with_context(|| "failed to serialise wizard summary for signing")?;
    let payload = format!(
        "{ENVELOPE_CONTEXT}\nname={}\nsummary={}\nnonce={}\nsigned_at={}",
        outcome.name,
        summary_json,
        nonce,
        signed_at.to_rfc3339(),
    );
    Ok(payload.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_core::config_types::McpServerConfig;
    use ed25519_dalek::Signer;
    use ed25519_dalek::SigningKey;
    use std::time::SystemTime;

    fn sample_outcome() -> WizardOutcome {
        WizardOutcome {
            name: "demo".to_string(),
            server: McpServerConfig {
                command: "demo".to_string(),
                ..McpServerConfig::default()
            },
            template_id: Some("builtin/demo".to_string()),
            source: None,
            generated_at: SystemTime::now(),
        }
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    #[test]
    fn verify_accepts_valid_envelope() {
        let outcome = sample_outcome();
        let signing_key = signing_key();
        let verifying_key = signing_key.verifying_key();
        let signed_at = Utc::now();
        let nonce = "nonce-123456";
        let message = build_envelope_message(&outcome, nonce, signed_at).unwrap();
        let signature = signing_key.sign(&message);

        let args = WizardArgs {
            signing_key: Some(URL_SAFE_NO_PAD.encode(verifying_key.to_bytes())),
            signature: Some(URL_SAFE_NO_PAD.encode(signature.to_bytes())),
            signed_at: Some(signed_at.to_rfc3339()),
            nonce: Some(nonce.to_string()),
            ..WizardArgs::default()
        };

        let envelope = CommandSignatureEnvelope::from_args(&args)
            .expect("parsing envelope")
            .expect("envelope present");
        envelope.verify(&outcome).expect("signature should verify");
    }

    #[test]
    fn verify_rejects_stale_signature() {
        let outcome = sample_outcome();
        let signing_key = signing_key();
        let verifying_key = signing_key.verifying_key();
        let signed_at = Utc::now() - Duration::minutes(30);
        let nonce = "nonce-abcdef";
        let message = build_envelope_message(&outcome, nonce, signed_at).unwrap();
        let signature = signing_key.sign(&message);

        let args = WizardArgs {
            signing_key: Some(URL_SAFE_NO_PAD.encode(verifying_key.to_bytes())),
            signature: Some(URL_SAFE_NO_PAD.encode(signature.to_bytes())),
            signed_at: Some(signed_at.to_rfc3339()),
            nonce: Some(nonce.to_string()),
            ..WizardArgs::default()
        };

        let envelope = CommandSignatureEnvelope::from_args(&args)
            .expect("parsing envelope")
            .expect("envelope present");
        assert!(envelope.verify(&outcome).is_err());
    }
}
