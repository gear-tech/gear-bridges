use std::{collections::BTreeMap, path::PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::message_relayer::eth_to_gear::paid_token_transfers::task_manager::Task;

pub enum Storage {
    #[allow(dead_code)]
    None,
    #[allow(dead_code)]
    Json(PathBuf),
}

impl Storage {
    pub async fn save_tasks(&self, tasks: &BTreeMap<Uuid, Task>) -> anyhow::Result<()> {
        match self {
            Storage::None => Ok(()),
            Storage::Json(path) => {
                if !path.exists() {
                    tokio::fs::create_dir_all(path).await?;
                }
                for (task_uuid, task) in tasks {
                    let mut filename = path.join(task_uuid.to_string());
                    filename.set_extension("json");

                    let mut file = tokio::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&filename)
                        .await?;
                    let json = serde_json::to_string(task)?;
                    file.write_all(json.as_bytes()).await?;
                    file.flush().await?;
                }
                Ok(())
            }
        }
    }

    pub async fn load_tasks(&self) -> anyhow::Result<BTreeMap<Uuid, Task>> {
        match self {
            Storage::None => Ok(BTreeMap::new()),
            Storage::Json(path) => {
                let mut tasks = BTreeMap::new();
                if !path.exists() {
                    return Ok(tasks);
                }
                let mut dir = tokio::fs::read_dir(path).await?;
                while let Some(entry) = dir.next_entry().await? {
                    if entry.file_type().await?.is_file() {
                        let mut file = tokio::fs::File::open(entry.path()).await?;
                        let uuid = entry
                            .file_name()
                            .to_str()
                            .and_then(|s| Uuid::parse_str(s).ok())
                            .ok_or_else(|| anyhow::anyhow!("Invalid UUID in filename"))?;
                        let mut contents = String::new();
                        file.read_to_string(&mut contents).await?;
                        let task: Task = serde_json::from_str(&contents)?;
                        if uuid != task.uuid {
                            return Err(anyhow::anyhow!(
                                "UUID in filename does not match task UUID"
                            ));
                        }
                        tasks.insert(task.uuid, task);
                    }
                }
                Ok(tasks)
            }
        }
    }
}
