use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{self, Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};

fn validate_truncate_dim(dim: usize) -> Result<()> {
    if (128..=768).contains(&dim) {
        Ok(())
    } else {
        bail!("truncate-dim должен быть в диапазоне 128..=768 (получено {dim})");
    }
}

#[derive(Debug, Parser)]
pub struct MemoryCli {
    #[command(subcommand)]
    command: MemoryCommand,
}

#[derive(Debug, Subcommand)]
enum MemoryCommand {
    Remember(MemoryRememberArgs),
    Forget(MemoryForgetArgs),
    List(MemoryListArgs),
    Search(MemorySearchArgs),
    Prune(MemoryPruneArgs),
}

#[derive(Debug, Parser)]
struct MemoryRememberArgs {
    #[arg(value_name = "TEXT")]
    text: Option<String>,

    #[arg(long = "file", value_name = "FILE")]
    file: Option<PathBuf>,

    #[arg(long = "tag", value_name = "TAG")]
    tags: Vec<String>,

    #[arg(long = "source", value_name = "LABEL")]
    source: Option<String>,

    #[arg(long = "importance", value_name = "LEVEL")]
    importance: Option<String>,

    #[arg(long = "ttl", value_name = "DURATION")]
    ttl: Option<String>,

    #[arg(long = "pinned")]
    pinned: bool,

    #[arg(long = "replace", value_name = "ID")]
    replace: Option<String>,

    #[arg(long = "memory", value_name = "FILE")]
    memory: Option<PathBuf>,

    #[arg(long = "model-path", value_name = "DIR")]
    model_path: Option<PathBuf>,

    #[arg(long = "device", value_enum)]
    device: Option<DeviceArg>,

    #[arg(long = "truncate-dim", value_name = "D")]
    truncate_dim: Option<usize>,
}

#[derive(Debug, Parser)]
struct MemoryForgetArgs {
    #[arg(long = "id", value_name = "ID")]
    ids: Vec<String>,

    #[arg(long = "tag", value_name = "TAG")]
    tags: Vec<String>,

    #[arg(long = "memory", value_name = "FILE")]
    memory: Option<PathBuf>,
}

#[derive(Debug, Parser)]
struct MemoryListArgs {
    #[arg(long = "tag", value_name = "TAG")]
    tags: Vec<String>,

    #[arg(long = "memory", value_name = "FILE")]
    memory: Option<PathBuf>,
}

#[derive(Debug, Parser)]
struct MemorySearchArgs {
    query: String,

    #[arg(long = "top-k", value_name = "K")]
    top_k: Option<usize>,

    #[arg(long = "tag", value_name = "TAG")]
    tags: Vec<String>,

    #[arg(long = "show-text")]
    show_text: bool,

    #[arg(long = "memory", value_name = "FILE")]
    memory: Option<PathBuf>,

    #[arg(long = "model-path", value_name = "DIR")]
    model_path: Option<PathBuf>,

    #[arg(long = "device", value_enum)]
    device: Option<DeviceArg>,

    #[arg(long = "truncate-dim", value_name = "D")]
    truncate_dim: Option<usize>,
}

#[derive(Debug, Parser)]
struct MemoryPruneArgs {
    #[arg(long = "memory", value_name = "FILE")]
    memory: Option<PathBuf>,

    #[arg(long = "max-records", value_name = "N")]
    max_records: Option<usize>,

    #[arg(long = "older-than", value_name = "ISO8601")]
    older_than: Option<String>,

    #[arg(long = "drop-expired")]
    drop_expired: bool,

    #[arg(long = "keep-expired")]
    keep_expired: bool,

    #[arg(long = "dedupe")]
    dedupe: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum DeviceArg {
    Cpu,
    Cuda,
    Auto,
}

impl DeviceArg {
    fn as_str(&self) -> &'static str {
        match self {
            DeviceArg::Cpu => "cpu",
            DeviceArg::Cuda => "cuda",
            DeviceArg::Auto => "auto",
        }
    }
}

impl MemoryRememberArgs {
    fn to_python_args(&self) -> Result<Vec<String>> {
        if self.text.is_none() && self.file.is_none() {
            bail!("укажите текст записи или --file");
        }
        let mut args = vec!["remember".to_string()];
        if let Some(text) = &self.text {
            args.push(text.clone());
        }
        if let Some(file) = &self.file {
            args.push("--file".into());
            args.push(file.display().to_string());
        }
        for tag in &self.tags {
            args.push("--tag".into());
            args.push(tag.clone());
        }
        if let Some(source) = &self.source {
            args.push("--source".into());
            args.push(source.clone());
        }
        if let Some(importance) = &self.importance {
            args.push("--importance".into());
            args.push(importance.clone());
        }
        if let Some(ttl) = &self.ttl {
            args.push("--ttl".into());
            args.push(ttl.clone());
        }
        if self.pinned {
            args.push("--pinned".into());
        }
        if let Some(replace) = &self.replace {
            args.push("--replace".into());
            args.push(replace.clone());
        }
        if let Some(memory) = &self.memory {
            args.push("--memory".into());
            args.push(memory.display().to_string());
        }
        if let Some(model_path) = &self.model_path {
            args.push("--model-path".into());
            args.push(model_path.display().to_string());
        }
        if let Some(device) = &self.device {
            args.push("--device".into());
            args.push(device.as_str().into());
        }
        if let Some(dim) = self.truncate_dim {
            validate_truncate_dim(dim)?;
            args.push("--truncate-dim".into());
            args.push(dim.to_string());
        }
        Ok(args)
    }
}

impl MemoryForgetArgs {
    fn to_python_args(&self) -> Result<Vec<String>> {
        if self.ids.is_empty() && self.tags.is_empty() {
            bail!("укажите хотя бы --id или --tag");
        }
        let mut args = vec!["forget".to_string()];
        for id in &self.ids {
            args.push("--id".into());
            args.push(id.clone());
        }
        for tag in &self.tags {
            args.push("--tag".into());
            args.push(tag.clone());
        }
        if let Some(memory) = &self.memory {
            args.push("--memory".into());
            args.push(memory.display().to_string());
        }
        Ok(args)
    }
}

impl MemoryListArgs {
    fn to_python_args(&self) -> Vec<String> {
        let mut args = vec!["list".to_string()];
        for tag in &self.tags {
            args.push("--tag".into());
            args.push(tag.clone());
        }
        if let Some(memory) = &self.memory {
            args.push("--memory".into());
            args.push(memory.display().to_string());
        }
        args
    }
}

impl MemorySearchArgs {
    fn to_python_args(&self) -> Result<Vec<String>> {
        let mut args = vec!["search".to_string(), self.query.clone()];
        if let Some(top_k) = self.top_k {
            args.push("--top-k".into());
            args.push(top_k.to_string());
        }
        for tag in &self.tags {
            args.push("--tag".into());
            args.push(tag.clone());
        }
        if self.show_text {
            args.push("--show-text".into());
        }
        if let Some(memory) = &self.memory {
            args.push("--memory".into());
            args.push(memory.display().to_string());
        }
        if let Some(model_path) = &self.model_path {
            args.push("--model-path".into());
            args.push(model_path.display().to_string());
        }
        if let Some(device) = &self.device {
            args.push("--device".into());
            args.push(device.as_str().into());
        }
        if let Some(dim) = self.truncate_dim {
            validate_truncate_dim(dim)?;
            args.push("--truncate-dim".into());
            args.push(dim.to_string());
        }
        Ok(args)
    }
}

impl MemoryPruneArgs {
    fn to_python_args(&self) -> Vec<String> {
        let mut args = vec!["prune".to_string()];
        if let Some(memory) = &self.memory {
            args.push("--memory".into());
            args.push(memory.display().to_string());
        }
        if let Some(max_records) = self.max_records {
            args.push("--max-records".into());
            args.push(max_records.to_string());
        }
        if let Some(older_than) = &self.older_than {
            args.push("--older-than".into());
            args.push(older_than.clone());
        }
        if self.drop_expired {
            args.push("--drop-expired".into());
        }
        if self.keep_expired {
            args.push("--keep-expired".into());
        }
        if self.dedupe {
            args.push("--dedupe".into());
        }
        args
    }
}

fn spawn_python(
    python: &str,
    module: &str,
    args: &[String],
    cwd: &Path,
    pythonpath: &str,
) -> Result<std::process::Output> {
    let mut command = Command::new(python);
    command.arg("-m").arg(module).args(args);
    command.current_dir(cwd);
    command.env("PYTHONPATH", pythonpath);
    Ok(command.output()?)
}

fn install_requirements(python: &str, cwd: &Path) -> Result<()> {
    let status = Command::new(python)
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("-r")
        .arg("requirements-docsearch.txt")
        .current_dir(cwd)
        .status()
        .context("failed to install docsearch requirements")?;
    if !status.success() {
        bail!("pip install returned non-zero status {status}");
    }
    Ok(())
}

fn needs_dependency_install(stderr: &str) -> bool {
    stderr.contains("ModuleNotFoundError")
        || stderr.contains("ImportError: No module named")
        || stderr.contains("sentence_transformers")
        || stderr.contains("urllib3")
        || stderr.contains("requests")
        || stderr.contains("rapidfuzz")
}

fn run_python(module: &str, args: &[String]) -> Result<()> {
    let python = env::var("PYTHON").unwrap_or_else(|_| "python".to_string());
    let repo_root = env::current_dir().context("failed to determine working directory")?;

    let repo_root_str = repo_root
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("workspace path contains non-UTF8 characters"))?;
    let separator = if cfg!(windows) { ";" } else { ":" };
    let pythonpath = match env::var("PYTHONPATH") {
        Ok(existing) if !existing.is_empty() => format!(
            "{repo}{sep}{existing}",
            repo = repo_root_str,
            sep = separator
        ),
        _ => repo_root_str.to_owned(),
    };

    let mut attempted_install = false;

    loop {
        let output = spawn_python(&python, module, args, &repo_root, &pythonpath)
            .with_context(|| format!("failed to launch Python module {module}"))?;

        if output.status.success() {
            if !output.stdout.is_empty() {
                print!("{}", String::from_utf8_lossy(&output.stdout));
            }
            if !output.stderr.is_empty() {
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
            }
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if !attempted_install && needs_dependency_install(&stderr) {
            install_requirements(&python, &repo_root)?;
            attempted_install = true;
            continue;
        }

        if stderr.contains("failed to map segment from shared object") {
            bail!(
                "docsearch backend failed to load PyTorch (libtorch_cpu.so). \n                 Выполните `codex memory ...` вне песочницы Codex CLI или увеличьте лимит памяти."
            );
        }

        bail!(
            "memory backend exited with status {}\n{}",
            output.status,
            stderr
        );
    }
}

pub fn run(cli: MemoryCli) -> Result<()> {
    match cli.command {
        MemoryCommand::Remember(args) => {
            let python_args = args.to_python_args()?;
            run_python("scripts.docsearch.memory", &python_args)
        }
        MemoryCommand::Forget(args) => {
            let python_args = args.to_python_args()?;
            run_python("scripts.docsearch.memory", &python_args)
        }
        MemoryCommand::List(args) => {
            let python_args = args.to_python_args();
            run_python("scripts.docsearch.memory", &python_args)
        }
        MemoryCommand::Search(args) => {
            let python_args = args.to_python_args()?;
            run_python("scripts.docsearch.memory", &python_args)
        }
        MemoryCommand::Prune(args) => {
            let python_args = args.to_python_args();
            run_python("scripts.docsearch.memory", &python_args)
        }
    }
}
