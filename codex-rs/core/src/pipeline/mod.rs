use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::DateTime;
use chrono::Utc;
use ed25519_dalek::Signature;
use ed25519_dalek::Signer;
use ed25519_dalek::SigningKey;
use ed25519_dalek::Verifier;
use ed25519_dalek::VerifyingKey;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use tar::Archive;
use tar::Builder as TarBuilder;
use tar::Header;
use tempfile::TempDir;
use thiserror::Error;
use tracing::info;
use walkdir::WalkDir;

use crate::config::find_codex_home;
use crate::security::AuditEvent;
use crate::security::AuditEventKind;
use crate::security::LedgerError;
use crate::security::append_audit_event;

const PIPELINE_ROOT: &str = "pipeline";
const BUNDLES_DIR: &str = "bundles";
const MANIFESTS_DIR: &str = "manifests";
const SIGNATURES_DIR: &str = "signatures";
const INSTALLED_DIR: &str = "installed";
const STATE_DIR: &str = "state";
const PAYLOAD_DIR: &str = "payload";
const MANIFEST_FILE: &str = "manifest.json";
const SIGNATURE_FILE: &str = "signature.json";
const MANIFEST_SCHEMA_VERSION: u32 = 1;
const SIGNATURE_SCHEMA: &str = "stellar.pipeline.signature.v1";

/// Pipeline-level error domain (REQ-OPS-01, REQ-INT-01, REQ-DX-01).
#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("audit failure: {0}")]
    Audit(#[from] LedgerError),
    #[error("signature failure: {0}")]
    Signature(#[from] ed25519_dalek::SignatureError),
    #[error("version parse error: {0}")]
    Version(#[from] semver::Error),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("verification failure: {0}")]
    Verification(String),
}

/// Knowledge pack manifest persisted alongside signed bundles (REQ-OPS-01).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgePackManifest {
    pub schema_version: u32,
    pub name: String,
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub file_count: usize,
    pub total_bytes: u64,
    pub files: Vec<FileEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl KnowledgePackManifest {
    /// Compute the canonical JSON representation used for signing.
    fn canonical_bytes(&self) -> Result<Vec<u8>, PipelineError> {
        Ok(serde_json::to_vec(self)?)
    }

    /// Compute the SHA-256 digest of the manifest JSON.
    fn digest_hex(&self) -> Result<String, PipelineError> {
        Ok(sha256_hex(&self.canonical_bytes()?))
    }
}

/// Individual file descriptor captured in the manifest (REQ-OPS-01, REQ-DX-01).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Signed manifest envelope metadata (REQ-OPS-01).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignatureEnvelope {
    pub schema: String,
    pub signer: String,
    pub signed_at: DateTime<Utc>,
    pub nonce: String,
    pub verifying_key: String,
    pub signature: String,
    pub manifest_digest: String,
}

impl SignatureEnvelope {
    /// Decode the Ed25519 verifying key encoded in the envelope.
    pub fn verifying_key(&self) -> Result<VerifyingKey, PipelineError> {
        let raw = URL_SAFE_NO_PAD
            .decode(self.verifying_key.as_bytes())
            .map_err(|err| {
                PipelineError::InvalidInput(format!("failed to decode verifying key: {err}"))
            })?;
        let bytes: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
            PipelineError::InvalidInput("verifying key must be 32 bytes".to_string())
        })?;
        Ok(VerifyingKey::from_bytes(&bytes)?)
    }

    /// Decode the Ed25519 signature payload stored in the envelope.
    pub fn signature(&self) -> Result<Signature, PipelineError> {
        let raw = URL_SAFE_NO_PAD
            .decode(self.signature.as_bytes())
            .map_err(|err| {
                PipelineError::InvalidInput(format!("failed to decode signature: {err}"))
            })?;
        let bytes: [u8; 64] = raw
            .as_slice()
            .try_into()
            .map_err(|_| PipelineError::InvalidInput("signature must be 64 bytes".to_string()))?;
        Ok(Signature::from_bytes(&bytes))
    }

    /// Compute a short fingerprint for display and allowlists.
    pub fn fingerprint(&self) -> Result<String, PipelineError> {
        let key = self.verifying_key()?;
        Ok(key_fingerprint(&key))
    }
}

/// Reported delta between manifests during verification (REQ-INT-01).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestDiff {
    pub added: Vec<FileChange>,
    pub removed: Vec<FileChange>,
    pub modified: Vec<FileDelta>,
}

impl ManifestDiff {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDelta {
    pub path: String,
    pub previous_size_bytes: u64,
    pub previous_sha256: String,
    pub next_size_bytes: u64,
    pub next_sha256: String,
}

/// Filesystem-backed pipeline store rooted within CODEX_HOME.
#[derive(Debug, Clone)]
pub struct PipelineStore {
    root: PathBuf,
}

impl PipelineStore {
    /// Open (or initialise) the store rooted at the provided path.
    pub fn open(root: PathBuf) -> Result<Self, PipelineError> {
        let store = Self { root };
        store.ensure_layout()?;
        Ok(store)
    }

    /// Resolve the store using CODEX_HOME/pipeline.
    pub fn default() -> Result<Self, PipelineError> {
        let mut root = find_codex_home()?;
        root.push(PIPELINE_ROOT);
        Self::open(root)
    }

    fn ensure_layout(&self) -> Result<(), PipelineError> {
        for dir in [
            self.bundles_dir(),
            self.manifests_dir(),
            self.signatures_dir(),
            self.installed_dir(),
            self.state_dir(),
        ] {
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    fn bundles_dir(&self) -> PathBuf {
        self.root.join(BUNDLES_DIR)
    }

    fn manifests_dir(&self) -> PathBuf {
        self.root.join(MANIFESTS_DIR)
    }

    fn signatures_dir(&self) -> PathBuf {
        self.root.join(SIGNATURES_DIR)
    }

    fn installed_dir(&self) -> PathBuf {
        self.root.join(INSTALLED_DIR)
    }

    fn state_dir(&self) -> PathBuf {
        self.root.join(STATE_DIR)
    }

    fn bundle_path(&self, name: &str, version: &str) -> PathBuf {
        self.bundles_dir()
            .join(name)
            .join(format!("{version}.tar.gz"))
    }

    fn manifest_path(&self, name: &str, version: &str) -> PathBuf {
        self.manifests_dir()
            .join(name)
            .join(format!("{version}.json"))
    }

    fn signature_path(&self, name: &str, version: &str) -> PathBuf {
        self.signatures_dir()
            .join(name)
            .join(format!("{version}.json"))
    }

    fn installed_version_dir(&self, name: &str, version: &str) -> PathBuf {
        self.installed_dir().join(name).join(version)
    }

    fn state_file(&self, name: &str) -> PathBuf {
        self.state_dir().join(name).join("current")
    }

    fn write_state(&self, name: &str, version: &str) -> Result<(), PipelineError> {
        let state_dir = self.state_dir().join(name);
        fs::create_dir_all(&state_dir)?;
        fs::write(state_dir.join("current"), format!("{version}\n"))?;
        Ok(())
    }

    /// Return the currently active version for the given knowledge pack.
    pub fn active_version(&self, name: &str) -> Result<Option<String>, PipelineError> {
        let path = self.state_file(name);
        if !path.exists() {
            return Ok(None);
        }
        let value = fs::read_to_string(path)?;
        Ok(Some(value.trim().to_string()))
    }

    /// Load a manifest for the specified knowledge pack version, if present.
    pub fn load_manifest(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<KnowledgePackManifest>, PipelineError> {
        let path = self.manifest_path(name, version);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(path)?;
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    /// Load the manifest for the currently active version, if any.
    pub fn load_active_manifest(
        &self,
        name: &str,
    ) -> Result<Option<KnowledgePackManifest>, PipelineError> {
        let Some(version) = self.active_version(name)? else {
            return Ok(None);
        };
        self.load_manifest(name, &version)
    }
}

/// Request payload for signing a knowledge pack bundle.
pub struct SignRequest<'a> {
    pub name: &'a str,
    pub version: &'a Version,
    pub source_dir: &'a Path,
    pub signing_key: &'a SigningKey,
    pub signer: &'a str,
    pub actor: &'a str,
    pub notes: Option<&'a str>,
    pub timestamp: DateTime<Utc>,
    pub bundle_out: Option<&'a Path>,
}

/// Result of a successful signing operation.
pub struct SignOutcome {
    pub bundle_path: PathBuf,
    pub manifest_path: PathBuf,
    pub signature_path: PathBuf,
    pub manifest: KnowledgePackManifest,
    pub signature: SignatureEnvelope,
    pub manifest_digest: String,
}

/// Verify/install request payload.
pub struct VerifyRequest<'a> {
    pub bundle_path: &'a Path,
    pub expected_fingerprint: Option<&'a str>,
    pub install: bool,
    pub force_install: bool,
    pub actor: &'a str,
}

/// Verification outcome returned to callers.
pub struct VerifyOutcome {
    pub manifest: KnowledgePackManifest,
    pub signature: SignatureEnvelope,
    pub diff: ManifestDiff,
    pub previous_version: Option<String>,
    pub installed_path: Option<PathBuf>,
}

/// Rollback request payload.
pub struct RollbackRequest<'a> {
    pub name: &'a str,
    pub version: &'a Version,
    pub actor: &'a str,
}

/// Rollback result information.
pub struct RollbackOutcome {
    pub previous_active: Option<String>,
    pub new_active: String,
}

/// Sign and bundle a knowledge pack directory into a pipeline artifact.
///
/// # Parameters
/// * `store` - Pipeline store used to persist manifests, signatures, and bundles.
/// * `request` - Signing request describing the pack metadata and key material.
///
/// # Returns
/// Returns a [`SignOutcome`] containing the paths to the generated bundle,
/// manifest, and signature alongside parsed metadata.
///
/// # Errors
/// Returns [`PipelineError`] when validation fails, I/O operations cannot be
/// completed, or the audit ledger is unavailable.
///
/// # Examples
/// ```
/// use chrono::Utc;
/// use codex_core::pipeline::{PipelineStore, SignRequest, sign_knowledge_pack};
/// use ed25519_dalek::SigningKey;
/// use semver::Version;
/// # use tempfile::TempDir;
/// # let temp = TempDir::new().unwrap();
/// # let store = PipelineStore::open(temp.path().join("pipeline")).unwrap();
/// # let source_dir = temp.path().join("pack");
/// # std::fs::create_dir_all(&source_dir).unwrap();
/// # std::fs::write(source_dir.join("README.md"), "hello").unwrap();
/// let key = SigningKey::from_bytes(&[7u8; 32]);
/// let version = Version::parse("1.0.0").unwrap();
/// let request = SignRequest {
///     name: "demo",
///     version: &version,
///     source_dir: &source_dir,
///     signing_key: &key,
///     signer: "vault:pipeline/demo",
///     actor: "ci",
///     notes: None,
///     timestamp: Utc::now(),
///     bundle_out: None,
/// };
/// let outcome = sign_knowledge_pack(&store, request).unwrap();
/// assert_eq!(outcome.manifest.name, "demo");
/// ```
pub fn sign_knowledge_pack(
    store: &PipelineStore,
    request: SignRequest<'_>,
) -> Result<SignOutcome, PipelineError> {
    validate_pack_name(request.name)?;
    validate_source_dir(request.source_dir)?;

    let (files, total_bytes) = collect_file_entries(request.source_dir)?;
    if files.is_empty() {
        return Err(PipelineError::InvalidInput(
            "knowledge pack must contain at least one file".to_string(),
        ));
    }

    let manifest = KnowledgePackManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        name: request.name.to_string(),
        version: request.version.to_string(),
        created_at: request.timestamp,
        file_count: files.len(),
        total_bytes,
        files,
        notes: request.notes.map(ToOwned::to_owned),
    };
    let manifest_bytes = manifest.canonical_bytes()?;
    let manifest_digest = sha256_hex(&manifest_bytes);

    let verifying_key = request.signing_key.verifying_key();
    let signature = request.signing_key.sign(&manifest_bytes);
    let nonce = derive_nonce(&manifest_digest, request.signer, request.timestamp);

    let envelope = SignatureEnvelope {
        schema: SIGNATURE_SCHEMA.to_string(),
        signer: request.signer.to_string(),
        signed_at: request.timestamp,
        nonce: nonce.clone(),
        verifying_key: URL_SAFE_NO_PAD.encode(verifying_key.to_bytes()),
        signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
        manifest_digest: manifest_digest.clone(),
    };
    let signature_bytes = serde_json::to_vec(&envelope)?;

    let bundle_path = store.bundle_path(request.name, &manifest.version);
    write_bundle(
        &bundle_path,
        request.source_dir,
        &manifest_bytes,
        &signature_bytes,
    )?;

    let manifest_path = store.manifest_path(request.name, &manifest.version);
    ensure_parent_dir(&manifest_path)?;
    fs::write(&manifest_path, format_json(&manifest_bytes)?)?;

    let signature_path = store.signature_path(request.name, &manifest.version);
    ensure_parent_dir(&signature_path)?;
    fs::write(&signature_path, format_json(&signature_bytes)?)?;

    if let Some(extra) = request.bundle_out {
        if let Some(parent) = extra.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&bundle_path, extra)?;
    }

    let mut audit_event = AuditEvent::new(
        AuditEventKind::SupplyChain,
        request.actor,
        "sign",
        format!("knowledge-pack:{}", manifest.name),
    )
    .with_metadata("version", manifest.version.clone())
    .with_metadata("files", manifest.file_count.to_string())
    .with_metadata("bytes", manifest.total_bytes.to_string())
    .with_metadata("fingerprint", envelope.fingerprint()?);
    audit_event
        .metadata
        .insert("digest".into(), manifest_digest.clone());
    append_audit_event(audit_event)?;

    info!(
        op = "pipeline.sign",
        req_id = %nonce,
        name = %manifest.name,
        version = %manifest.version,
        digest = %manifest_digest,
        file_count = manifest.file_count,
        total_bytes = manifest.total_bytes,
        msg = "knowledge pack signed"
    );

    Ok(SignOutcome {
        bundle_path,
        manifest_path,
        signature_path,
        manifest,
        signature: envelope,
        manifest_digest,
    })
}

/// Verify a signed bundle and optionally install it into the pipeline store.
///
/// # Parameters
/// * `store` - Pipeline store handle.
/// * `request` - Verification configuration.
///
/// # Returns
/// [`VerifyOutcome`] containing the parsed metadata and diff against the
/// currently active knowledge pack (if any).
///
/// # Errors
/// Returns [`PipelineError`] when the bundle is malformed, signatures are
/// invalid, or installation fails.
pub fn verify_bundle(
    store: &PipelineStore,
    request: VerifyRequest<'_>,
) -> Result<VerifyOutcome, PipelineError> {
    if !request.bundle_path.is_file() {
        return Err(PipelineError::InvalidInput(format!(
            "bundle path {} is not a file",
            request.bundle_path.display()
        )));
    }

    let extracted = extract_bundle(request.bundle_path)?;
    validate_pack_name(&extracted.manifest.name)?;

    let manifest_digest = extracted.manifest.digest_hex()?;
    if manifest_digest != extracted.signature.manifest_digest {
        return Err(PipelineError::Verification(
            "manifest digest mismatch".to_string(),
        ));
    }

    let verifying_key = extracted.signature.verifying_key()?;
    verifying_key.verify(&extracted.manifest_bytes, &extracted.signature.signature()?)?;

    if let Some(expected) = request.expected_fingerprint {
        let expected_norm = expected.trim().to_ascii_lowercase();
        if extracted.signature.fingerprint()?.to_ascii_lowercase() != expected_norm {
            return Err(PipelineError::Verification(format!(
                "verifying key fingerprint mismatch (expected {expected}, got {})",
                extracted.signature.fingerprint()?
            )));
        }
    }

    verify_payload_contents(&extracted.manifest, &extracted.payload_dir)?;

    let previous_manifest = store.load_active_manifest(&extracted.manifest.name)?;
    let diff = diff_manifests(previous_manifest.as_ref(), &extracted.manifest);

    let mut previous_version = previous_manifest.as_ref().map(|m| m.version.clone());
    let mut installed_path = None;

    if request.install {
        let prev_active = install_bundle(
            store,
            &extracted,
            request.force_install,
            request.bundle_path,
        )?;
        previous_version = prev_active.or(previous_version);
        installed_path = Some(
            store.installed_version_dir(&extracted.manifest.name, &extracted.manifest.version),
        );

        let mut audit_event = AuditEvent::new(
            AuditEventKind::SupplyChain,
            request.actor,
            "install",
            format!("knowledge-pack:{}", extracted.manifest.name),
        )
        .with_metadata("version", extracted.manifest.version.clone())
        .with_metadata("fingerprint", extracted.signature.fingerprint()?)
        .with_metadata("files", extracted.manifest.file_count.to_string())
        .with_metadata("bytes", extracted.manifest.total_bytes.to_string());
        if let Some(prev) = previous_version.as_ref() {
            audit_event = audit_event.with_metadata("previous", prev.clone());
        }
        append_audit_event(audit_event)?;

        info!(
            op = "pipeline.install",
            req_id = %extracted.signature.nonce,
            name = %extracted.manifest.name,
            version = %extracted.manifest.version,
            previous = previous_version.clone().unwrap_or_else(|| "none".into()),
            msg = "knowledge pack installed"
        );
    }

    Ok(VerifyOutcome {
        manifest: extracted.manifest,
        signature: extracted.signature,
        diff,
        previous_version,
        installed_path,
    })
}

/// Roll back the active knowledge pack to a previously installed version.
///
/// # Parameters
/// * `store` - Pipeline store handle.
/// * `request` - Rollback instructions.
///
/// # Returns
/// [`RollbackOutcome`] containing the previous and new active versions.
///
/// # Errors
/// Returns [`PipelineError`] when the requested version is missing or the audit
/// ledger cannot be updated.
pub fn rollback(
    store: &PipelineStore,
    request: RollbackRequest<'_>,
) -> Result<RollbackOutcome, PipelineError> {
    validate_pack_name(request.name)?;
    let version_str = request.version.to_string();
    let installed = store.installed_version_dir(request.name, &version_str);
    if !installed.is_dir() {
        return Err(PipelineError::InvalidInput(format!(
            "version {version_str} is not installed for {}",
            request.name
        )));
    }

    let previous_active = store.active_version(request.name)?;
    store.write_state(request.name, &version_str)?;

    let mut audit_event = AuditEvent::new(
        AuditEventKind::SupplyChain,
        request.actor,
        "rollback",
        format!("knowledge-pack:{}", request.name),
    )
    .with_metadata("version", version_str.clone());
    if let Some(prev) = previous_active.as_ref() {
        audit_event = audit_event.with_metadata("previous", prev.clone());
    }
    append_audit_event(audit_event)?;

    info!(
        op = "pipeline.rollback",
        req_id = %format!("{}:{version_str}", request.name),
        name = %request.name,
        version = %version_str,
        previous = previous_active.clone().unwrap_or_else(|| "none".into()),
        msg = "knowledge pack rollback applied"
    );

    Ok(RollbackOutcome {
        previous_active,
        new_active: version_str,
    })
}

struct ExtractedBundle {
    _temp_dir: TempDir,
    manifest: KnowledgePackManifest,
    manifest_bytes: Vec<u8>,
    signature: SignatureEnvelope,
    signature_bytes: Vec<u8>,
    payload_dir: PathBuf,
}

fn extract_bundle(bundle_path: &Path) -> Result<ExtractedBundle, PipelineError> {
    let file = fs::File::open(bundle_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let temp_dir = TempDir::new()?;
    archive
        .unpack(temp_dir.path())
        .map_err(|err| PipelineError::InvalidInput(format!("failed to unpack bundle: {err}")))?;

    validate_bundle_layout(temp_dir.path())?;

    let manifest_path = temp_dir.path().join(MANIFEST_FILE);
    let signature_path = temp_dir.path().join(SIGNATURE_FILE);
    let payload_dir = temp_dir.path().join(PAYLOAD_DIR);

    let manifest_bytes = fs::read(&manifest_path)?;
    let signature_bytes = fs::read(&signature_path)?;

    let manifest: KnowledgePackManifest = serde_json::from_slice(&manifest_bytes)?;
    let signature: SignatureEnvelope = serde_json::from_slice(&signature_bytes)?;

    Ok(ExtractedBundle {
        _temp_dir: temp_dir,
        manifest,
        manifest_bytes,
        signature,
        signature_bytes,
        payload_dir,
    })
}

fn validate_bundle_layout(root: &Path) -> Result<(), PipelineError> {
    let mut allowed_top = HashMap::new();
    allowed_top.insert(MANIFEST_FILE.to_string(), false);
    allowed_top.insert(SIGNATURE_FILE.to_string(), false);
    allowed_top.insert(PAYLOAD_DIR.to_string(), false);

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        match allowed_top.get_mut(&name) {
            Some(flag) => {
                *flag = true;
            }
            None => {
                return Err(PipelineError::InvalidInput(format!(
                    "unexpected entry '{name}' in bundle"
                )));
            }
        }
    }

    for (key, seen) in allowed_top {
        if !seen {
            return Err(PipelineError::InvalidInput(format!(
                "bundle missing required entry '{key}'"
            )));
        }
    }
    Ok(())
}

fn install_bundle(
    store: &PipelineStore,
    extracted: &ExtractedBundle,
    force: bool,
    bundle_src: &Path,
) -> Result<Option<String>, PipelineError> {
    let name = &extracted.manifest.name;
    let version = &extracted.manifest.version;

    let dest_payload = store.installed_version_dir(name, version);
    if dest_payload.exists() {
        if !force {
            return Err(PipelineError::InvalidInput(format!(
                "version {version} already installed for {name}; use --force to overwrite"
            )));
        }
        fs::remove_dir_all(&dest_payload)?;
    }
    fs::create_dir_all(dest_payload.parent().unwrap())?;
    copy_dir_recursive(&extracted.payload_dir, &dest_payload)?;

    let manifest_path = store.manifest_path(name, version);
    ensure_parent_dir(&manifest_path)?;
    fs::write(&manifest_path, format_json(&extracted.manifest_bytes)?)?;

    let signature_path = store.signature_path(name, version);
    ensure_parent_dir(&signature_path)?;
    fs::write(&signature_path, format_json(&extracted.signature_bytes)?)?;

    let bundle_dest = store.bundle_path(name, version);
    ensure_parent_dir(&bundle_dest)?;
    fs::copy(bundle_src, &bundle_dest)?;

    let previous = store.active_version(name)?;
    store.write_state(name, version)?;

    Ok(previous)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), PipelineError> {
    fs::create_dir_all(dst)?;
    let mut entries: Vec<_> = WalkDir::new(src)
        .min_depth(1)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| {
            PipelineError::InvalidInput(format!(
                "failed to traverse directory {}: {err}",
                src.display()
            ))
        })?;
    entries.sort_by(|a, b| a.path().cmp(b.path()));
    for entry in entries {
        let rel = entry.path().strip_prefix(src).map_err(|_| {
            PipelineError::InvalidInput("failed to compute relative path".to_string())
        })?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        } else {
            return Err(PipelineError::InvalidInput(format!(
                "unsupported entry type at {}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

fn write_bundle(
    bundle_path: &Path,
    source_dir: &Path,
    manifest_bytes: &[u8],
    signature_bytes: &[u8],
) -> Result<(), PipelineError> {
    if let Some(parent) = bundle_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::File::create(bundle_path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = TarBuilder::new(encoder);
    append_bytes(&mut builder, MANIFEST_FILE, manifest_bytes)?;
    append_bytes(&mut builder, SIGNATURE_FILE, signature_bytes)?;

    let mut files = collect_payload_paths(source_dir)?;
    files.sort();
    for rel_path in files {
        let abs_path = source_dir.join(&rel_path);
        let mut header = Header::new_gnu();
        let metadata = fs::metadata(&abs_path)?;
        header.set_size(metadata.len());
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_cksum();
        let mut file = fs::File::open(&abs_path)?;
        let archive_path = format!("{PAYLOAD_DIR}/{}", rel_path.display());
        builder
            .append_data(&mut header, archive_path, &mut file)
            .map_err(|err| {
                PipelineError::InvalidInput(format!("failed to add file to bundle: {err}"))
            })?;
    }

    builder.finish().map_err(|err| {
        PipelineError::InvalidInput(format!("failed to finalize bundle archive: {err}"))
    })?;
    let encoder = builder.into_inner().map_err(|err| {
        PipelineError::InvalidInput(format!("failed to finalize archive writer: {err}"))
    })?;
    encoder.finish()?;
    Ok(())
}

fn append_bytes(
    builder: &mut TarBuilder<GzEncoder<fs::File>>,
    name: &str,
    bytes: &[u8],
) -> Result<(), PipelineError> {
    let mut header = Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_cksum();
    let mut cursor = std::io::Cursor::new(bytes);
    builder
        .append_data(&mut header, name, &mut cursor)
        .map_err(|err| {
            PipelineError::InvalidInput(format!("failed to add {name} to bundle: {err}"))
        })?;
    Ok(())
}

fn collect_payload_paths(source_dir: &Path) -> Result<Vec<PathBuf>, PipelineError> {
    let mut paths = Vec::new();
    for entry in WalkDir::new(source_dir).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(source_dir).map_err(|_| {
                PipelineError::InvalidInput("failed to compute relative path".to_string())
            })?;
            paths.push(rel.to_path_buf());
        }
    }
    Ok(paths)
}

fn validate_pack_name(name: &str) -> Result<(), PipelineError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(PipelineError::InvalidInput(format!(
            "invalid knowledge pack name '{name}'"
        )));
    }
    Ok(())
}

fn validate_source_dir(path: &Path) -> Result<(), PipelineError> {
    if !path.is_dir() {
        return Err(PipelineError::InvalidInput(format!(
            "source directory {} does not exist",
            path.display()
        )));
    }
    Ok(())
}

fn collect_file_entries(source_dir: &Path) -> Result<(Vec<FileEntry>, u64), PipelineError> {
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut entries: Vec<_> = WalkDir::new(source_dir)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| {
            PipelineError::InvalidInput(format!(
                "failed to traverse directory {}: {err}",
                source_dir.display()
            ))
        })?;
    entries.sort_by(|a, b| a.path().cmp(b.path()));
    for entry in entries {
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() {
            return Err(PipelineError::InvalidInput(format!(
                "unsupported entry type at {}",
                entry.path().display()
            )));
        }
        let rel = entry.path().strip_prefix(source_dir).map_err(|_| {
            PipelineError::InvalidInput("failed to compute relative path".to_string())
        })?;
        let rel_str = normalize_relative_path(rel)?;
        let (sha, size) = hash_file(entry.path())?;
        total = total.saturating_add(size);
        files.push(FileEntry {
            path: rel_str,
            size_bytes: size,
            sha256: sha,
        });
    }
    Ok((files, total))
}

fn verify_payload_contents(
    manifest: &KnowledgePackManifest,
    payload_dir: &Path,
) -> Result<(), PipelineError> {
    if !payload_dir.is_dir() {
        return Err(PipelineError::InvalidInput(
            "bundle payload directory missing".to_string(),
        ));
    }
    let mut expected: BTreeMap<&str, &FileEntry> = manifest
        .files
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();

    for entry in WalkDir::new(payload_dir).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() {
            return Err(PipelineError::InvalidInput(format!(
                "unsupported entry type in bundle payload at {}",
                entry.path().display()
            )));
        }
        let rel = entry.path().strip_prefix(payload_dir).map_err(|_| {
            PipelineError::InvalidInput("failed to compute relative path".to_string())
        })?;
        let rel_str = normalize_relative_path(rel)?;
        let Some(expected_entry) = expected.remove(rel_str.as_str()) else {
            return Err(PipelineError::Verification(format!(
                "payload contains unexpected file '{rel_str}'"
            )));
        };
        let (sha, size) = hash_file(entry.path())?;
        if sha != expected_entry.sha256 || size != expected_entry.size_bytes {
            return Err(PipelineError::Verification(format!(
                "payload mismatch for '{rel_str}'"
            )));
        }
    }

    if !expected.is_empty() {
        let missing: Vec<_> = expected.keys().cloned().collect();
        return Err(PipelineError::Verification(format!(
            "payload missing files: {}",
            missing.join(", ")
        )));
    }
    Ok(())
}

fn diff_manifests(
    previous: Option<&KnowledgePackManifest>,
    next: &KnowledgePackManifest,
) -> ManifestDiff {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();

    let mut previous_map = BTreeMap::new();
    if let Some(prev) = previous {
        for file in &prev.files {
            previous_map.insert(file.path.clone(), file);
        }
    }

    for file in &next.files {
        match previous_map.remove(&file.path) {
            None => added.push(FileChange {
                path: file.path.clone(),
                size_bytes: file.size_bytes,
                sha256: file.sha256.clone(),
            }),
            Some(prev_file) if prev_file.sha256 != file.sha256 => {
                modified.push(FileDelta {
                    path: file.path.clone(),
                    previous_size_bytes: prev_file.size_bytes,
                    previous_sha256: prev_file.sha256.clone(),
                    next_size_bytes: file.size_bytes,
                    next_sha256: file.sha256.clone(),
                });
            }
            Some(_) => {}
        }
    }

    for (path, file) in previous_map {
        removed.push(FileChange {
            path,
            size_bytes: file.size_bytes,
            sha256: file.sha256.clone(),
        });
    }

    ManifestDiff {
        added,
        removed,
        modified,
    }
}

fn normalize_relative_path(path: &Path) -> Result<String, PipelineError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(os) => parts.push(os.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PipelineError::InvalidInput(format!(
                    "invalid relative path component in {}",
                    path.display()
                )));
            }
        }
    }
    if parts.is_empty() {
        return Err(PipelineError::InvalidInput(
            "encountered empty file path".to_string(),
        ));
    }
    Ok(parts.join("/"))
}

fn hash_file(path: &Path) -> Result<(String, u64), PipelineError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((hex::encode(hasher.finalize()), total))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn derive_nonce(digest: &str, signer: &str, timestamp: DateTime<Utc>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(digest.as_bytes());
    hasher.update(signer.as_bytes());
    hasher.update(timestamp.timestamp_micros().to_be_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash[..16])
}

fn key_fingerprint(key: &VerifyingKey) -> String {
    let hash = Sha256::digest(key.to_bytes());
    hex::encode(&hash[..8])
}

fn ensure_parent_dir(path: &Path) -> Result<(), PipelineError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn format_json(bytes: &[u8]) -> Result<Vec<u8>, PipelineError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    Ok(serde_json::to_vec_pretty(&value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn fixed_timestamp() -> DateTime<Utc> {
        Utc.timestamp_opt(1_725_000_000, 0).unwrap()
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    #[test]
    fn sign_verify_install_and_rollback_flow() {
        let temp = TempDir::new().unwrap();
        let store = PipelineStore::open(temp.path().join("pipeline")).unwrap();
        let src = temp.path().join("pack");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("README.md"), "Hello world").unwrap();
        fs::create_dir_all(src.join("templates")).unwrap();
        fs::write(src.join("templates/main.txt"), "template").unwrap();

        let version = Version::parse("1.0.0").unwrap();
        let request = SignRequest {
            name: "demo",
            version: &version,
            source_dir: &src,
            signing_key: &signing_key(),
            signer: "vault:pipeline/demo",
            actor: "ci",
            notes: Some("initial"),
            timestamp: fixed_timestamp(),
            bundle_out: None,
        };
        let outcome = sign_knowledge_pack(&store, request).unwrap();
        assert_eq!(outcome.manifest.file_count, 2);
        assert!(outcome.bundle_path.exists());

        let verify = VerifyRequest {
            bundle_path: &outcome.bundle_path,
            expected_fingerprint: Some(&outcome.signature.fingerprint().unwrap()),
            install: true,
            force_install: false,
            actor: "ci",
        };
        let result = verify_bundle(&store, verify).unwrap();
        assert!(result.diff.added.len() >= 1);
        assert!(
            store
                .installed_version_dir("demo", &result.manifest.version)
                .exists()
        );
        assert_eq!(store.active_version("demo").unwrap().unwrap(), "1.0.0");

        let rollback_req = RollbackRequest {
            name: "demo",
            version: &Version::parse("1.0.0").unwrap(),
            actor: "platform",
        };
        let rollback_outcome = rollback(&store, rollback_req).unwrap();
        assert_eq!(rollback_outcome.new_active, "1.0.0");
    }

    #[test]
    fn diff_manifests_detects_changes() {
        let manifest = KnowledgePackManifest {
            schema_version: 1,
            name: "demo".into(),
            version: "1.0.0".into(),
            created_at: fixed_timestamp(),
            file_count: 2,
            total_bytes: 10,
            files: vec![
                FileEntry {
                    path: "a.txt".into(),
                    size_bytes: 4,
                    sha256: "aaa".into(),
                },
                FileEntry {
                    path: "b.txt".into(),
                    size_bytes: 6,
                    sha256: "bbb".into(),
                },
            ],
            notes: None,
        };
        let updated = KnowledgePackManifest {
            files: vec![
                FileEntry {
                    path: "a.txt".into(),
                    size_bytes: 4,
                    sha256: "ccc".into(),
                },
                FileEntry {
                    path: "c.txt".into(),
                    size_bytes: 7,
                    sha256: "ddd".into(),
                },
            ],
            file_count: 2,
            total_bytes: 11,
            ..manifest.clone()
        };
        let diff = diff_manifests(Some(&manifest), &updated);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.modified.len(), 1);
    }
}
