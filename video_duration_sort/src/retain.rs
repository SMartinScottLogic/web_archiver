use std::{
    path::Path,
    process::{Command, Stdio},
};

use serde::Deserialize;
use tracing::{debug, error, info};
use walkdir::WalkDir;

#[derive(Debug)]
struct Stats {
    filename: String,
    size: u64,
    width: i64,
    height: i64,
}

impl From<FFProbeStats> for Stats {
    fn from(value: FFProbeStats) -> Self {
        Self {
            filename: value.format.filename,
            size: value.format.size.parse().unwrap_or_default(),
            width: value.streams[0].width,
            height: value.streams[0].height,
        }
    }
}

#[derive(Debug, Deserialize)]
struct FFProbeStats {
    format: Format,
    streams: Vec<Stream>,
}

#[derive(Debug, Deserialize)]
struct Format {
    filename: String,
    size: String,
}

#[derive(Debug, Deserialize)]
struct Stream {
    width: i64,
    height: i64,
}

pub fn analyse(args: &crate::RetainArgs) -> anyhow::Result<u64> {
    let mut total_free = 0u64;
    for entry in WalkDir::new(".")
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|p| p.to_str())
            && name.starts_with("video_")
        {
            total_free += analyse_folder(args, path);
        };
    }
    Ok(total_free)
}

fn analyse_folder(args: &crate::RetainArgs, folder: &Path) -> u64 {
    info!(?folder, "analyse");

    let stats = WalkDir::new(folder)
        .max_depth(1)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            Some(path.to_path_buf())
        }).collect::<Vec<_>>();
    let mut free = 0u64;
    if stats.len() <= 1 {
        return free;
    }
    let mut stats = stats.into_iter().filter_map(|path| {
            let stats = ffprobe_size(&path)?;
            debug!(?folder, ?path, ?stats, "file");
            Some(stats)
        })
        .collect::<Vec<_>>();
    if stats.len() > 1 {
        sort_stats(&mut stats);
        let remove = stats.split_off(1);
        if args.read_only {
            free = remove.iter().map(|stat| stat.size).sum();
            info!(
                ?stats,
                ?remove,
                "read-only: would retain best file and delete the rest"
            );
        } else {
            info!(?stats, ?remove, "deleting lower-ranked files");
            free = remove_files(&remove);
        }
    }
    free
}

fn remove_files(stats: &[Stats]) -> u64 {
    let mut freed = 0;
    for stat in stats {
        match std::fs::remove_file(&stat.filename) {
            Ok(()) => {
                info!(filename = %stat.filename, size = stat.size, "deleted file");
                freed += stat.size;
            }
            Err(err) => error!(
                filename = %stat.filename,
                ?err,
                "failed to delete file"
            ),
        }
    }
    freed
}

fn sort_stats(stats: &mut [Stats]) {
    stats.sort_by_cached_key(|stat| {
        let min_d = i64::min(stat.height, stat.width);
        (
            min_d < 720,
            (min_d - 720).abs(),
            !stat.filename.ends_with(".mp4"),
        )
    });
}

fn ffprobe_size(path: &Path) -> Option<Stats> {
    debug!(?path, "ffprobe size");
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=filename,size:stream=width,height",
            "-select_streams",
            "v:0",
            "-of",
            "json",
            path.to_str()?,
        ])
        .stdin(Stdio::null())
        .output()
        .ok()?;

    if !out.status.success() {
        error!(out = ?String::from_utf8_lossy(&out.stdout), err =  ?String::from_utf8_lossy(&out.stderr), "probe failure");
        return None;
    }

    let s = String::from_utf8_lossy(&out.stdout);
    debug!(?s, "result");
    serde_json::from_str(&s)
        .inspect_err(|e| error!(?e, "serde failure"))
        .map(|s: FFProbeStats| s.into())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(filename: &str, width: i64, height: i64) -> Stats {
        Stats {
            filename: filename.to_owned(),
            size: 0,
            width,
            height,
        }
    }

    fn filenames(stats: &[Stats]) -> Vec<&str> {
        stats.iter().map(|stat| stat.filename.as_str()).collect()
    }

    #[test]
    fn sorts_dimensions_at_or_above_720_before_smaller_dimensions() {
        let mut items = vec![
            stats("small.mp4", 640, 480),
            stats("large.mp4", 1920, 1080),
            stats("threshold.mp4", 1280, 720),
        ];

        sort_stats(&mut items);

        assert_eq!(
            filenames(&items),
            ["threshold.mp4", "large.mp4", "small.mp4"]
        );
    }

    #[test]
    fn sorts_by_distance_from_720_within_each_dimension_group() {
        let mut items = vec![
            stats("below-far.mp4", 640, 480),
            stats("above-far.mp4", 3840, 2160),
            stats("below-near.mp4", 1280, 700),
            stats("above-near.mp4", 1920, 740),
        ];

        sort_stats(&mut items);

        assert_eq!(
            filenames(&items),
            [
                "above-near.mp4",
                "above-far.mp4",
                "below-near.mp4",
                "below-far.mp4"
            ]
        );
    }

    #[test]
    fn prefers_mp4_when_dimensions_have_the_same_sort_key() {
        let mut items = vec![
            stats("other.mkv", 1280, 720),
            stats("preferred.mp4", 1920, 720),
        ];

        sort_stats(&mut items);

        assert_eq!(filenames(&items), ["preferred.mp4", "other.mkv"]);
    }

    #[test]
    fn removes_files_and_reports_successfully_freed_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let filename = directory.path().join("duplicate.mp4");
        std::fs::write(&filename, b"duplicate").unwrap();
        let mut item = stats(filename.to_str().unwrap(), 1280, 720);
        item.size = 9;

        assert_eq!(remove_files(&[item]), 9);
        assert!(!filename.exists());
    }

    #[test]
    fn reports_zero_bytes_when_file_removal_fails() {
        let directory = tempfile::tempdir().unwrap();
        let item = stats(directory.path().to_str().unwrap(), 1280, 720);

        assert_eq!(remove_files(&[item]), 0);
        assert!(directory.path().exists());
    }
}
