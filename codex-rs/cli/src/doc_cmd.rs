use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{self, Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};

const DEFAULT_INDEX_PATH: &str = "docs/.docsearch/index.jsonl";

/// Semantic documentation search helpers powered by EmbeddingGemma-300M.
#[derive(Debug, Parser)]
pub struct DocCli {
    #[command(subcommand)]
    command: DocCommand,
}

#[derive(Debug, Subcommand)]
enum DocCommand {
    /// Построить или обновить индекс документации.
    Index(DocIndexArgs),
    /// Выполнить семантический поиск по индексу.
    Search(DocSearchArgs),
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

#[derive(Debug, Parser)]
struct DocIndexArgs {
    /// Каталог с документацией.
    #[arg(value_name = "DOCS_ROOT")]
    docs_root: Option<PathBuf>,

    /// Путь до файла индекса (JSONL).
    #[arg(long = "output", value_name = "FILE")]
    output: Option<PathBuf>,

    /// Минимальное количество символов в чанке.
    #[arg(long = "min-chars", value_name = "N")]
    min_chars: Option<usize>,

    /// Максимальное количество символов в чанке.
    #[arg(long = "max-chars", value_name = "N")]
    max_chars: Option<usize>,

    /// Размер батча при инференсе.
    #[arg(long = "batch-size", value_name = "N")]
    batch_size: Option<usize>,

    /// Рекурсивно обходить подкаталоги.
    #[arg(long = "recursive")]
    recursive: bool,

    /// Путь к локальной папке EmbeddingGemma-300M.
    #[arg(long = "model-path", value_name = "DIR")]
    model_path: Option<PathBuf>,

    /// Устройство инференса sentence-transformers.
    #[arg(long = "device", value_enum)]
    device: Option<DeviceArg>,

    /// Размерность эмбеддинга после усечения Matryoshka (768/512/256/128).
    #[arg(long = "truncate-dim", value_name = "D", value_parser = clap::value_parser!(usize).range(128..=768))]
    truncate_dim: Option<usize>,
}

impl DocIndexArgs {
    fn to_python_args(&self) -> Result<Vec<String>> {
        let mut args = Vec::new();
        if let Some(docs_root) = &self.docs_root {
            args.push(docs_root.display().to_string());
        }
        if let Some(output) = &self.output {
            args.push("--output".into());
            args.push(output.display().to_string());
        }
        if let Some(min_chars) = self.min_chars {
            args.push("--min-chars".into());
            args.push(min_chars.to_string());
        }
        if let Some(max_chars) = self.max_chars {
            args.push("--max-chars".into());
            args.push(max_chars.to_string());
        }
        if let Some(batch_size) = self.batch_size {
            args.push("--batch-size".into());
            args.push(batch_size.to_string());
        }
        if self.recursive {
            args.push("--recursive".into());
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

#[derive(Debug, Parser)]
struct DocSearchArgs {
    /// Текст запроса для семантического поиска.
    query: String,

    /// Файл индекса, созданный `codex doc index`.
    #[arg(long = "index", value_name = "FILE")]
    index: Option<PathBuf>,

    /// Количество совпадений в выдаче.
    #[arg(long = "top-k", value_name = "K")]
    top_k: Option<usize>,

    /// Показывать текстовые чанки в выдаче.
    #[arg(long = "show-text")]
    show_text: bool,

    /// Путь к локальной папке EmbeddingGemma-300M.
    #[arg(long = "model-path", value_name = "DIR")]
    model_path: Option<PathBuf>,

    /// Устройство инференса sentence-transformers.
    #[arg(long = "device", value_enum)]
    device: Option<DeviceArg>,

    /// Размерность эмбеддинга после усечения Matryoshka.
    #[arg(long = "truncate-dim", value_name = "D", value_parser = clap::value_parser!(usize).range(128..=768))]
    truncate_dim: Option<usize>,
}

impl DocSearchArgs {
    fn to_python_args(&self) -> Result<Vec<String>> {
        let mut args = vec![self.query.clone()];
        if let Some(index) = &self.index {
            args.push("--index".into());
            args.push(index.display().to_string());
        }
        if let Some(top_k) = self.top_k {
            args.push("--top-k".into());
            args.push(top_k.to_string());
        }
        if self.show_text {
            args.push("--show-text".into());
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
        || stderr.contains("No module named")
        || stderr.contains("sentence_transformers")
        || stderr.contains("urllib3")
        || stderr.contains("requests")
}

fn validate_truncate_dim(dim: usize) -> Result<()> {
    match dim {
        768 | 512 | 256 | 128 => Ok(()),
        _ => bail!("unsupported truncate-dim: {dim}. Допустимые значения: 768, 512, 256, 128"),
    }
}

pub fn run(cli: DocCli) -> Result<()> {
    match cli.command {
        DocCommand::Index(mut args) => {
            if args.docs_root.is_none() {
                args.docs_root = Some(PathBuf::from("docs"));
            }
            if args.output.is_none() {
                args.output = Some(PathBuf::from(DEFAULT_INDEX_PATH));
            }
            let python_args = args.to_python_args()?;
            run_python("scripts.docsearch.index", &python_args)
        }
        DocCommand::Search(mut args) => {
            let index_path = args
                .index
                .clone()
                .unwrap_or_else(|| PathBuf::from(DEFAULT_INDEX_PATH));

            if !index_path.exists() {
                let mut index_args = DocIndexArgs {
                    docs_root: Some(PathBuf::from("docs")),
                    output: Some(index_path.clone()),
                    min_chars: None,
                    max_chars: None,
                    batch_size: None,
                    recursive: true,
                    model_path: args.model_path.clone(),
                    device: args.device.clone(),
                    truncate_dim: args.truncate_dim,
                };
                let python_args = index_args.to_python_args()?;
                run_python("scripts.docsearch.index", &python_args)?;
            }

            args.index = Some(index_path);
            let python_args = args.to_python_args()?;
            run_python("scripts.docsearch.query", &python_args)
        }
    }
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
                "docsearch backend failed to load PyTorch (libtorch_cpu.so). 
                 Выполните `codex doc index`/`codex doc search` вне песочницы Codex CLI или увеличьте лимит памяти."
            );
        }

        bail!(
            "docsearch backend exited with status {}
{}",
            output.status,
            stderr
        );
    }
}
