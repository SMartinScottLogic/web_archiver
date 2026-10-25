use std::path::absolute;
use tracing::info;

pub fn analyse(args: crate::AnalyseArgs) -> anyhow::Result<()> {
    use crate::mover::Operation;
    use crate::planner::Planner;

    info!(?args, "Starting video-sorter");

    let mut planner = Planner::new(args.clone());

    for root in &args.paths {
        let abs_path = absolute(root)?;
        info!(path=%&abs_path.display(), "Starting scan");
        planner.scan(&abs_path)?;
        info!(path=%&abs_path.display(), "Finished scan");
    }

    let plan = planner.finalize();

    let total = plan.len();
    let images = plan
        .iter()
        .filter(|op| matches!(op, Operation::Image(_)))
        .count();
    let clusters = plan
        .iter()
        .filter(|op| {
            matches!(
                op,
                Operation::Cluster {
                    target: _,
                    files: _
                }
            )
        })
        .count();
    let delete = plan
        .iter()
        .filter(|op| matches!(op, Operation::Delete(_)))
        .count();

    info!(total, images, clusters, delete, "Plan finalized");

    if !args.read_only {
        crate::mover::execute(plan)?;
    } else {
        info!(?plan, "plan");
    }

    Ok(())
}
