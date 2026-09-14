// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{checked_path, storage_error, StorageDescriptor};
use std::path::{Path, PathBuf};

pub(super) fn verify_protection(root: &Path) -> Result<(), sqlx::Error> {
    if root.join("vault.meta").exists() || root.join(".vault_locked").exists() {
        return Err(storage_error(
            "hybrid storage requires a root without vault protection",
        ));
    }
    Ok(())
}

/// On-disk inventory includes pending generations and scratch files, so both
/// storage accounting and offline operators see the complete storage footprint.
pub fn inventory(root: &Path) -> Result<Vec<PathBuf>, sqlx::Error> {
    let mut files = Vec::new();
    if let Some(descriptor) = StorageDescriptor::read(root)? {
        files.push(checked_path(root, &descriptor.index)?);
        for suffix in ["-wal", "-shm"] {
            let path = root.join(format!("{}{suffix}", descriptor.index.display()));
            if path.is_file() {
                files.push(path);
            }
        }
    }
    collect_files(&root.join("storage"), &mut files)?;
    for name in [
        "storage.json",
        "storage-init.json",
        "storage-migration.json",
        "storage-migration-complete.json",
        "storage-maintenance.json",
        "storage-maintenance-complete.json",
    ] {
        let path = root.join(name);
        if path.is_file() {
            files.push(path);
        }
    }
    for name in ["db.sqlite", "db.sqlite-wal", "db.sqlite-shm"] {
        let path = root.join(name);
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), sqlx::Error> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(storage_error("storage inventory contains a symlink"));
        }
        if kind.is_dir() {
            collect_files(&entry.path(), files)?;
        } else if kind.is_file() {
            files.push(entry.path());
        }
    }
    Ok(())
}

pub fn artifact_bytes(path: &Path) -> Result<u64, sqlx::Error> {
    if path.is_file() {
        return Ok(std::fs::metadata(path)?.len());
    }
    let mut files = Vec::new();
    collect_files(path, &mut files)?;
    files.iter().try_fold(0_u64, |sum, path| {
        Ok(sum.saturating_add(std::fs::metadata(path)?.len()))
    })
}
