use std::path::Path;

pub mod image;
pub mod video;

#[allow(dead_code)]
pub fn is_image(mime: &str) -> bool {
    mime.starts_with("image/")
}

#[allow(dead_code)]
pub fn is_video(mime: &str) -> bool {
    mime.starts_with("video/")
}

pub fn infer_mime(path: &Path) -> Option<String> {
    infer::get_from_path(path)
        .ok()
        .flatten()
        .map(|k| k.mime_type().to_string())
}
