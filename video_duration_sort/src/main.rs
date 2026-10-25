mod analyse;
mod bktree;
mod media;
mod mover;
mod planner;
mod retain;

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

#[derive(Parser, Debug)]
#[command(name = "video-sorter")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Debug, Clone)]
struct AnalyseArgs {
    /// Paths to scan (positional, can be multiple)
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    /// Re-scan destination folders (video_*, image)
    #[arg(long)]
    rescan_destinations: bool,

    /// Read-only (scan and plan only)
    #[arg(long)]
    read_only: bool,

    /// Duration precision (decimal places)
    #[arg(long, default_value_t = 1)]
    duration_precision: usize,

    /// Frame hashing threshold (Hamming distance)
    #[arg(long, default_value_t = 8)]
    hash_threshold: u32,

    /// Minimum image pixels to retain
    #[arg(long, default_value_t = 0)]
    min_image_pixels: u32,
}

#[derive(Args, Debug, Clone)]
struct RetainArgs {
    /// Read-only (scan and plan only)
    #[arg(long)]
    read_only: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Analyse and sort media files
    Analyse(AnalyseArgs),
    /// Retain the best media files
    RetainBest(RetainArgs),
}

/// Initialize logging ---
fn setup_logging() {
    // Initialize logging
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy()
        .add_directive("html5ever::serialize=error".parse().unwrap());
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_span_events(FmtSpan::ACTIVE)
        .init();
}

fn main() -> anyhow::Result<()> {
    setup_logging();

    match Cli::parse().command {
        Command::Analyse(args) => analyse::analyse(args),
        Command::RetainBest(args) => retain::analyse(&args).map(|freed| {
            info!(
                freed = humfmt::bytes(freed).to_string(),
                ?args,
                "retain best"
            )
        }),
    }
}
