use image::ImageReader;
use image_hasher::{HashAlg, HasherConfig, ImageHash};
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::tempdir;
use tracing::debug;

#[derive(Clone, Debug)]
pub struct VideoFingerprint {
    pub q25: Option<ImageHash>,
    pub q50: Option<ImageHash>,
    pub q75: Option<ImageHash>,
}

pub trait ToHex {
    fn to_hex(&self) -> String;
}

/// Convert an ImageHash to a proper hex string for storage and folder names
impl ToHex for ImageHash {
    fn to_hex(&self) -> String {
        // Use a hash of the debug representation to create a stable hex string
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        format!("{:?}", self).hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

pub fn fingerprint_video(path: &Path, duration: f64) -> Option<VideoFingerprint> {
    debug!(?path, "fingerprint");
    let q25 = extract_and_hash(path, duration * 0.25);
    let q50 = extract_and_hash(path, duration * 0.50);
    let q75 = extract_and_hash(path, duration * 0.75);

    if q25.is_none() && q50.is_none() && q75.is_none() {
        return None;
    }

    Some(VideoFingerprint { q25, q50, q75 })
}

fn extract_and_hash(path: &Path, ts: f64) -> Option<ImageHash> {
    // Create temp directory and temp file with .ppm extension
    let tmp_dir = tempdir().ok()?;
    let tmp_path = tmp_dir.path().join("frame.ppm");

    let status = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-ss",
            &ts.to_string(),
            "-i",
            path.to_str()?,
            "-frames:v",
            "1",
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

    let hasher = HasherConfig::new().hash_alg(HashAlg::Gradient).to_hasher();

    Some(hasher.hash_image(&img))
}

pub fn distance(a: &VideoFingerprint, b: &VideoFingerprint) -> u32 {
    let mut sum = 0;
    let mut shared_samples = 0;

    for (a, b) in [(&a.q25, &b.q25), (&a.q50, &b.q50), (&a.q75, &b.q75)] {
        if let (Some(a), Some(b)) = (a, b) {
            sum += a.dist(b);
            shared_samples += 1;
        }
    }

    if shared_samples == 0 {
        return u32::MAX;
    }

    (sum * 3 + shared_samples / 2) / shared_samples
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, GrayImage, Luma};
    use image_hasher::{HashAlg, HasherConfig};

    fn hash_with_pattern(alternate_pattern: bool) -> ImageHash {
        let image = GrayImage::from_fn(8, 8, |x, y| {
            let value = if if alternate_pattern {
                x % 2 == 0
            } else {
                (x + y) % 2 == 0
            } {
                0
            } else {
                255
            };
            Luma([value])
        });
        HasherConfig::new()
            .hash_alg(HashAlg::Gradient)
            .to_hasher()
            .hash_image(&DynamicImage::ImageLuma8(image))
    }

    #[test]
    fn partial_fingerprint_distance_uses_only_shared_samples_and_normalizes() {
        let same = hash_with_pattern(false);
        let different = hash_with_pattern(true);
        let partial = VideoFingerprint {
            q25: Some(same.clone()),
            q50: None,
            q75: None,
        };
        let full = VideoFingerprint {
            q25: Some(different.clone()),
            q50: Some(same.clone()),
            q75: Some(same),
        };

        assert_eq!(
            distance(&partial, &full),
            3 * different.dist(&hash_with_pattern(false))
        );
        assert!(distance(&partial, &full) > 0);
    }

    #[test]
    fn fingerprints_without_shared_samples_do_not_match() {
        let hash = hash_with_pattern(false);
        let a = VideoFingerprint {
            q25: Some(hash.clone()),
            q50: None,
            q75: None,
        };
        let b = VideoFingerprint {
            q25: None,
            q50: Some(hash),
            q75: None,
        };

        assert_eq!(distance(&a, &b), u32::MAX);
    }
}
