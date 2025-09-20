use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ApplyPatchAction;
use crate::ApplyPatchFileChange;

#[derive(Debug, Error)]
pub enum UndoError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("Unable to serialize undo record: {0}")]
    Serialize(serde_json::Error),
    #[allow(dead_code)]
    #[error("Undo history is empty.")]
    Empty,
    #[error("Undo record verification failed: {0}")]
    Verification(String),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UndoRecord {
    pub created_epoch_secs: u64,
    pub entries: Vec<UndoEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UndoEntry {
    Added {
        path: PathBuf,
        content: String,
    },
    Deleted {
        path: PathBuf,
        content: String,
    },
    Updated {
        original_path: PathBuf,
        moved_path: Option<PathBuf>,
        original_content: String,
        new_content: String,
    },
}

impl UndoRecord {
    pub fn build(action: &ApplyPatchAction) -> Self {
        let mut entries = Vec::new();
        for hunk in &action.hunks {
            let path = hunk.resolve_path(&action.cwd);
            let change = action
                .changes()
                .get(&path)
                .expect("change metadata should exist for every hunk");
            match change {
                ApplyPatchFileChange::Add { content } => {
                    entries.push(UndoEntry::Added {
                        path: path.clone(),
                        content: content.clone(),
                    });
                }
                ApplyPatchFileChange::Delete { content } => {
                    entries.push(UndoEntry::Deleted {
                        path: path.clone(),
                        content: content.clone(),
                    });
                }
                ApplyPatchFileChange::Update {
                    move_path,
                    original_content,
                    new_content,
                    ..
                } => {
                    entries.push(UndoEntry::Updated {
                        original_path: path.clone(),
                        moved_path: move_path.clone(),
                        original_content: original_content.clone(),
                        new_content: new_content.clone(),
                    });
                }
            }
        }
        Self {
            created_epoch_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            entries,
        }
    }
}

pub fn store_last_record(cwd: &Path, record: &UndoRecord) -> Result<(), UndoError> {
    let path = history_path(cwd);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec_pretty(record).map_err(UndoError::Serialize)?;
    fs::write(path, data)?;
    Ok(())
}

pub fn load_last_record(cwd: &Path) -> Result<Option<UndoRecord>, UndoError> {
    let path = history_path(cwd);
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read(path)?;
    let record: UndoRecord = serde_json::from_slice(&data).map_err(UndoError::Serialize)?;
    Ok(Some(record))
}

pub fn clear_history(cwd: &Path) -> Result<(), UndoError> {
    let path = history_path(cwd);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn history_path(cwd: &Path) -> PathBuf {
    cwd.join(".codex").join("apply_patch_history.json")
}

pub fn apply_undo(record: &UndoRecord) -> Result<(), UndoError> {
    for entry in record.entries.iter().rev() {
        match entry {
            UndoEntry::Added { path, content } => {
                verify_file_contents(path, Some(content))?;
                if path.exists() {
                    fs::remove_file(path)?;
                }
            }
            UndoEntry::Deleted { path, content } => {
                verify_file_contents(path, None)?;
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() {
                        fs::create_dir_all(parent)?;
                    }
                }
                fs::write(path, content)?;
            }
            UndoEntry::Updated {
                original_path,
                moved_path,
                original_content,
                new_content,
            } => {
                let current_path = moved_path.as_ref().unwrap_or(original_path);
                verify_file_contents(current_path, Some(new_content))?;
                if let Some(parent) = original_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        fs::create_dir_all(parent)?;
                    }
                }
                fs::write(original_path, original_content)?;
                if let Some(moved) = moved_path {
                    if moved != original_path {
                        if moved.exists() {
                            fs::remove_file(moved)?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn verify_file_contents(path: &Path, expected: Option<&String>) -> Result<(), UndoError> {
    match expected {
        Some(expected) => {
            if !path.exists() {
                return Err(UndoError::Verification(format!(
                    "Expected file {} to exist but it is missing.",
                    path.display()
                )));
            }
            let actual = fs::read_to_string(path)?;
            if &actual != expected {
                return Err(UndoError::Verification(format!(
                    "File {} differs from recorded state; aborting.",
                    path.display()
                )));
            }
            Ok(())
        }
        None => {
            if path.exists() {
                Err(UndoError::Verification(format!(
                    "Expected file {} to be absent; aborting undo.",
                    path.display()
                )))
            } else {
                Ok(())
            }
        }
    }
}
