use image::ImageReader;
use image_hasher::{HashAlg, HasherConfig, ImageHash};
use tracing::debug;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::tempdir;

#[derive(Clone, Debug)]
pub struct VideoFingerprint {
    pub q25: ImageHash,
    pub q50: ImageHash,
    pub q75: ImageHash,
}

/// Convert an ImageHash to a proper hex string for storage and folder names
pub fn hash_to_hex(hash: &ImageHash) -> String {
    // Use a hash of the debug representation to create a stable hex string
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    format!("{:?}", hash).hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn fingerprint_video(path: &Path, duration: f64) -> Option<VideoFingerprint> {
    debug!(?path, "fingerprint");
    let q25 = extract_and_hash(path, duration * 0.25)?;
    let q50 = extract_and_hash(path, duration * 0.50)?;
    let q75 = extract_and_hash(path, duration * 0.75)?;

    Some(VideoFingerprint { q25, q50, q75 })
}

fn extract_and_hash(path: &Path, ts: f64) -> Option<ImageHash> {
    // Create temp directory and temp file with .ppm extension
    let tmp_dir = tempdir().ok()?;
    let tmp_path = tmp_dir.path().join("frame.ppm");

    let status = Command::new("ffmpeg")
        .args([
            "-v", "error",
            "-ss", &ts.to_string(),
            "-i", path.to_str()?,
            "-frames:v", "1",
            tmp_path.to_str()?,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;

    if !status.success() {
        return None;
    }

    let img = ImageReader::open(&tmp_path).ok()?.decode().ok()?;

    let hasher = HasherConfig::new()
        .hash_alg(HashAlg::Gradient)
        .to_hasher();

    Some(hasher.hash_image(&img))
}

pub fn distance(a: &VideoFingerprint, b: &VideoFingerprint) -> u32 {
    a.q25.dist(&b.q25)
        + a.q50.dist(&b.q50)
        + a.q75.dist(&b.q75)
}