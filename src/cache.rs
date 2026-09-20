use std::{path::Path, sync::Arc};

use anyhow::Context;
use redb::{Database, ReadableDatabase, TableDefinition};
use serde::{Serialize, de::DeserializeOwned};

const RESULTS: TableDefinition<&str, &[u8]> = TableDefinition::new("results");

#[derive(Clone)]
pub struct Cache {
    database: Arc<Database>,
}

impl Cache {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let path = path.to_owned();
        let database = tokio::task::spawn_blocking(move || Database::create(path)).await??;
        {
            let transaction = database.begin_write()?;
            transaction.open_table(RESULTS)?;
            transaction.commit()?;
        }
        Ok(Self {
            database: Arc::new(database),
        })
    }

    pub async fn get_many<T>(&self, keys: Vec<String>) -> anyhow::Result<Vec<Option<T>>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let database = Arc::clone(&self.database);
        tokio::task::spawn_blocking(move || {
            let transaction = database.begin_read()?;
            let table = transaction.open_table(RESULTS)?;
            keys.iter()
                .map(|key| {
                    table
                        .get(key.as_str())?
                        .map(|value| {
                            serde_json::from_slice(value.value()).context("invalid cached value")
                        })
                        .transpose()
                })
                .collect()
        })
        .await?
    }

    pub async fn put_many<T>(&self, values: Vec<(String, T)>) -> anyhow::Result<()>
    where
        T: Serialize + Send + 'static,
    {
        let database = Arc::clone(&self.database);
        tokio::task::spawn_blocking(move || {
            let serialized = values
                .into_iter()
                .map(|(key, value)| Ok((key, serde_json::to_vec(&value)?)))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let transaction = database.begin_write()?;
            {
                let mut table = transaction.open_table(RESULTS)?;
                for (key, value) in &serialized {
                    table.insert(key.as_str(), value.as_slice())?;
                }
            }
            transaction.commit()?;
            Ok(())
        })
        .await?
    }
}
