//! `FileMonitor` — file ingest reader.
//!
//! Modes:
//! - `OneShot`: read until EOF, enqueue `Sentinel::end(path)`.
//! - `Tail`: follow growth; detect rotation via inode change and
//!   reopen.
//!
//! Output: lines to a bounded `mpsc` channel, plus a `Sentinel` on
//! source termination.

use std::path::{Path, PathBuf};
use std::time::Duration;

use spank_cfg::FileMode;
use spank_core::error::{Result, SpankError};
use spank_core::lifecycle::Lifecycle;
use spank_core::Sentinel;
use spank_obs::{ingest_event, lifecycle_event};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader, SeekFrom};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct FileLine {
    pub path: PathBuf,
    pub line: String,
}

/// Output emitted by a `FileMonitor`. Consumers should treat the
/// `Sentinel` as the end marker for a given path.
#[derive(Debug)]
pub enum FileOutput {
    Line(FileLine),
    Done(Sentinel),
}

pub struct FileMonitor {
    pub path: PathBuf,
    pub mode: FileMode,
    pub channel_depth: usize,
}

impl FileMonitor {
    #[must_use]
    pub fn new(path: PathBuf, mode: FileMode, channel_depth: usize) -> Self {
        Self {
            path,
            mode,
            channel_depth,
        }
    }

    /// Run the monitor. Returns when the source is exhausted (one-shot)
    /// or when `lifecycle` cancels (tail).
    pub async fn run(self, tx: mpsc::Sender<FileOutput>, lifecycle: Lifecycle) -> Result<()> {
        lifecycle_event!(
            component = "file_monitor",
            kind = "start",
            path = %self.path.display(),
            mode = ?self.mode,
        );
        let res = match self.mode {
            FileMode::OneShot => self.run_oneshot(&tx, &lifecycle).await,
            FileMode::Tail => self.run_tail(&tx, &lifecycle).await,
        };
        // Always emit Sentinel::end on exit, success or error.
        let _ = tx
            .send(FileOutput::Done(Sentinel::end(self.path.display().to_string())))
            .await;
        lifecycle_event!(
            component = "file_monitor",
            kind = "stop",
            path = %self.path.display()
        );
        res
    }

    async fn run_oneshot(&self, tx: &mpsc::Sender<FileOutput>, lifecycle: &Lifecycle) -> Result<()> {
        let f = File::open(&self.path)
            .await
            .map_err(|e| SpankError::io_path("open", &self.path, e))?;
        let mut r = BufReader::new(f).lines();
        let mut count: u64 = 0;
        loop {
            tokio::select! {
                _ = lifecycle.token.cancelled() => break,
                line = r.next_line() => {
                    let line = line.map_err(|e| SpankError::io_path("read_line", &self.path, e))?;
                    match line {
                        Some(l) => {
                            count += 1;
                            metrics::counter!(spank_obs::metrics::names::FILE_LINES_READ_TOTAL).increment(1);
                            metrics::counter!(spank_obs::metrics::names::FILE_BYTES_READ_TOTAL)
                                .increment((l.len() + 1) as u64);
                            if tx.send(FileOutput::Line(FileLine {
                                path: self.path.clone(),
                                line: l,
                            })).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
        ingest_event!(
            kind = "file.oneshot_done",
            path = %self.path.display(),
            lines = count
        );
        Ok(())
    }

    async fn run_tail(&self, tx: &mpsc::Sender<FileOutput>, lifecycle: &Lifecycle) -> Result<()> {
        // Open at the current end and follow growth. On inode change
        // (rotation), reopen at the start and continue.
        let mut current_inode = inode_of(&self.path).ok();
        let mut f = File::open(&self.path)
            .await
            .map_err(|e| SpankError::io_path("open", &self.path, e))?;
        f.seek(SeekFrom::End(0))
            .await
            .map_err(|e| SpankError::io_path("seek_end", &self.path, e))?;
        let mut reader = BufReader::new(f).lines();

        loop {
            tokio::select! {
                _ = lifecycle.token.cancelled() => break,
                line = reader.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            metrics::counter!(spank_obs::metrics::names::FILE_LINES_READ_TOTAL).increment(1);
                            metrics::counter!(spank_obs::metrics::names::FILE_BYTES_READ_TOTAL)
                                .increment((l.len() + 1) as u64);
                            if tx.send(FileOutput::Line(FileLine {
                                path: self.path.clone(),
                                line: l,
                            })).await.is_err() {
                                break;
                            }
                        }
                        Ok(None) => {
                            // EOF — sleep then check rotation.
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            let new_inode = inode_of(&self.path).ok();
                            if new_inode != current_inode {
                                ingest_event!(
                                    kind = "file.rotated",
                                    path = %self.path.display()
                                );
                                current_inode = new_inode;
                                let f = File::open(&self.path)
                                    .await
                                    .map_err(|e| SpankError::io_path("reopen", &self.path, e))?;
                                reader = BufReader::new(f).lines();
                            }
                        }
                        Err(e) => {
                            return Err(SpankError::io_path("read_line", &self.path, e));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn inode_of(path: &Path) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::metadata(path)?.ino())
}

#[cfg(not(unix))]
fn inode_of(_path: &Path) -> std::io::Result<u64> {
    Ok(0)
}
