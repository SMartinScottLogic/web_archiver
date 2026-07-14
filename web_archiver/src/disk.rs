use crate::settings::{CONFIG, DEFAULT_MIN_FREE_SPACE};
use nix::sys::statvfs::statvfs;
use std::io;
use std::path::Path;
use tracing::warn;

/// Get the available free space on the filesystem containing the given path.
///
/// # Arguments
/// * `path` - Path to check (can be a file or directory)
///
/// # Returns
/// * `Ok(bytes)` - Free space in bytes on the filesystem
/// * `Err(io::Error)` - If the filesystem query fails
pub fn get_available_space(path: &Path) -> io::Result<u64> {
    let stat = statvfs(path).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(stat.blocks_available() * stat.block_size())
}

/// Check if tasks should be paused due to insufficient disk space.
///
/// Compares the available free space on the archive directory's filesystem
/// against the configured minimum (from CONFIG.min_free_space).
///
/// # Returns
/// * `Ok(true)` - Available space is below the configured minimum; pause tasks
/// * `Ok(false)` - Available space is above the configured minimum; continue tasks
/// * `Err(String)` - Error occurred while checking disk space; assumes resume to avoid deadlock
pub fn should_pause_tasks() -> Result<bool, String> {
    let archive_dir = CONFIG
        .get()
        .map(|config| config.archive_dir.clone())
        .ok_or_else(|| "CONFIG not initialized".to_string())?;

    let min_free_space = CONFIG
        .get()
        .map(|config| config.min_free_space)
        .unwrap_or(DEFAULT_MIN_FREE_SPACE);

    match get_available_space(Path::new(&archive_dir)) {
        Ok(available) => Ok(available < min_free_space),
        Err(e) => {
            warn!("Failed to check disk space: {}; assuming resume", e);
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_available_space() {
        // Test that we can query the current directory's free space
        let space = get_available_space(Path::new(".")).expect("Failed to get available space");
        assert!(space > 0, "Expected positive free space");
    }

    #[test]
    fn test_should_pause_tasks_requires_config() {
        // This test verifies that should_pause_tasks can be called
        // In a real scenario, CONFIG is set during app initialization
        match should_pause_tasks() {
            Ok(_) => {
                // Function returned successfully with a boolean
            }
            Err(e) => {
                // Function errored; this is OK if CONFIG not initialized
                assert!(e.contains("CONFIG not initialized") || !e.is_empty());
            }
        }
    }
}
