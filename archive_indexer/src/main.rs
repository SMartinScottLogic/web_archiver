use anyhow::Result;
use archive_indexer::{Args, run};
use clap::Parser;

fn main() -> Result<()> {
    let args = Args::parse();
    run(args)
}
