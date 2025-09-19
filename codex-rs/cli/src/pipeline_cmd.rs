use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64_STANDARD;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64_URL_SAFE;
use chrono::SecondsFormat;
use chrono::Utc;
use clap::Parser;
use clap::Subcommand;
use codex_core::pipeline::FileChange;
use codex_core::pipeline::FileDelta;
use codex_core::pipeline::ManifestDiff;
use codex_core::pipeline::PipelineError;
use codex_core::pipeline::PipelineStore;
use codex_core::pipeline::RollbackOutcome;
use codex_core::pipeline::RollbackRequest;
use codex_core::pipeline::SignOutcome;
use codex_core::pipeline::SignRequest;
use codex_core::pipeline::VerifyOutcome;
use codex_core::pipeline::VerifyRequest;
use codex_core::pipeline::rollback;
use codex_core::pipeline::sign_knowledge_pack;
use codex_core::pipeline::verify_bundle;
use ed25519_dalek::SigningKey;
use semver::Version;

/// Signed pipeline management (REQ-OPS-01, REQ-INT-01, REQ-DX-01).
#[derive(Debug, Parser)]
pub struct PipelineCli {
    #[command(subcommand)]
    command: PipelineCommand,
}

#[derive(Debug, Subcommand)]
enum PipelineCommand {
    /// Sign a knowledge pack directory and produce a bundle.
    Sign(SignArgs),
    /// Verify a signed bundle and optionally install it.
    Verify(VerifyArgs),
    /// Roll back to a previously installed knowledge pack version.
    Rollback(RollbackArgs),
}

#[derive(Debug, Parser)]
pub struct SignArgs {
    /// Knowledge pack name (alphanumeric, '-', '_', '.').
    #[arg(value_name = "NAME")]
    name: String,
    /// Semantic version identifier (e.g. 1.2.3).
    #[arg(value_name = "VERSION")]
    version: String,
    /// Directory containing the knowledge pack contents.
    #[arg(long = "source", short = 's', value_name = "DIR")]
    source: PathBuf,
    /// Optional path to a base64-encoded Ed25519 signing key.
    #[arg(long = "signing-key", value_name = "PATH")]
    signing_key: Option<PathBuf>,
    /// Signing key identifier (e.g. Vault path, cosign identity).
    #[arg(long = "signer", value_name = "ID")]
    signer: String,
    /// Optional release notes embedded in the manifest.
    #[arg(long = "notes", value_name = "TEXT")]
    notes: Option<String>,
    /// Optional additional output path for the signed bundle.
    #[arg(long = "bundle-out", value_name = "PATH")]
    bundle_out: Option<PathBuf>,
    /// Actor recorded in the audit log (defaults to current user).
    #[arg(long = "actor", value_name = "ACTOR")]
    actor: Option<String>,
}

#[derive(Debug, Parser)]
pub struct VerifyArgs {
    /// Path to the signed bundle (`.tar.gz`).
    #[arg(value_name = "BUNDLE")]
    bundle: PathBuf,
    /// Expected verifying-key fingerprint (hex). When provided the fingerprint must match.
    #[arg(long = "expect-fingerprint", value_name = "HEX")]
    expect_fingerprint: Option<String>,
    /// Install the bundle into CODEX_HOME/pipeline after successful verification.
    #[arg(long = "install", default_value_t = false)]
    install: bool,
    /// Force reinstallation when the target version already exists.
    #[arg(long = "force", default_value_t = false)]
    force: bool,
    /// Actor recorded in the audit log (defaults to current user).
    #[arg(long = "actor", value_name = "ACTOR")]
    actor: Option<String>,
}

#[derive(Debug, Parser)]
pub struct RollbackArgs {
    /// Knowledge pack name.
    #[arg(value_name = "NAME")]
    name: String,
    /// Version to activate.
    #[arg(value_name = "VERSION")]
    version: String,
    /// Actor recorded in the audit log (defaults to current user).
    #[arg(long = "actor", value_name = "ACTOR")]
    actor: Option<String>,
}

pub fn run(cli: PipelineCli) -> Result<()> {
    match cli.command {
        PipelineCommand::Sign(args) => run_sign(args),
        PipelineCommand::Verify(args) => run_verify(args),
        PipelineCommand::Rollback(args) => run_rollback(args),
    }
}

fn run_sign(args: SignArgs) -> Result<()> {
    let store = PipelineStore::default()
        .map_err(to_anyhow)
        .context("failed to open pipeline store")?;
    let version = Version::parse(&args.version).context("invalid semantic version")?;
    let signing_key = load_signing_key(&args)?;
    let actor = args.actor.unwrap_or_else(default_actor);
    let notes = args.notes.as_deref();
    let bundle_out = args.bundle_out.as_deref();

    let request = SignRequest {
        name: &args.name,
        version: &version,
        source_dir: &args.source,
        signing_key: &signing_key,
        signer: &args.signer,
        actor: &actor,
        notes,
        timestamp: Utc::now(),
        bundle_out,
    };

    let outcome = sign_knowledge_pack(&store, request)
        .map_err(to_anyhow)
        .context("failed to sign knowledge pack")?;
    print_sign_outcome(&outcome)?;
    Ok(())
}

fn run_verify(args: VerifyArgs) -> Result<()> {
    if args.force && !args.install {
        return Err(anyhow!("--force may only be used together with --install"));
    }
    let store = PipelineStore::default()
        .map_err(to_anyhow)
        .context("failed to open pipeline store")?;
    let actor = args.actor.unwrap_or_else(default_actor);
    let request = VerifyRequest {
        bundle_path: &args.bundle,
        expected_fingerprint: args.expect_fingerprint.as_deref(),
        install: args.install,
        force_install: args.force,
        actor: &actor,
    };
    let outcome = verify_bundle(&store, request)
        .map_err(to_anyhow)
        .context("verification failed")?;
    print_verify_outcome(&outcome)?;
    Ok(())
}

fn run_rollback(args: RollbackArgs) -> Result<()> {
    let store = PipelineStore::default()
        .map_err(to_anyhow)
        .context("failed to open pipeline store")?;
    let version = Version::parse(&args.version).context("invalid semantic version")?;
    let actor = args.actor.unwrap_or_else(default_actor);
    let request = RollbackRequest {
        name: &args.name,
        version: &version,
        actor: &actor,
    };
    let outcome = rollback(&store, request)
        .map_err(to_anyhow)
        .context("rollback failed")?;
    print_rollback_outcome(&args.name, &args.version, &outcome);
    Ok(())
}

fn load_signing_key(args: &SignArgs) -> Result<SigningKey> {
    let raw = if let Some(path) = &args.signing_key {
        fs::read_to_string(path)
            .with_context(|| format!("failed to read signing key from {}", path.display()))?
    } else {
        std::env::var("CODEX_PIPELINE_SIGNING_KEY")
            .context("CODEX_PIPELINE_SIGNING_KEY is not set and --signing-key was not provided")?
    };
    let cleaned = raw.trim();
    let decoded = B64_URL_SAFE
        .decode(cleaned.as_bytes())
        .or_else(|_| B64_STANDARD.decode(cleaned.as_bytes()))
        .context("signing key is not valid base64 (url or standard)")?;
    if decoded.len() != 32 {
        return Err(anyhow!(
            "signing key must decode to 32 bytes (Ed25519 secret key)"
        ));
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&decoded);
    Ok(SigningKey::from_bytes(&bytes))
}

fn print_sign_outcome(outcome: &SignOutcome) -> Result<()> {
    let fingerprint = outcome
        .signature
        .fingerprint()
        .map_err(to_anyhow)
        .context("failed to compute verifying key fingerprint")?;
    println!("Signed knowledge pack bundle");
    println!("  Name: {}", outcome.manifest.name);
    println!("  Version: {}", outcome.manifest.version);
    println!(
        "  Files: {} ({} bytes)",
        outcome.manifest.file_count, outcome.manifest.total_bytes
    );
    println!("  Manifest digest: {}", outcome.manifest_digest);
    println!("  Verifying fingerprint: {}", fingerprint);
    println!("  Bundle stored at: {}", outcome.bundle_path.display());
    println!("  Manifest stored at: {}", outcome.manifest_path.display());
    println!(
        "  Signature stored at: {}",
        outcome.signature_path.display()
    );
    Ok(())
}

fn print_verify_outcome(outcome: &VerifyOutcome) -> Result<()> {
    let fingerprint = outcome
        .signature
        .fingerprint()
        .map_err(to_anyhow)
        .context("failed to compute verifying key fingerprint")?;
    println!(
        "Verified knowledge pack {} v{}",
        outcome.manifest.name, outcome.manifest.version
    );
    println!(
        "  Signed at: {}",
        outcome
            .signature
            .signed_at
            .to_rfc3339_opts(SecondsFormat::Secs, true)
    );
    println!("  Verifying fingerprint: {}", fingerprint);
    if let Some(prev) = &outcome.previous_version {
        println!("  Previous active version: {}", prev);
    } else {
        println!("  Previous active version: none");
    }
    print_diff(&outcome.diff);
    if let Some(path) = &outcome.installed_path {
        println!("  Installed payload at: {}", path.display());
    }
    Ok(())
}

fn print_diff(diff: &ManifestDiff) {
    if diff.is_empty() {
        println!("  Diff vs active: no file changes");
        return;
    }
    if !diff.added.is_empty() {
        println!("  Added files:");
        for FileChange {
            path,
            size_bytes,
            sha256,
        } in &diff.added
        {
            println!("    + {} ({} bytes, {})", path, size_bytes, sha256);
        }
    }
    if !diff.removed.is_empty() {
        println!("  Removed files:");
        for FileChange {
            path,
            size_bytes,
            sha256,
        } in &diff.removed
        {
            println!("    - {} ({} bytes, {})", path, size_bytes, sha256);
        }
    }
    if !diff.modified.is_empty() {
        println!("  Modified files:");
        for FileDelta {
            path,
            previous_size_bytes,
            previous_sha256,
            next_size_bytes,
            next_sha256,
        } in &diff.modified
        {
            println!(
                "    ~ {} ({} bytes -> {} bytes, {} -> {})",
                path, previous_size_bytes, next_size_bytes, previous_sha256, next_sha256
            );
        }
    }
}

fn print_rollback_outcome(name: &str, version: &str, outcome: &RollbackOutcome) {
    if let Some(prev) = &outcome.previous_active {
        println!(
            "Rolled back {} to version {} (previous active {}).",
            name, version, prev
        );
    } else {
        println!(
            "Activated {} version {} (no previous active version).",
            name, version
        );
    }
}

fn default_actor() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "codex-cli".to_string())
}

fn to_anyhow(err: PipelineError) -> anyhow::Error {
    anyhow!(err)
}
