use std::{
    path::Path,
    process::{Command, Stdio},
};

use tracing::debug;

pub mod fingerprint;

pub fn ffprobe_duration(path: &Path) -> Option<f64> {
    debug!(?path, "ffprobe duration");
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path.to_str()?,
        ])
        .stdin(Stdio::null())
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    let s = String::from_utf8_lossy(&out.stdout);
    s.trim().parse().ok()
}
