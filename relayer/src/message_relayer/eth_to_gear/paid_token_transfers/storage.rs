use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    str::FromStr,
};

use super::tx_manager::{Transaction, TransactionManager};
use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn save(&self, tx_manager: &TransactionManager) -> anyhow::Result<()>;
    async fn load(&self, tx_manager: &TransactionManager) -> anyhow::Result<()>;
}

pub struct NoStorage;

#[async_trait]
impl Storage for NoStorage {
    async fn save(&self, _tx_manager: &TransactionManager) -> anyhow::Result<()> {
        /* no-op */
        Ok(())
    }

    async fn load(&self, _tx_manager: &TransactionManager) -> anyhow::Result<()> {
        /* no-op */
        Ok(())
    }
}

/// Simple storage for transactions which keeps them in a JSON file under
/// specified directory.
pub struct JSONStorage {
    path: PathBuf,
}

impl JSONStorage {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    async fn write_tx(&self, tx_uuid: &Uuid, tx: &Transaction) -> anyhow::Result<()> {
        let filename = self.path.join(tx_uuid.to_string());
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(filename)
            .await?;

        let json = serde_json::to_string(&tx)?;
        file.write_all(json.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    async fn read_tx(&self, path: PathBuf, tx_file: OsString) -> anyhow::Result<Transaction> {
        let uuid = tx_file
            .to_str()
            .and_then(|s| Uuid::from_str(s).ok())
            .ok_or_else(|| anyhow::anyhow!("Invalid UUID in filename {tx_file:?}"))?;
        let mut contents = String::new();

        let mut file = tokio::fs::File::open(path).await?;
        file.read_to_string(&mut contents).await?;

        let tx: Transaction = serde_json::from_str(&contents)?;

        if tx.uuid != uuid {
            return Err(anyhow::anyhow!(
                "UUID in filename does not match transaction UUID: {} != {}",
                uuid,
                tx.uuid
            ));
        }

        Ok(tx)
    }
}

#[async_trait]
impl Storage for JSONStorage {
    async fn save(&self, tx_manager: &TransactionManager) -> anyhow::Result<()> {
        if !self.path.exists() {
            tokio::fs::create_dir_all(&self.path).await?;
        }

        for (tx_uuid, tx) in tx_manager.transactions.read().await.iter() {
            self.write_tx(tx_uuid, tx).await?;
        }

        Ok(())
    }

    async fn load(&self, tx_manager: &TransactionManager) -> anyhow::Result<()> {
        if !self.path.exists() {
            return Ok(());
        }

        let mut dir = tokio::fs::read_dir(&self.path).await?;

        while let Some(entry) = dir.next_entry().await? {
            if entry.file_type().await?.is_file() {
                let tx = self.read_tx(entry.path(), entry.file_name()).await?;

                tx_manager.add_transaction(tx).await;
            }
        }

        Ok(())
    }
}
