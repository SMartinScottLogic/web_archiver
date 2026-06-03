mod cache;
mod fingerprint;
mod media;
mod planner;
mod mover;

use clap::Parser;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};
use std::path::{PathBuf, absolute};
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "video-sorter")]
struct Args {
    /// Paths to scan (positional, can be multiple)
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    /// Re-scan destination folders (video_*, image, temp1)
    #[arg(long)]
    rescan_destinations: bool,

    /// Re-scan only temp1 (ignore other destination filters)
    #[arg(long)]
    rescan_temp1: bool,

    /// Duration precision (decimal places)
    #[arg(long, default_value_t = 1)]
    duration_precision: usize,

    /// Frame hashing threshold (Hamming distance)
    #[arg(long, default_value_t = 8)]
    hash_threshold: u32,
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
        .with_span_events(FmtSpan::NONE)
        .init();
}

fn main() -> anyhow::Result<()> {
    setup_logging();
    
    let args = Args::parse();

    info!(?args, "Starting video-sorter");

    let mut cache = cache::load_cache()?;
    cache::prune_missing(&mut cache);

    let mut planner = planner::Planner::new(cache, args.hash_threshold, args.duration_precision);

    for root in &args.paths {
        let abs_path = absolute(root)?;
        info!(path=%&abs_path.display(), "Starting scan");
        planner.scan(&abs_path)?;
        info!(path=%&abs_path.display(), "Finished scan");
    }

    let (plan, cache) = planner.finalize();

    let total = plan.len();
    let images = plan.iter().filter(|op| matches!(op, mover::MoveOp::Image(_))).count();
    let temp1 = plan.iter().filter(|op| matches!(op, mover::MoveOp::Temp1(_))).count();
    let clusters = plan.iter().filter(|op| matches!(op, mover::MoveOp::Cluster(_, _))).count();

    info!(total, images, temp1, clusters, "Plan finalized");

    mover::execute(plan)?;

    cache::save_cache(&cache)?;

    Ok(())
}