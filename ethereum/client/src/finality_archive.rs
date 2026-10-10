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
pub(crate) struct HeaderArchive {
    directory: PathBuf,
    chunks: Mutex<BTreeMap<u64, Vec<PathBuf>>>,
    _owner: File,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fence {
    genesis: B256,
    tips: Vec<Header>,
}

pub(crate) fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
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
            _owner: owner,
        })
    }

    /// Retain every unresolved witnessed generation. A restart must bridge these
    /// exact tips, not merely reach an old numeric height on a different branch.
    pub(crate) fn witness(&self, genesis: B256, head: &Header) -> Result<Vec<(u64, B256)>> {
        let path = self.directory.join("fence.json");
        let mut fence = if path.exists() {
            ensure!(
                fs::metadata(&path)?.len() <= 64 * 1024 * 1024,
                "oversized finalized generation fence; HOLD"
            );
            let fence: Fence = serde_json::from_slice(&fs::read(&path)?)
                .context("corrupt finalized generation fence; HOLD")?;
            ensure!(
                fence.genesis == genesis && !fence.tips.is_empty(),
                "finalized archive belongs to another chain; HOLD"
            );
            fence
        } else {
            Fence {
                genesis,
                tips: Vec::new(),
            }
        };
        let hash = head.hash_slow();
        let mut required = Vec::with_capacity(fence.tips.len());
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
        if fence.tips.last().is_none_or(|tip| tip.hash_slow() != hash) {
            ensure!(
                fence.tips.len() < 32_768,
                "unfinished finalized history exceeds the retained generation bound; HOLD"
            );
            fence.tips.push(head.clone());
            atomic_json(&path, &fence)?;
        }
        Ok(required)
    }

    pub(crate) fn complete(&self, genesis: B256, head: &Header) -> Result<()> {
        atomic_json(
            &self.directory.join("fence.json"),
            &Fence {
                genesis,
                tips: vec![head.clone()],
            },
        )
    }

    /// Read at most one bounded chunk. Corrupt header material is ignored as a
    /// hint and reconstructed by hash; its old file is preserved for diagnosis.
    pub(crate) fn read(&self, number: u64, expected: B256) -> Result<Option<Vec<Header>>> {
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
                    return Ok(Some(headers.into_iter().skip(index).collect()));
                }
            }
        }
        Ok(None)
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
