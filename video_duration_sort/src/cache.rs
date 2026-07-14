use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedVideo {
    pub size: u64,
    pub mtime: u64,
    pub duration: f64,
    pub hash25: String, // hex-encoded
    pub hash50: String, // hex-encoded
    pub hash75: String, // hex-encoded
}

pub type Cache = HashMap<PathBuf, CachedVideo>;

fn cache_path() -> PathBuf {
    let proj = ProjectDirs::from("com", "video-sorter", "video-sorter").expect("no project dir");
    proj.cache_dir().join("cache.json")
}

/// Convert a path to absolute path for cache key consistency
fn to_absolute_path2(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

pub fn load_cache() -> anyhow::Result<Cache> {
    let path = cache_path();

    if !path.exists() {
        return Ok(HashMap::new());
    }

    let data = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

pub fn save_cache(cache: &Cache) -> anyhow::Result<()> {
    let path = cache_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, serde_json::to_string_pretty(cache)?)?;
    Ok(())
}

pub fn prune_missing(cache: &mut Cache) {
    cache.retain(|path, _| path.exists());
}

/// Check if a file is cached and hasn't changed, returning the cached entry if valid
pub fn get_cache_if_valid(cache: &Cache, path: &PathBuf) -> Option<CachedVideo> {
    assert!(path.is_absolute());
    //let abs_path = to_absolute_path(path).ok()?;

    if let Ok(metadata) = fs::metadata(&path) {
        if let Some(cached) = cache.get(&path.to_path_buf()) {
            if let Ok(mtime) = metadata.modified() {
                let current_mtime = mtime
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if cached.size == metadata.len() && cached.mtime == current_mtime {
                    return Some(cached.clone());
                }
            }
        }
    }
    None
}

/// Check if a file is cached and hasn't changed (based on size and mtime)
pub fn is_cache_valid(cache: &Cache, path: &PathBuf) -> bool {
    get_cache_if_valid(cache, path).is_some()
}

/// Get the current size and mtime of a file
pub fn get_file_stats(path: &PathBuf) -> anyhow::Result<(u64, u64)> {
    let metadata = fs::metadata(path)?;
    let mtime = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    Ok((metadata.len(), mtime))
}

/// Update cache with new fingerprint data
pub fn update_cache_fingerprint(
    cache: &mut Cache,
    path: PathBuf,
    duration: f64,
    hash25: String,
    hash50: String,
    hash75: String,
) -> anyhow::Result<()> {
    assert!(path.is_absolute());
    //let abs_path = to_absolute_path(&path)?;
    let (size, mtime) = get_file_stats(&path)?;
    cache.insert(
        path,
        CachedVideo {
            size,
            mtime,
            duration,
            hash25,
            hash50,
            hash75,
        },
    );
    Ok(())
}
