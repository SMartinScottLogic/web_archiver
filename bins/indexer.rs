use anyhow::Result;
use clap::Parser;
use csv::create_archive_index;

mod csv;

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

    create_archive_index(&args.archive_root, &args.output_csv)?;
    println!("Archive index written to {}", args.output_csv);
    Ok(())
}
