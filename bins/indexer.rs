use anyhow::Result;
use std::env;
use csv::create_archive_index;

mod csv;

fn main() -> Result<()> {
    // Arguments: archive_root output_csv
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <archive_root> <output_csv>", args[0]);
        std::process::exit(1);
    }

    let archive_root = &args[1];
    let output_csv = &args[2];

    create_archive_index(archive_root, output_csv)?;
    println!("Archive index written to {}", output_csv);

    Ok(())
}
