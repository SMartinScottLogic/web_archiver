use std::time::Duration;

use anyhow::Result;
use archive_indexer::create_archive_index;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Parser, Debug)]
#[clap(
    name = "archive_indexer",
    version = "0.1.1",
    about = "Create an index of archive files"
)]
struct Args {
    /// Archive root directory
    #[clap(value_name = "ARCHIVE_ROOT")]
    archive_root: String,

    /// Output CSV file
    #[clap(value_name = "OUTPUT_CSV")]
    output_csv: String,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let pb = ProgressBar::new_spinner();

    pb.set_style(
        ProgressStyle::with_template("{spinner} {pos} items [{elapsed}] ({per_sec}) {msg}")
            .unwrap(),
    );

    pb.enable_steady_tick(Duration::from_millis(100));
    pb.set_message("Processing...");

    create_archive_index(&args.archive_root, &args.output_csv, &pb)?;

    pb.finish_with_message("Done");

    println!("Archive index written to {}", args.output_csv);
    Ok(())
}
