//! Host-wide Gear account fencing, including durable ownership of cutover originals.
use crate::durable::{committed_bytes, committed_file, entry_exists, immutable_json};
use anyhow::{Context, Result};
use primitive_types::H256;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CutoverSignerOwner {
    schema_version: u32,
    gear_genesis_hash: H256,
    sender: [u8; 32],
    work_path: PathBuf,
}
fn signer_lock_path(genesis: H256, sender: [u8; 32]) -> Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    anyhow::ensure!(
        genesis != H256::zero() && sender != [0; 32],
        "HOLD: missing signer chain/account identity"
    );
    let uid = unsafe { libc::geteuid() };
    let home = std::env::var_os("HOME").context("HOLD: missing persistent signer state home")?;
    let directory = fs::canonicalize(home)?.join(".local/state/vara-bridge-signers");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    let metadata = fs::symlink_metadata(&directory)?;
    anyhow::ensure!(
        metadata.is_dir()
            && metadata.permissions().mode() & 0o777 == 0o700
            && metadata.uid() == uid,
        "HOLD: signer lock directory must be private and not a symlink"
    );
    Ok(directory.join(format!(
        "{}-{}.lock",
        hex::encode(genesis.0),
        hex::encode(sender)
    )))
}
fn acquire_signer(genesis: H256, sender: [u8; 32]) -> Result<(File, PathBuf)> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let path = signer_lock_path(genesis, sender)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?;
    anyhow::ensure!(
        file.metadata()?.is_file()
            && file.metadata()?.permissions().mode() & 0o777 == 0o600
            && file.metadata()?.uid() == unsafe { libc::geteuid() },
        "HOLD: signer lock must be a private regular file"
    );
    file.try_lock().context("HOLD: another original lane/lifecycle signer owns this Gear account; drain and stop it before cutover")?;
    Ok((file, path.with_extension("cutover-owner")))
}
/// Normal runtimes never take over a lifecycle original merely because its process stopped.
pub fn lock_signer(genesis: H256, sender: [u8; 32]) -> Result<File> {
    let (file, owner) = acquire_signer(genesis, sender)?;
    anyhow::ensure!(!entry_exists(&owner)? && !entry_exists(&PathBuf::from(format!("{}.new",owner.display())))?,
        "HOLD: this account has durable cutover original ownership; resume its cutover-work.json before restarting a relayer");
    Ok(file)
}
fn signer_work_path(path: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        path.file_name()
            .is_some_and(|name| name == "cutover-work.json"),
        "HOLD: signer ownership requires cutover-work.json"
    );
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let path = fs::canonicalize(parent)?.join("cutover-work.json");
    // Missing original work cannot be interpreted as an unsigned first attempt.
    anyhow::ensure!(
        committed_file(&path)?.len() > 0,
        "HOLD: cutover work intent is missing or empty"
    );
    Ok(path)
}
pub fn lock_cutover_signer(genesis: H256, sender: [u8; 32], work_path: &Path) -> Result<File> {
    let work_path = signer_work_path(work_path)?;
    let (file, owner) = acquire_signer(genesis, sender)?;
    immutable_json(
        &owner,
        &CutoverSignerOwner {
            schema_version: 1,
            gear_genesis_hash: genesis,
            sender,
            work_path,
        },
    )?;
    Ok(file)
}
/// Caller must first authenticate every owned original as terminal and prove the lifecycle safe to resume.
/// Keep the supplied OS signer guard held until after this durable release; never unlink its lock inode.
pub fn release_cutover_signer(
    genesis: H256,
    sender: [u8; 32],
    work_path: &Path,
    ownership: &File,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    ownership
        .try_lock()
        .context("HOLD: cutover account has another signing owner")?;
    let path = signer_lock_path(genesis, sender)?;
    let held = ownership.metadata()?;
    let current = fs::symlink_metadata(&path)?;
    anyhow::ensure!(
        current.is_file() && held.dev() == current.dev() && held.ino() == current.ino(),
        "HOLD: cutover release does not hold the original account lock inode"
    );
    let owner = path.with_extension("cutover-owner");
    let actual: CutoverSignerOwner = serde_json::from_slice(&committed_bytes(&owner)?)?;
    anyhow::ensure!(
        actual
            == CutoverSignerOwner {
                schema_version: 1,
                gear_genesis_hash: genesis,
                sender,
                work_path: signer_work_path(work_path)?
            },
        "HOLD: cutover signer owner/account/work path changed"
    );
    fs::remove_file(&owner)?;
    File::open(
        owner
            .parent()
            .context("HOLD: signer owner has no directory")?,
    )?
    .sync_all()?;
    Ok(())
}
