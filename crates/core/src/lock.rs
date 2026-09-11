//! Transactional and exclusive file lock guards.
//! Guarantees exclusive create-new semantics, durable write-sync, and
//! ownership-aware rollback on drop or failure.

use crate::error::CoreError;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

/// An exclusive lockfile guard that holds an uncommitted `.lock` file.
///
/// On drop, if not committed, the lockfile is removed.
/// An existing lock file is NEVER truncated or stolen from another process.
#[derive(Debug)]
pub struct LockFile {
    target_path: PathBuf,
    lock_path: PathBuf,
    file: Option<File>,
    active: bool,
}

impl LockFile {
    /// Attempts to acquire an exclusive lock on `target_path`.
    /// Creates `<target_path>.lock` using atomic `create_new(true)` semantics.
    pub fn acquire(target_path: impl AsRef<Path>) -> Result<Self, CoreError> {
        let target_path = target_path.as_ref().to_path_buf();
        let lock_path = PathBuf::from(format!("{}.lock", target_path.display()));

        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).map_err(CoreError::Io)?;
        }

        let file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                return Err(CoreError::LockError(format!(
                    "lockfile '{}' already exists; unable to lock '{}'",
                    lock_path.display(),
                    target_path.display()
                )));
            }
            Err(e) => return Err(CoreError::Io(e)),
        };

        Ok(Self {
            target_path,
            lock_path,
            file: Some(file),
            active: true,
        })
    }

    /// Returns the target path being locked.
    pub fn target_path(&self) -> &Path {
        &self.target_path
    }

    /// Returns the lock path on disk.
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    /// Explicitly syncs buffered content and atomically renames the lock file to the target path.
    pub fn commit(mut self) -> Result<(), CoreError> {
        if let Some(mut file) = self.file.take() {
            file.flush().map_err(CoreError::Io)?;
            file.sync_all().map_err(CoreError::Io)?;
            drop(file); // Release file descriptor before rename (mandatory on Windows)
        }

        // Atomically replace target with lock file
        if cfg!(windows) && self.target_path.exists() {
            let backup_path = PathBuf::from(format!("{}.lockbackup", self.target_path.display()));
            if backup_path.exists() {
                let _ = fs::remove_file(&backup_path);
            }
            if let Err(_e) = fs::rename(&self.target_path, &backup_path) {
                // If moving to backup fails, attempt standard rename directly
                fs::rename(&self.lock_path, &self.target_path).map_err(CoreError::Io)?;
            } else {
                match fs::rename(&self.lock_path, &self.target_path) {
                    Ok(()) => {
                        let _ = fs::remove_file(&backup_path);
                    }
                    Err(e) => {
                        // Restore from backup on failure
                        let _ = fs::rename(&backup_path, &self.target_path);
                        return Err(CoreError::Io(e));
                    }
                }
            }
        } else {
            fs::rename(&self.lock_path, &self.target_path).map_err(CoreError::Io)?;
        }
        self.active = false;
        Ok(())
    }

    /// Explicitly cancels the operation and removes the lock file.
    pub fn rollback(mut self) {
        self.active = false;
        self.file.take();
        let _ = fs::remove_file(&self.lock_path);
    }
}

impl Write for LockFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.file.as_mut() {
            Some(f) => f.write(buf),
            None => Err(io::Error::other(
                "lockfile already committed or rolled back",
            )),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        if self.active {
            self.file.take();
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lock_acquire_and_commit() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("index");

        let mut lock = LockFile::acquire(&target).unwrap();
        let lock_path = lock.lock_path().to_path_buf();
        assert!(lock_path.exists());
        assert!(!target.exists());

        lock.write_all(b"test data").unwrap();
        lock.commit().unwrap();

        assert!(!lock_path.exists());
        assert!(target.exists());
        assert_eq!(fs::read(&target).unwrap(), b"test data");
    }

    #[test]
    fn test_existing_lock_blocks_acquire() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("index");
        let lock_path = temp_dir.path().join("index.lock");

        fs::write(&lock_path, b"existing owner").unwrap();

        let res = LockFile::acquire(&target);
        assert!(res.is_err());
        // Existing lock file must still exist and be intact
        assert!(lock_path.exists());
        assert_eq!(fs::read(&lock_path).unwrap(), b"existing owner");
    }

    #[test]
    fn test_lock_drop_cleans_up_without_commit() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("index");

        {
            let mut lock = LockFile::acquire(&target).unwrap();
            lock.write_all(b"aborted").unwrap();
            assert!(lock.lock_path().exists());
        }

        assert!(!target.exists());
        assert!(!temp_dir.path().join("index.lock").exists());
    }
}
