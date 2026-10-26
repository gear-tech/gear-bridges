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
fn signer_directory() -> Result<PathBuf> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let uid = unsafe { libc::geteuid() };
    let directory = PathBuf::from(
        std::env::var_os("VARA_BRIDGE_SIGNER_LOCK_DIR")
            .context("HOLD: provision VARA_BRIDGE_SIGNER_LOCK_DIR on the shared signing host")?,
    );
    anyhow::ensure!(
        directory.is_absolute(),
        "HOLD: signer namespace must be absolute"
    );
    anyhow::ensure!(
        fs::canonicalize(&directory)? == directory,
        "HOLD: signer namespace must be canonical and not contain symlinks"
    );
    let metadata = fs::symlink_metadata(&directory)?;
    anyhow::ensure!(
        metadata.is_dir()
            && metadata.permissions().mode() & 0o777 == 0o700
            && metadata.uid() == uid,
        "HOLD: signer lock directory must be private and not a symlink"
    );
    Ok(directory)
}

fn signer_lock_path(genesis: H256, sender: [u8; 32]) -> Result<PathBuf> {
    anyhow::ensure!(
        genesis != H256::zero() && sender != [0; 32],
        "HOLD: missing signer chain/account identity"
    );
    Ok(signer_directory()?.join(format!(
        "{}-{}.lock",
        hex::encode(genesis.0),
        hex::encode(sender)
    )))
}
fn acquire_signer(genesis: H256, sender: [u8; 32]) -> Result<(File, PathBuf)> {
    let path = signer_lock_path(genesis, sender)?;
    Ok((acquire_lock(&path)?, path.with_extension("cutover-owner")))
}

fn acquire_lock(path: &Path) -> Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    anyhow::ensure!(
        file.metadata()?.is_file()
            && file.metadata()?.permissions().mode() & 0o777 == 0o600
            && file.metadata()?.uid() == unsafe { libc::geteuid() },
        "HOLD: signer lock must be a private regular file"
    );
    file.try_lock()
        .context("HOLD: another signing owner holds this chain/account")?;
    let current = fs::symlink_metadata(path)?;
    let held = file.metadata()?;
    anyhow::ensure!(
        current.is_file() && current.dev() == held.dev() && current.ino() == held.ino(),
        "HOLD: signer lock inode was replaced"
    );
    Ok(file)
}
/// Normal runtimes never take over a lifecycle original merely because its process stopped.
pub fn lock_signer(genesis: H256, sender: [u8; 32]) -> Result<File> {
    let (file, owner) = acquire_signer(genesis, sender)?;
    anyhow::ensure!(!entry_exists(&owner)? && !entry_exists(&PathBuf::from(format!("{}.new",owner.display())))?,
        "HOLD: this account has durable cutover original ownership; resume its cutover-work.json before restarting a relayer");
    Ok(file)
}

/// Same namespace for every Ethereum role and executable, never keyed by journal/HOME.
pub fn lock_ethereum_signer(chain_id: u64, genesis: H256, sender: [u8; 20]) -> Result<File> {
    acquire_lock(&ethereum_signer_path(chain_id, genesis, sender)?)
}

/// Spending evidence shares the mandatory chain/account ownership namespace.
pub fn ethereum_spend_ledger_path(
    chain_id: u64,
    genesis: H256,
    sender: [u8; 20],
) -> Result<PathBuf> {
    Ok(ethereum_signer_path(chain_id, genesis, sender)?.with_extension("spend.json"))
}

fn ethereum_signer_path(chain_id: u64, genesis: H256, sender: [u8; 20]) -> Result<PathBuf> {
    anyhow::ensure!(
        chain_id != 0 && genesis != H256::zero() && sender != [0; 20],
        "HOLD: missing Ethereum signer chain/account identity"
    );
    Ok(signer_directory()?.join(format!(
        "ethereum-{chain_id}-{}-{}.lock",
        hex::encode(genesis.0),
        hex::encode(sender)
    )))
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

/// Development identities are checked by derived account, not URI spelling.
pub fn validate_suri(suri: &str, endpoint: Option<&str>) -> Result<()> {
    let owned = std::env::var_os("GEAR_BRIDGE_OWNED_FIXTURE").is_some_and(|value| value == "1");
    validate_suri_for(suri, endpoint, owned)
}

fn validate_suri_for(suri: &str, endpoint: Option<&str>, owned: bool) -> Result<()> {
    use sp_core::{sr25519, Pair};
    anyhow::ensure!(!suri.trim().is_empty(), "HOLD: Gear signer URI is required");
    let pair = sr25519::Pair::from_string(suri, None)
        .map_err(|_| anyhow::anyhow!("HOLD: invalid Gear signer URI"))?;
    static DEVELOPMENT: std::sync::LazyLock<[sr25519::Public; 17]> =
        std::sync::LazyLock::new(|| {
            std::array::from_fn(|index| {
                let uri = if index == 0 {
                    sp_core::crypto::DEV_PHRASE.to_owned()
                } else {
                    let names = [
                        "Alice", "Bob", "Charlie", "Dave", "Eve", "Ferdie", "One", "Two",
                    ];
                    let suffix = if index % 2 == 0 { "//stash" } else { "" };
                    format!("//{}{suffix}", names[(index - 1) / 2])
                };
                sr25519::Pair::from_string(&uri, None)
                    .expect("built-in development identity")
                    .public()
            })
        });
    let development = DEVELOPMENT.contains(&pair.public());
    let loopback = endpoint
        .and_then(|endpoint| url::Url::parse(endpoint).ok())
        .and_then(|url| url.host_str().map(str::to_owned))
        .and_then(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .ok()
        })
        .is_some_and(|host| host.is_loopback());
    anyhow::ensure!(
        !development || (owned && loopback),
        "HOLD: development signer requires explicit owned numeric-loopback fixture"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reconstructed from supplied HTML / audited7fe1b032, not original PoC artifacts.
    #[test]
    fn supplied_o407_equivalent_development_signers_require_owned_loopback() {
        use sp_core::{sr25519, Pair};
        let (_, seed) = sr25519::Pair::from_string_with_seed("//Alice", None).unwrap();
        let equivalent = format!("0x{}", hex::encode(seed.unwrap()));
        for suri in [
            "//Alice".to_owned(),
            format!("{}//Alice", sp_core::crypto::DEV_PHRASE),
            equivalent,
        ] {
            assert!(validate_suri_for(&suri, Some("ws://127.0.0.1:9944"), false).is_err());
            assert!(validate_suri_for(&suri, Some("wss://vara.example"), true).is_err());
            assert!(validate_suri_for(&suri, Some("ws://localhost:9944"), true).is_err());
            assert!(validate_suri_for(&suri, None, true).is_err());
            validate_suri_for(&suri, Some("ws://127.0.0.1:9944"), true).unwrap();
            validate_suri_for(&suri, Some("ws://[::1]:9944"), true).unwrap();
        }
        assert!(validate_suri_for("", None, false).is_err());
    }

    #[test]
    fn supplied_o408_two_home_values_share_one_lock() {
        use std::{
            io::{BufRead, Write},
            os::unix::fs::PermissionsExt,
            process::{Command, Stdio},
        };
        const CHILD: &str = "GEAR_SIGNER_LOCK_TEST_CHILD";
        let genesis = H256::repeat_byte(201);
        let sender = [202; 32];
        if let Ok(mode) = std::env::var(CHILD) {
            if mode == "hold" {
                let _owner = lock_signer(genesis, sender).unwrap();
                let _ethereum = lock_ethereum_signer(31337, genesis, [203; 20]).unwrap();
                println!("LOCK_READY");
                std::io::stdout().flush().unwrap();
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).unwrap();
            } else {
                assert!(lock_signer(genesis, sender).is_err());
                assert!(lock_ethereum_signer(31337, genesis, [203; 20]).is_err());
            }
            return;
        }
        let directory = std::env::temp_dir().join(format!(
            "gear-signers-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let directory = fs::canonicalize(directory).unwrap();
        let command = |home: &str, mode: &str| {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "signer_ownership::tests::supplied_o408_two_home_values_share_one_lock",
                    "--nocapture",
                ])
                .env(CHILD, mode)
                .env("HOME", home)
                .env("VARA_BRIDGE_SIGNER_LOCK_DIR", &directory);
            command
        };
        let mut first = command("/different-home-a", "hold")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut reader = std::io::BufReader::new(first.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert!(
                reader.read_line(&mut line).unwrap() > 0,
                "lock owner exited before readiness"
            );
            if line.trim() == "LOCK_READY" {
                break;
            }
        }
        let second = command("/different-home-b", "contend").status().unwrap();
        first.stdin.take().unwrap().write_all(b"release\n").unwrap();
        let first = first.wait().unwrap();
        fs::remove_dir_all(directory).unwrap();
        assert!(first.success() && second.success());
    }
}
