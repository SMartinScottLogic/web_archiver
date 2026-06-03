use crate::fingerprint::{distance, fingerprint_video};
use crate::media::{ffprobe_duration, infer_mime, is_image};
use crate::cache::{Cache, get_cache_if_valid, update_cache_fingerprint};
use rayon::prelude::*;
use tracing::{info, instrument};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub struct Planner {
    videos: Vec<PathBuf>,
    images: Vec<PathBuf>,
    durations: HashMap<String, usize>,
    cache: Cache,
    hash_threshold: u32,
    duration_precision: usize,
}

impl Planner {
    pub fn new(cache: Cache, hash_threshold: u32, duration_precision: usize) -> Self {
        Self {
            videos: vec![],
            images: vec![],
            durations: HashMap::new(),
            cache,
            hash_threshold,
            duration_precision,
        }
    }

    #[instrument(skip(self))]
    pub fn scan(&mut self, root: &Path) -> anyhow::Result<()> {
        assert!(root.is_absolute());
        info!("Planner: scanning path");
        for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let mime = infer_mime(path);

            if let Some(m) = mime
                && is_image(&m) {
                    self.images.push(path.to_path_buf());
                    continue;
                }

            let path_buf = path.to_path_buf();
            let duration = if let Some(cached) = get_cache_if_valid(&self.cache, &path_buf) {
                Some(cached.duration)
            } else {
                ffprobe_duration(path)
            };

            if let Some(d) = duration {
                let bucket = format!("{:.prec$}", d, prec = self.duration_precision);
                *self.durations.entry(bucket).or_insert(0) += 1;
                self.videos.push(path_buf);
            }
        }

        info!(videos=%self.videos.len(), images=%self.images.len(), "Planner: scan complete");

        Ok(())
    }

    pub fn finalize(mut self) -> (Vec<crate::mover::MoveOp>, Cache) {
        info!("Planner: finalizing plan");

        let mut ops = vec![];

        let grouped: HashMap<String, Vec<PathBuf>> = self
            .videos
            .into_iter()
            .fold(HashMap::new(), |mut acc, v| {
                let d = ffprobe_duration(&v).unwrap_or(0.0);
                let key = format!("{:.prec$}", d, prec = self.duration_precision);
                acc.entry(key).or_default().push(v);
                acc
            });

        for (bucket, files) in grouped {
            info!(bucket=%bucket, count=%files.len(), "Processing bucket");

            if files.len() == 1 {
                ops.push(crate::mover::MoveOp::Temp1(files[0].clone()));
                continue;
            }

            // Collect fingerprints with file paths and durations for caching
            let fps_with_meta: Vec<_> = files
                .par_iter()
                .map(|f| {
                    let d = ffprobe_duration(f).unwrap_or(0.0);
                    let fp = fingerprint_video(f, d);
                    (f.clone(), d, fp)
                })
                .collect();

            // Update cache with newly computed fingerprints
            for (path, duration, fp) in &fps_with_meta {
                if let Some(fp) = fp
                    && let Ok(_) = update_cache_fingerprint(
                        &mut self.cache,
                        path.clone(),
                        *duration,
                        crate::fingerprint::hash_to_hex(&fp.q25),
                        crate::fingerprint::hash_to_hex(&fp.q50),
                        crate::fingerprint::hash_to_hex(&fp.q75),
                    ) {
                        info!(path=%path.display(), "Cached fingerprint");
                    }
            }

            // Extract just the fingerprints for clustering
            let fps: Vec<Option<crate::fingerprint::VideoFingerprint>> = fps_with_meta
                .iter()
                .map(|(_, _, fp)| fp.clone())
                .collect();

            let mut clusters: Vec<(Vec<PathBuf>, crate::fingerprint::VideoFingerprint)> = vec![];

            for (i, f) in files.iter().enumerate() {
                let fp = match &fps[i] {
                    Some(x) => x,
                    None => continue,
                };

                let mut placed = false;

                for (cluster, rep_fp) in &mut clusters {
                    if distance(fp, rep_fp) <= self.hash_threshold {
                        cluster.push(f.clone());
                        placed = true;
                        break;
                    }
                }

                if !placed {
                    clusters.push((vec![f.clone()], fp.clone()));
                }
            }

            for (cluster, rep_fp) in clusters.into_iter() {
                let hash_repr = crate::fingerprint::hash_to_hex(&rep_fp.q50)[0..12].to_string();
                info!(bucket=%bucket, hash=%hash_repr, cluster_size=%cluster.len(), "Found cluster");

                if cluster.len() == 1 {
                    ops.push(crate::mover::MoveOp::Temp1(cluster[0].clone()));
                } else {
                    let dir = format!("video_{}_{}", bucket, hash_repr);
                    ops.push(crate::mover::MoveOp::Cluster(dir, cluster));
                }
            }
        }

        for img in self.images {
            ops.push(crate::mover::MoveOp::Image(img));
        }

        info!(ops=%ops.len(), "Planner: finalize complete");

        (ops, self.cache)
    }
}