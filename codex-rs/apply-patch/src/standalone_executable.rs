use clap::Parser;
use clap::ValueEnum;
use std::fs;
use std::io::IsTerminal;
use std::io::Read;
use std::io::Write;
use std::io::{self};
use std::path::Path;
use std::path::PathBuf;

use crate::ApplyPatchConfig;
use crate::ApplyPatchError;
use crate::ConflictDiagnostic;
use crate::NewlineMode;
use crate::PatchReport;
use crate::PatchReportMode;
use crate::apply_patch_with_config;
use crate::emit_report;
use crate::report_to_json;
use crate::report_to_machine_json;
use time::Duration;
use time::OffsetDateTime;
use time::macros::format_description;

#[derive(Parser, Debug)]
#[command(
    name = "apply_patch",
    about = "Apply Serena-style *** Begin Patch blocks to the filesystem.",
    disable_help_subcommand = true
)]
struct Cli {
    /// Read patch content from the specified file instead of the command argument or STDIN.
    #[arg(short = 'f', long = "patch-file", value_name = "PATH")]
    patch_file: Option<PathBuf>,

    /// Treat file paths as relative to this directory (default: current directory).
    #[arg(short = 'C', long = "root", value_name = "PATH", default_value = ".")]
    root: PathBuf,

    /// Validate the patch and show the summary without writing changes.
    #[arg(long = "dry-run")]
    dry_run: bool,

    /// Suppress the human-readable summary in the output.
    #[arg(long = "no-summary")]
    no_summary: bool,

    /// Emit a single-line JSON report suitable for downstream automation.
    #[arg(long = "machine")]
    machine: bool,

    /// Select which outputs to emit. Defaults to both human and JSON.
    #[arg(long = "output-format", value_enum, default_value = "human")]
    output_format: OutputFormat,

    /// Write the JSON report to the given path in addition to STDOUT (if requested).
    #[arg(long = "json-path", value_name = "PATH")]
    json_path: Option<PathBuf>,

    /// Directory where structured logs are written (default: reports/logs).
    #[arg(long = "log-dir", value_name = "PATH", default_value = "reports/logs")]
    log_dir: PathBuf,

    /// Number of days to retain log files.
    #[arg(long = "log-retention-days", default_value_t = 14)]
    log_retention_days: u32,

    /// Maximum number of log files to keep.
    #[arg(long = "log-keep", default_value_t = 200)]
    log_keep: usize,

    /// Do not write JSON logs for this run.
    #[arg(long = "no-logs")]
    no_logs: bool,

    /// Directory where conflict hints are written when a patch fails.
    #[arg(
        long = "conflict-dir",
        value_name = "PATH",
        default_value = "reports/conflicts"
    )]
    conflict_dir: PathBuf,

    /// Emit verbose JSON (pretty-printed) when --output-format includes json.
    #[arg(long = "verbose")]
    verbose: bool,

    /// Encoding used when reading and writing files (default: utf-8).
    #[arg(long = "encoding", default_value = "utf-8")]
    encoding: String,

    /// Newline normalization strategy.
    #[arg(long = "newline", value_enum, default_value = "preserve")]
    newline: NewlineArg,

    /// Strip trailing spaces and tabs from each line before writing.
    #[arg(long = "strip-trailing-whitespace")]
    strip_trailing_whitespace: bool,

    /// Ensure output ends with a newline.
    #[arg(long = "final-newline")]
    final_newline: bool,

    /// Ensure output does not end with a newline.
    #[arg(long = "no-final-newline")]
    no_final_newline: bool,

    /// Do not preserve the original UNIX mode bits on updates.
    #[arg(long = "no-preserve-mode")]
    no_preserve_mode: bool,

    /// Do not preserve original access/modified timestamps on updates.
    #[arg(long = "no-preserve-times")]
    no_preserve_times: bool,

    /// Set the mode for newly created files (octal, e.g. 644).
    #[arg(long = "new-file-mode", value_parser = parse_octal_mode)]
    new_file_mode: Option<u32>,

    /// Inline patch payload. If omitted, read from --patch-file or STDIN.
    #[arg(value_name = "PATCH")]
    patch: Option<String>,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum OutputFormat {
    Human,
    Json,
    Both,
}

impl OutputFormat {
    fn includes_human(self) -> bool {
        matches!(self, Self::Human | Self::Both)
    }

    fn includes_json(self) -> bool {
        matches!(self, Self::Json | Self::Both)
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum NewlineArg {
    Preserve,
    Lf,
    Crlf,
    Native,
}

impl From<NewlineArg> for NewlineMode {
    fn from(value: NewlineArg) -> Self {
        match value {
            NewlineArg::Preserve => NewlineMode::Preserve,
            NewlineArg::Lf => NewlineMode::Lf,
            NewlineArg::Crlf => NewlineMode::Crlf,
            NewlineArg::Native => NewlineMode::Native,
        }
    }
}

fn parse_octal_mode(value: &str) -> Result<u32, String> {
    u32::from_str_radix(value, 8).map_err(|err| format!("invalid mode '{value}': {err}"))
}

pub fn main() -> ! {
    let exit_code = run_main();
    std::process::exit(exit_code);
}

pub fn run_main() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("{err}");
            return 2;
        }
    };

    match run(cli) {
        Ok(()) => 0,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("{message}");
            }
            1
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let patch = load_patch(&cli).map_err(|err| err.to_string())?;

    let mut config = ApplyPatchConfig {
        root: cli.root.clone(),
        ..ApplyPatchConfig::default()
    };
    config.encoding = cli.encoding.clone();
    config.normalization.newline = cli.newline.into();
    config.normalization.strip_trailing_whitespace = cli.strip_trailing_whitespace;
    config.normalization.ensure_final_newline = if cli.no_final_newline {
        Some(false)
    } else if cli.final_newline {
        Some(true)
    } else {
        None
    };
    config.preserve_mode = !cli.no_preserve_mode;
    config.preserve_times = !cli.no_preserve_times;
    config.new_file_mode = cli.new_file_mode;

    if cli.dry_run {
        config.mode = PatchReportMode::DryRun;
    }

    let machine_mode = cli.machine;
    let output_format = if machine_mode {
        OutputFormat::Json
    } else {
        cli.output_format
    };

    let log_dir = if cli.no_logs {
        None
    } else {
        Some(resolve_output_path(&cli.root, &cli.log_dir))
    };
    let logger = log_dir.map(|dir| RunLogger::new(dir, cli.log_retention_days, cli.log_keep));
    let conflict_writer = ConflictWriter::new(resolve_output_path(&cli.root, &cli.conflict_dir));

    let mut stdout = io::stdout();
    let emit_options = EmitOutputsOptions {
        machine_mode,
        output_format,
        show_summary: !cli.no_summary,
        verbose: cli.verbose,
        root: &cli.root,
        json_path: cli.json_path.as_deref(),
    };

    match apply_patch_with_config(&patch, &config) {
        Ok(report) => {
            emit_outputs(&report, &mut stdout, &emit_options)?;
            if let Some(logger) = logger.as_ref()
                && let Err(err) = logger.record(&report)
            {
                eprintln!("Failed to write apply_patch log: {err}");
            }
            Ok(())
        }
        Err(ApplyPatchError::Execution(exec_error)) => {
            emit_outputs(&exec_error.report, &mut stdout, &emit_options)?;
            if let Some(logger) = logger.as_ref()
                && let Err(err) = logger.record(&exec_error.report)
            {
                eprintln!("Failed to write apply_patch log: {err}");
            }
            if let Some(conflict) = exec_error.conflict() {
                match conflict_writer.write(conflict) {
                    Ok(Some(path)) => eprintln!("Conflict hint written to: {}", path.display()),
                    Ok(None) => {}
                    Err(err) => eprintln!("Failed to write conflict hint: {err}"),
                }
            }
            Err(exec_error.message)
        }
        Err(other) => Err(other.to_string()),
    }
}

struct EmitOutputsOptions<'a> {
    machine_mode: bool,
    output_format: OutputFormat,
    show_summary: bool,
    verbose: bool,
    root: &'a Path,
    json_path: Option<&'a Path>,
}

fn emit_outputs(
    report: &PatchReport,
    stdout: &mut impl Write,
    options: &EmitOutputsOptions<'_>,
) -> Result<(), String> {
    let machine_mode = options.machine_mode;
    let output_format = options.output_format;
    let show_summary = options.show_summary;
    let verbose = options.verbose;
    let root = options.root;
    let json_path = options.json_path;
    let mut wrote_human = false;

    if !machine_mode && output_format.includes_human() && show_summary {
        emit_report(stdout, report).map_err(|err| err.to_string())?;
        wrote_human = true;
    }

    let json_value = report_to_json(report);

    if machine_mode {
        let machine_json = report_to_machine_json(report);
        let json_str = serde_json::to_string(&machine_json).map_err(|err| err.to_string())?;
        writeln!(stdout, "{json_str}").map_err(|err| err.to_string())?;
    } else if output_format.includes_json() {
        if wrote_human {
            writeln!(stdout).map_err(|err| err.to_string())?;
        }
        let json_str = if verbose {
            serde_json::to_string_pretty(&json_value)
        } else {
            serde_json::to_string(&json_value)
        }
        .map_err(|err| err.to_string())?;
        writeln!(stdout, "{json_str}").map_err(|err| err.to_string())?;
    }

    if let Some(path) = json_path {
        let resolved = resolve_output_path(root, path);
        if let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let json_str = serde_json::to_string_pretty(&json_value).map_err(|err| err.to_string())?;
        fs::write(
            &resolved,
            format!(
                "{json_str}
"
            ),
        )
        .map_err(|err| err.to_string())?;
    }

    Ok(())
}

fn resolve_output_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

struct RunLogger {
    directory: PathBuf,
    retention_days: u32,
    max_files: usize,
}

impl RunLogger {
    fn new(directory: PathBuf, retention_days: u32, max_files: usize) -> Self {
        Self {
            directory,
            retention_days,
            max_files: max_files.max(1),
        }
    }

    fn record(&self, report: &PatchReport) -> io::Result<PathBuf> {
        fs::create_dir_all(&self.directory)?;
        self.cleanup()?;
        let base = timestamp_utc();
        let json = report_to_json(report);
        let payload = serde_json::to_string_pretty(&json).map_err(io::Error::other)?;
        let mut attempt = 0;
        loop {
            let filename = if attempt == 0 {
                format!("{base}.json")
            } else {
                format!("{base}_{attempt}.json")
            };
            let candidate = self.directory.join(filename);
            if candidate.exists() {
                attempt += 1;
                continue;
            }
            fs::write(
                &candidate,
                format!(
                    "{payload}
"
                ),
            )?;
            return Ok(candidate);
        }
    }

    fn cleanup(&self) -> io::Result<()> {
        if !self.directory.exists() {
            return Ok(());
        }
        let mut files = collect_json_files(&self.directory)?;
        if self.retention_days > 0 {
            let cutoff = OffsetDateTime::now_utc() - Duration::days(i64::from(self.retention_days));
            for path in &files {
                if let Ok(metadata) = fs::metadata(path)
                    && let Ok(modified) = metadata.modified()
                    && OffsetDateTime::from(modified) < cutoff
                {
                    let _ = fs::remove_file(path);
                }
            }
            files = collect_json_files(&self.directory)?;
        }
        if files.len() > self.max_files {
            for extra in files.iter().take(files.len() - self.max_files) {
                let _ = fs::remove_file(extra);
            }
        }
        Ok(())
    }
}

struct ConflictWriter {
    directory: PathBuf,
}

impl ConflictWriter {
    fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn write(&self, conflict: &ConflictDiagnostic) -> io::Result<Option<PathBuf>> {
        if conflict.diff_hint().is_empty() {
            return Ok(None);
        }
        fs::create_dir_all(&self.directory)?;
        let slug = sanitize_slug(&conflict.path);
        let base = format!("{}_{}", timestamp_utc(), slug);
        let hint = conflict.diff_hint().join(
            "
",
        );
        let mut attempt = 0;
        loop {
            let filename = if attempt == 0 {
                format!("{base}.diff")
            } else {
                format!("{base}_{attempt}.diff")
            };
            let candidate = self.directory.join(filename);
            if candidate.exists() {
                attempt += 1;
                continue;
            }
            fs::write(
                &candidate,
                format!(
                    "{hint}
"
                ),
            )?;
            return Ok(Some(candidate));
        }
    }
}

fn collect_json_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn timestamp_utc() -> String {
    let format = format_description!("[year][month][day]T[hour][minute][second]Z");
    let now = OffsetDateTime::now_utc();
    now.format(&format)
        .unwrap_or_else(|_| now.unix_timestamp().to_string())
}

fn sanitize_slug(path: &Path) -> String {
    let raw = path.display().to_string();
    let slug: String = raw
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    let trimmed = slug.trim_matches('_');
    if trimmed.is_empty() {
        "conflict".to_string()
    } else {
        trimmed.to_string()
    }
}

fn load_patch(cli: &Cli) -> io::Result<String> {
    if let Some(inline) = &cli.patch {
        return Ok(inline.to_string());
    }

    if let Some(path) = &cli.patch_file {
        return fs::read_to_string(path);
    }

    if io::stdin().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "No patch content provided. Supply via STDIN or --patch-file.",
        ));
    }

    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf)?;
    if buf.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "No patch content provided. Supply via STDIN or --patch-file.",
        ));
    }
    Ok(buf)
}
