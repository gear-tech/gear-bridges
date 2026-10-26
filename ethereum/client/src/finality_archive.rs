use alloy::{consensus::Header, primitives::B256};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

const CHUNK_HEADERS: usize = 512;

/// Files are untrusted proof material, never a persisted finality verdict.
/// A chunk is reusable only after every canonical RLP hash and parent link is checked.
/// Ownership is per canonical directory, not per chain: independent journals can
/// retain separate archives for one genesis. Only the authority explicitly owning
/// a directory authenticates and advances its durable fence.
pub(crate) struct HeaderArchive {
    directory: PathBuf,
    chunks: Mutex<BTreeMap<u64, Vec<PathBuf>>>,
    authority: Mutex<std::sync::Weak<crate::beacon_finality::BeaconAuthority>>,
    _owner: File,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fence {
    genesis: B256,
    tips: Vec<Header>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quorum_checkpoint: Option<crate::FinalityCheckpoint>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedRecord {
    previous: Option<B256>,
    checkpoint: crate::FinalityCheckpoint,
}

/// Atomically replace a derived cache; never use for immutable originals or journals.
pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let directory = path.parent().context("proof path has no parent")?;
    fs::create_dir_all(directory)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = std::io::BufWriter::new(File::create(&temporary)?);
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.get_ref().sync_all()?;
    fs::rename(temporary, path)?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

impl HeaderArchive {
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn claim(
        &self,
        authority: &std::sync::Arc<crate::beacon_finality::BeaconAuthority>,
    ) -> Result<()> {
        let mut owner = self
            .authority
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = owner.upgrade() {
            ensure!(
                std::sync::Arc::ptr_eq(&previous, authority),
                "another Beacon authority owns this writable header archive; HOLD"
            );
        } else {
            *owner = std::sync::Arc::downgrade(authority);
        }
        Ok(())
    }

    pub(crate) fn open(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let owner = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("owner.lock"))?;
        owner
            .try_lock()
            .context("another process owns this finalized proof archive; HOLD")?;
        let mut chunks = BTreeMap::<u64, Vec<PathBuf>>::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.ends_with(".json") {
                continue;
            }
            let Some(top) = name
                .strip_prefix("headers-")
                .and_then(|name| name.split('-').next())
                .and_then(|number| number.parse::<u64>().ok())
            else {
                continue;
            };
            chunks.entry(top).or_default().push(entry.path());
        }
        Ok(Self {
            directory: directory.to_owned(),
            chunks: Mutex::new(chunks),
            authority: Default::default(),
            _owner: owner,
        })
    }

    /// The pointer is only a cache. Reconstruct from the entire immutable chain;
    /// missing links, forks and changed bytes never become a new bootstrap.
    fn verified_records(&self) -> Result<Vec<(B256, VerifiedRecord)>> {
        let directory = self.directory.join("verified");
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut records = BTreeMap::new();
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("invalid verified record filename")?;
            ensure!(
                !name.ends_with(".new"),
                "pending immutable finality evidence; HOLD"
            );
            let digest: B256 = name
                .strip_suffix(".json")
                .context("unknown verified finality record")?
                .parse()?;
            let bytes = gear_common::durable::committed_bytes(&entry.path())?;
            let record: VerifiedRecord = serde_json::from_slice(&bytes)?;
            let canonical = serde_json::to_vec(&record)?;
            ensure!(
                B256::from(sp_core::hashing::sha2_256(&canonical)) == digest,
                "changed immutable finality record; HOLD"
            );
            ensure!(
                record.checkpoint.beacon().is_some(),
                "non-Beacon verified record; HOLD"
            );
            ensure!(
                records.insert(record.previous, (digest, record)).is_none(),
                "forked immutable finality chain; HOLD"
            );
        }
        let mut ordered = Vec::with_capacity(records.len());
        let mut previous = None;
        while let Some((digest, record)) = records.remove(&previous) {
            previous = Some(digest);
            ordered.push((digest, record));
        }
        ensure!(
            records.is_empty(),
            "missing immutable finality predecessor; HOLD"
        );
        Ok(ordered)
    }

    fn legacy_fence(&self) -> Result<Option<Fence>> {
        let path = self.directory.join("fence.json");
        if !path.exists() {
            return Ok(None);
        }
        ensure!(
            fs::metadata(&path)?.len() <= 64 * 1024 * 1024,
            "oversized legacy finalized generation fence; HOLD"
        );
        Ok(Some(serde_json::from_slice(&fs::read(path)?)?))
    }

    pub(crate) fn retained_checkpoints(&self) -> Result<Vec<crate::FinalityCheckpoint>> {
        let mut checkpoints = Vec::new();
        if let Some(fence) = self.legacy_fence()? {
            checkpoints.extend(fence.quorum_checkpoint);
        }
        checkpoints.extend(
            self.verified_records()?
                .into_iter()
                .map(|(_, record)| record.checkpoint),
        );
        Ok(checkpoints)
    }

    /// Read existing obligations only. An unqualified observation cannot grow
    /// durable authority; legacy fences remain byte-for-byte immutable.
    pub(crate) fn witness(
        &self,
        genesis: B256,
        head: &Header,
        approved: Option<&crate::FinalityCheckpoint>,
    ) -> Result<Vec<(u64, B256)>> {
        let fence = self.legacy_fence()?.unwrap_or(Fence {
            genesis,
            tips: Vec::new(),
            quorum_checkpoint: None,
        });
        ensure!(
            fence.genesis == genesis,
            "finalized archive belongs to another chain; HOLD"
        );
        let hash = head.hash_slow();
        if let Some(checkpoint) = &fence.quorum_checkpoint {
            ensure!(
                checkpoint.genesis_hash() == genesis
                    && approved.is_some_and(|fresh| fresh.chain_id() == checkpoint.chain_id())
                    && fence
                        .tips
                        .iter()
                        .any(|tip| tip == checkpoint.common_header()),
                "invalid retained finality checkpoint; HOLD"
            );
            for (number, expected) in checkpoint.fences()? {
                ensure!(
                    number <= head.number,
                    "Beacon head has not reached every retained historical fence; HOLD"
                );
                ensure!(
                    number != head.number || expected == hash,
                    "retained checkpoint conflicts with Beacon head; HOLD"
                );
            }
        }
        let mut required = Vec::with_capacity(fence.tips.len());
        if let Some(checkpoint) = &fence.quorum_checkpoint {
            required.extend(
                checkpoint
                    .fences()?
                    .into_iter()
                    .filter(|(_, expected)| *expected != hash),
            );
        }
        for tip in &fence.tips {
            ensure!(
                tip.number <= head.number,
                "finalized head regressed below a durable witnessed tip; HOLD"
            );
            let old_hash = tip.hash_slow();
            ensure!(
                tip.number != head.number || old_hash == hash,
                "conflicting durable finalized generation; HOLD"
            );
            if old_hash != hash {
                required.push((tip.number, old_hash));
            }
        }
        for (_, record) in self.verified_records()? {
            ensure!(
                record.checkpoint.genesis_hash() == genesis,
                "verified finality record belongs to another chain; HOLD"
            );
            for (number, expected) in record.checkpoint.fences()? {
                ensure!(
                    number <= head.number && (number != head.number || expected == hash),
                    "verified finality record conflicts with candidate; HOLD"
                );
                if expected != hash {
                    required.push((number, expected));
                }
            }
        }
        Ok(required)
    }

    pub(crate) fn complete(&self, checkpoint: &crate::FinalityCheckpoint) -> Result<()> {
        ensure!(
            checkpoint.beacon().is_some(),
            "only Beacon checkpoints may complete an archive fence; HOLD"
        );
        let records = self.verified_records()?;
        fs::create_dir_all(self.directory.join("verified"))?;
        // Persist the directory entry before publishing any authoritative record.
        File::open(&self.directory)?.sync_all()?;
        let previous = records.last().map(|(digest, _)| *digest);
        let digest = if records
            .last()
            .is_some_and(|(_, record)| &record.checkpoint == checkpoint)
        {
            previous.unwrap()
        } else {
            let record = VerifiedRecord {
                previous,
                checkpoint: checkpoint.clone(),
            };
            let digest = B256::from(sp_core::hashing::sha2_256(&serde_json::to_vec(&record)?));
            gear_common::durable::immutable_json(
                &self
                    .directory
                    .join("verified")
                    .join(format!("{digest:#x}.json")),
                &record,
            )?;
            digest
        };
        // No authority is read from this pointer. A damaged/missing pointer is
        // repaired only here, after all retained records and pins requalify.
        atomic_json(&self.directory.join("verified-head.json"), &digest)
    }

    /// Read at most one bounded chunk. Corrupt header material is ignored as a
    /// hint and reconstructed by hash; its old file is preserved for diagnosis.
    pub(crate) fn read(&self, number: u64, expected: B256) -> Option<Vec<Header>> {
        let chunks = self
            .chunks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (_, paths) in chunks.range(number..number.saturating_add(CHUNK_HEADERS as u64)) {
            for path in paths {
                if fs::metadata(path).map_or(true, |metadata| metadata.len() > 2 * 1024 * 1024) {
                    continue;
                }
                let Ok(bytes) = fs::read(path) else { continue };
                let Ok(headers) = serde_json::from_slice::<Vec<Header>>(&bytes) else {
                    continue;
                };
                if headers.is_empty() || headers.len() > CHUNK_HEADERS {
                    continue;
                }
                if headers.windows(2).any(|pair| {
                    pair[0].number.checked_sub(1) != Some(pair[1].number)
                        || pair[0].parent_hash != pair[1].hash_slow()
                }) {
                    continue;
                }
                if let Some(index) = headers
                    .iter()
                    .position(|header| header.number == number && header.hash_slow() == expected)
                {
                    return Some(headers.into_iter().skip(index).collect());
                }
            }
        }
        None
    }

    pub(crate) fn save(&self, headers: &[Header]) -> Result<()> {
        if headers.is_empty() {
            return Ok(());
        }
        ensure!(
            headers.len() <= CHUNK_HEADERS,
            "oversized finalized proof chunk"
        );
        ensure!(
            headers
                .windows(2)
                .all(|pair| pair[0].number.checked_sub(1) == Some(pair[1].number)
                    && pair[0].parent_hash == pair[1].hash_slow()),
            "invalid finalized proof chunk"
        );
        let first = &headers[0];
        let name = format!(
            "headers-{}-{:x}-{}.json",
            first.number,
            first.hash_slow(),
            headers.last().unwrap().number
        );
        let mut path = self.directory.join(name);
        if path.exists() {
            if fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Vec<Header>>(&bytes).ok())
                .as_deref()
                == Some(headers)
            {
                return Ok(());
            }
            // Preserve damaged material; never erase evidence to make a proof pass.
            path = self.directory.join(format!(
                "headers-{}-{:x}-{}-{}.json",
                first.number,
                first.hash_slow(),
                headers.last().unwrap().number,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
        }
        atomic_json(&path, &headers)?;
        self.chunks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(first.number)
            .or_default()
            .push(path);
        Ok(())
    }
}
