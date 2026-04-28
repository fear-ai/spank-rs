//! `spank-files` — `FileMonitor` and supporting machinery.
//!
//! Reads files in two modes:
//! - `OneShot`: read the file end-to-end, emit `Sentinel::end(path)`.
//! - `Tail`: follow growth, react to rotation by inode change.
//!
//! Sequencing knobs (configurable per source):
//! - `order`: lexicographic | mtime
//! - `workers`: parse-side parallelism per monitor
//! - `channel_depth`: bounded channel size

pub mod monitor;
pub mod sequencer;

pub use monitor::{FileMonitor, FileLine};
pub use sequencer::{order_paths, ReadOrder};
