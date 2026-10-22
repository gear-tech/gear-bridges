//! Committed evidence reads and immutable JSON writes shared by signer ownership and cutover.
use anyhow::{Context, Result};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub fn entry_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
pub fn committed_file(path: &Path) -> Result<fs::Metadata> {
    for pending in [
        PathBuf::from(format!("{}.new", path.display())),
        PathBuf::from(format!("{}.tmp", path.display())),
        path.with_extension("tmp"),
    ] {
        anyhow::ensure!(
            !entry_exists(&pending)?,
            "HOLD: pending journal evidence {}",
            pending.display()
        );
    }
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.file_type().is_file(),
        "HOLD: seal input must be a regular committed file"
    );
    Ok(metadata)
}
pub fn committed_bytes(path: &Path) -> Result<Vec<u8>> {
    committed_file(path)?;
    fs::read(path).with_context(|| format!("HOLD: missing original evidence {}", path.display()))
}
pub fn immutable_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let bytes = serde_json::to_vec(value)?;
    let temp = PathBuf::from(format!("{}.new", path.display()));
    if path.try_exists()? {
        anyhow::ensure!(
            fs::symlink_metadata(path)?.file_type().is_file() && fs::read(path)? == bytes,
            "HOLD: immutable cutover evidence changed"
        );
        if temp.try_exists()? {
            anyhow::ensure!(
                fs::symlink_metadata(&temp)?.file_type().is_file() && fs::read(&temp)? == bytes,
                "HOLD: interrupted cutover evidence has a different intent"
            );
            fs::remove_file(&temp)?;
            File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()?;
        }
        return Ok(());
    }
    if temp.try_exists()? {
        anyhow::ensure!(
            fs::symlink_metadata(&temp)?.file_type().is_file() && fs::read(&temp)? == bytes,
            "HOLD: interrupted cutover evidence has a different intent"
        );
        File::open(&temp)?.sync_all()?;
    } else {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    // hard_link never overwrites another intent; an exact interrupted link is resumable.
    fs::hard_link(&temp, path)?;
    fs::remove_file(&temp)?;
    File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()?;
    Ok(())
}
