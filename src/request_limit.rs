use std::{
    env,
    fs::{self, File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use tokio::sync::{Semaphore, SemaphorePermit};

const MAX_CONCURRENT_REQUESTS: usize = 2;
static LOCAL_PERMITS: Semaphore = Semaphore::const_new(MAX_CONCURRENT_REQUESTS);

pub struct RequestPermit {
    _local: SemaphorePermit<'static>,
    _shared: File,
}

pub async fn acquire() -> Result<RequestPermit> {
    acquire_in(&lock_directory()).await
}

async fn acquire_in(directory: &Path) -> Result<RequestPermit> {
    let local = LOCAL_PERMITS.acquire().await?;
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;

    loop {
        for slot in 0..MAX_CONCURRENT_REQUESTS {
            let path = directory.join(format!("slot-{slot}.lock"));
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .with_context(|| format!("failed to open {}", path.display()))?;
            match file.try_lock() {
                Ok(()) => {
                    return Ok(RequestPermit {
                        _local: local,
                        _shared: file,
                    });
                }
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(error)) => {
                    return Err(error)
                        .with_context(|| format!("failed to lock {}", path.display()));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn lock_directory() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join(".cache/jevlint/request-semaphore")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn limits_requests_to_two_shared_slots() {
        let directory = tempfile::tempdir().unwrap();
        let first = acquire_in(directory.path()).await.unwrap();
        let second = acquire_in(directory.path()).await.unwrap();

        for slot in 0..MAX_CONCURRENT_REQUESTS {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(directory.path().join(format!("slot-{slot}.lock")))
                .unwrap();
            assert!(matches!(file.try_lock(), Err(TryLockError::WouldBlock)));
        }

        assert!(
            tokio::time::timeout(Duration::from_millis(100), acquire_in(directory.path()))
                .await
                .is_err()
        );
        drop(first);
        let third = tokio::time::timeout(Duration::from_secs(1), acquire_in(directory.path()))
            .await
            .unwrap()
            .unwrap();
        drop((second, third));
    }
}
