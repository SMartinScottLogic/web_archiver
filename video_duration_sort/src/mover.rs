use std::fs;
use std::path::{Path, PathBuf, absolute};
use tracing::info;

#[derive(Debug)]
pub enum Operation {
    Cluster { target: String, files: Vec<PathBuf> },
    Image(PathBuf),
    Delete(PathBuf),
}

pub fn execute(ops: Vec<Operation>) -> anyhow::Result<()> {
    info!(count = ops.len(), "Mover: executing move operations");

    for (idx, op) in ops.into_iter().enumerate() {
        match op {
            Operation::Image(p) => move_file(&p, Path::new("./image"))?,
            Operation::Cluster { target: dir, files } => {
                let base = PathBuf::from(dir);

                for f in files {
                    info!(op_index=idx, src=%f.display(), dest=%base.display(), "Mover: moving to cluster dir");
                    move_file(&f, &base)?;
                }
            }
            Operation::Delete(p) => delete_file(&p)?,
        }
    }

    Ok(())
}

fn delete_file(file: &Path) -> anyhow::Result<()> {
    info!(file=%file.display(), "Mover: preparing to delete file");
    Ok(())
}

fn move_file(src: &Path, dest_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dest_dir)?;

    let dest = dest_dir.join(src.file_name().unwrap());

    info!(src=%src.display(), dest=%dest.display(), "Mover: preparing to move file");

    // Normalize to absolute paths for comparison
    let src_abs = absolute(src)?;
    let dest_abs = absolute(&dest)?;

    // If source and destination are the same, skip
    if src_abs == dest_abs {
        info!(path=%src_abs.display(), "Mover: file already at destination, skipping");
        return Ok(());
    }

    if dest.exists() {
        // Check if dest is the same file as src (by inode)
        if let (Ok(src_meta), Ok(dest_meta)) = (fs::metadata(&src_abs), fs::metadata(&dest_abs)) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if src_meta.ino() == dest_meta.ino() {
                    info!(path=%src_abs.display(), "Mover: source and destination are the same file, skipping");
                    return Ok(());
                }
            }
        }

        // Different file exists at destination, rename source with suffix
        let mut n = 1;
        let mut new_dest = dest.clone();

        while new_dest.exists() {
            new_dest = dest_dir.join(format!(
                "{}.~{}~",
                src.file_name().unwrap().to_string_lossy(),
                n
            ));
            n += 1;
        }

        info!(src=%src.display(), new_dest=%new_dest.display(), "Mover: destination exists (different file), using new name");
        fs::rename(&src_abs, new_dest)?;
    } else {
        fs::rename(&src_abs, &dest)?;
    }

    info!(src=%src.display(), dest=%dest.display(), "Mover: move complete");

    Ok(())
}
