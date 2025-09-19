use std::ffi::OsString;
use std::io::{self, Read};

use thiserror::Error;

/// Configuration switches controlling how the patch application workflow should run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliConfig {
    pub interactive_mode: InteractiveMode,
    pub preview_mode: PreviewMode,
    pub dry_run: bool,
    pub assume_yes: bool,
    pub run_after: Vec<String>,
    pub summary_mode: SummaryMode,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            interactive_mode: InteractiveMode::Auto,
            preview_mode: PreviewMode::Auto,
            dry_run: false,
            assume_yes: false,
            run_after: Vec::new(),
            summary_mode: SummaryMode::Standard,
        }
    }
}

impl CliConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractiveMode {
    Auto,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryMode {
    Quiet,
    Standard,
    Detailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Apply { patch: String },
    UndoLast,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInvocation {
    pub config: CliConfig,
    pub command: Command,
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("help requested")]
    DisplayedHelp,
    #[error("Error: apply_patch requires a UTF-8 PATCH argument.")]
    InvalidPatchUtf8,
    #[error("Error: apply_patch accepts exactly one argument.")]
    TooManyArguments,
    #[error("Error: Failed to read PATCH from stdin.\n{0}")]
    FailedToReadPatch(io::Error),
    #[error("Usage: apply_patch 'PATCH'\n       echo 'PATCH' | apply-patch")]
    MissingPatch,
    #[error("`--run-after` requires a non-empty command value.")]
    MissingRunAfterValue,
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    FlagConflict(String),
}

const USAGE: &str = "\
apply_patch [OPTIONS] [PATCH]\n\n\
Options:\n\
  -i, --interactive        Enable interactive hunk selection (default: auto)\n\
      --no-interactive     Disable interactive hunk selection\n\
  -n, --dry-run            Validate and preview without writing changes\n\
      --preview            Force a diff preview before applying\n\
      --no-preview         Skip diff preview even when interactive\n\
  -y, --yes                Assume 'yes' for interactive confirmations\n\
      --run-after <cmd>    Run a shell command after a successful apply\n\
      --undo-last          Revert the last applied patch\n\
  -q, --quiet              Suppress success summary output\n\
      --detailed-summary   Include statistics in success summary\n\
  -h, --help               Print this help text\n";

pub fn print_usage(mut out: impl io::Write) -> io::Result<()> {
    out.write_all(USAGE.as_bytes())
}

pub fn parse_from_env() -> Result<CliInvocation, CliError> {
    let mut args = std::env::args_os();
    // Drop argv[0]
    args.next();
    parse_args(args)
}

fn parse_args(mut args: impl Iterator<Item = OsString>) -> Result<CliInvocation, CliError> {
    let mut config = CliConfig::default();
    let mut patch_arg: Option<String> = None;
    let mut undo_requested = false;
    let mut treat_rest_as_patch = false;

    while let Some(arg) = args.next() {
        let arg_string = arg.into_string().map_err(|_| CliError::InvalidPatchUtf8)?;
        if treat_rest_as_patch {
            if patch_arg.is_some() {
                return Err(CliError::TooManyArguments);
            }
            patch_arg = Some(arg_string);
            continue;
        }

        match arg_string.as_str() {
            "--" => {
                treat_rest_as_patch = true;
            }
            "-h" | "--help" => {
                return Err(CliError::DisplayedHelp);
            }
            "-i" | "--interactive" => {
                config.interactive_mode = InteractiveMode::Enabled;
            }
            "--no-interactive" => {
                config.interactive_mode = InteractiveMode::Disabled;
            }
            "-n" | "--dry-run" => {
                config.dry_run = true;
            }
            "--preview" => {
                config.preview_mode = PreviewMode::Always;
            }
            "--no-preview" => {
                config.preview_mode = PreviewMode::Never;
            }
            "-y" | "--yes" => {
                config.assume_yes = true;
            }
            "-q" | "--quiet" => {
                config.summary_mode = SummaryMode::Quiet;
            }
            "--detailed-summary" => {
                config.summary_mode = SummaryMode::Detailed;
            }
            "--undo-last" => {
                undo_requested = true;
            }
            "--run-after" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::MissingRunAfterValue)?
                    .into_string()
                    .map_err(|_| CliError::InvalidPatchUtf8)?;
                if value.trim().is_empty() {
                    return Err(CliError::MissingRunAfterValue);
                }
                config.run_after.push(value);
            }
            _ => {
                if patch_arg.is_some() {
                    return Err(CliError::TooManyArguments);
                }
                patch_arg = Some(arg_string);
            }
        }
    }

    if undo_requested {
        if patch_arg.is_some() {
            return Err(CliError::FlagConflict(
                "Error: `--undo-last` cannot be combined with a PATCH argument.".to_string(),
            ));
        }
        if !config.run_after.is_empty() {
            return Err(CliError::FlagConflict(
                "Error: `--run-after` is not supported together with `--undo-last`.".to_string(),
            ));
        }
        return Ok(CliInvocation {
            config,
            command: Command::UndoLast,
        });
    }

    let patch = match patch_arg {
        Some(patch) => patch,
        None => read_patch_from_stdin()?,
    };

    Ok(CliInvocation {
        config,
        command: Command::Apply { patch },
    })
}

fn read_patch_from_stdin() -> Result<String, CliError> {
    let mut buf = String::new();
    match io::stdin().read_to_string(&mut buf) {
        Ok(_) => {
            if buf.is_empty() {
                Err(CliError::MissingPatch)
            } else {
                Ok(buf)
            }
        }
        Err(err) => Err(CliError::FailedToReadPatch(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[&str]) -> Result<CliInvocation, CliError> {
        parse_args(input.iter().map(|s| OsString::from(s)))
    }

    #[test]
    fn parse_defaults_with_arg_patch() {
        let patch = "*** Begin Patch\n*** End Patch\n";
        let result = parse(&[patch]).unwrap();
        assert!(matches!(
            result.command,
            Command::Apply { patch: ref body } if body == patch
        ));
        assert_eq!(result.config, CliConfig::default());
    }

    #[test]
    fn parse_help() {
        let err = parse(&["--help"]).expect_err("should error");
        assert!(matches!(err, CliError::DisplayedHelp));
    }

    #[test]
    fn parse_interactive_flags() {
        let patch = "*** Begin Patch\n*** End Patch\n";
        let result = parse(&["--interactive", patch]).unwrap();
        assert_eq!(result.config.interactive_mode, InteractiveMode::Enabled);
    }

    #[test]
    fn reject_multiple_patches() {
        let err = parse(&["patch1", "patch2"]).expect_err("should fail");
        assert!(matches!(err, CliError::TooManyArguments));
    }

    #[test]
    fn parse_undo_last() {
        let invocation = parse(&["--undo-last"]).unwrap();
        assert!(matches!(invocation.command, Command::UndoLast));
    }

    #[test]
    fn reject_undo_with_patch() {
        let err = parse(&["--undo-last", "foo"]).expect_err("should fail");
        assert!(matches!(err, CliError::FlagConflict(_)));
    }

    #[test]
    fn run_after_requires_value() {
        let err = parse(&["--run-after"]).expect_err("should fail");
        assert!(matches!(err, CliError::MissingRunAfterValue));
    }
}
