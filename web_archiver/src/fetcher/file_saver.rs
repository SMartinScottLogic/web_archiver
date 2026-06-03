use std::{fs::{File, create_dir_all}, io::Write as _, path::Path};

use anyhow::Result;
use mockall::automock;
use tracing::{debug, error, info};

#[automock]
pub trait Saver {
    fn save_content(&self, filename: &Path, content: &[u8], content_type: Option<(String, String)>) -> Result<()>;
}

pub struct FileSaver {
    
}

impl Saver for FileSaver {
    fn save_content(&self, filename: &Path, content: &[u8], content_type: Option<(String, String)>) -> Result<()> {
                    debug!(
                binary_size = content.len(),
                ?filename,
                ?content_type,
                "other content_type"
            );
            let _ = Self::save_content(filename, content)
                .inspect_err(|e| error!(?e, ?filename, "save content"));
            // TODO Update DB

            Ok(())

    }
}

impl FileSaver {
fn save_content(filename: &Path, content: &[u8]) -> Result<()> {
    create_dir_all(filename.parent().unwrap())?;
    let mut file = File::create(filename)?;
    file.write_all(content)?;
    info!(?filename, "media file");
    Ok(())
}
}