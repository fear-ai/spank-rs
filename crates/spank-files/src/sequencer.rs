//! Path sequencing — lexicographic or by mtime.

use std::path::PathBuf;

pub use spank_cfg::ReadOrder;

/// Order paths according to `ReadOrder`. Paths missing metadata fall
/// back to lexicographic ordering.
pub fn order_paths(mut paths: Vec<PathBuf>, order: ReadOrder) -> Vec<PathBuf> {
    match order {
        ReadOrder::Lexicographic => {
            paths.sort();
        }
        ReadOrder::Mtime => {
            paths.sort_by_key(|p| {
                std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            });
        }
    }
    paths
}
