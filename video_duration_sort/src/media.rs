use std::path::Path;
use std::process::{Command, Stdio};

use tracing::debug;

#[allow(dead_code)]
pub fn is_image(mime: &str) -> bool {
    mime.starts_with("image/")
}

#[allow(dead_code)]
pub fn is_video(mime: &str) -> bool {
    mime.starts_with("video/")
}

pub fn infer_mime(path: &Path) -> Option<String> {
    infer::get_from_path(path).ok().flatten().map(|k| k.mime_type().to_string())
}

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