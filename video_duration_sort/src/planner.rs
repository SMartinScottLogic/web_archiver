use crate::Args;
use crate::cache::{Cache, get_cache_if_valid, update_cache_fingerprint};
use crate::media::video::fingerprint::{ToHex, distance, fingerprint_video};
use crate::media::{video::ffprobe_duration, infer_mime, is_image};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, instrument};
use walkdir::WalkDir;

#[derive(Debug)]
pub struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    pub fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    pub fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            let p = self.parent[x];
            self.parent[x] = self.find(p);
        }
        self.parent[x]
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);

        if ra == rb {
            return;
        }

        match self.rank[ra].cmp(&self.rank[rb]) {
            std::cmp::Ordering::Less => {
                self.parent[ra] = rb;
            }
            std::cmp::Ordering::Greater => {
                self.parent[rb] = ra;
            }
            std::cmp::Ordering::Equal => {
                self.parent[rb] = ra;
                self.rank[ra] += 1;
            }
        }
    }
}

pub struct Planner {
    videos: Vec<PathBuf>,
    images: Vec<PathBuf>,
    durations: HashMap<String, usize>,
    cache: Cache,
    config: Args,
}

impl Planner {
    pub fn new(cache: Cache, config: Args) -> Self {
        Self {
            videos: vec![],
            images: vec![],
            durations: HashMap::new(),
            cache,
            config,
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
                && is_image(&m)
            {
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
                let bucket = format!("{:.prec$}", d, prec = self.config.duration_precision);
                *self.durations.entry(bucket).or_insert(0) += 1;
                self.videos.push(path_buf);
            }
        }

        info!(videos=%self.videos.len(), images=%self.images.len(), "Planner: scan complete");

        Ok(())
    }

    pub fn finalize(mut self) -> (Vec<crate::mover::Operation>, Cache) {
        info!("Planner: finalizing plan");

        let mut ops = vec![];

        finalize_videos(self.videos, &self.config, &mut self.cache, &mut ops);
        finalize_images(self.images, &self.config, &mut ops);

        // let grouped: HashMap<String, Vec<PathBuf>> =
        //     self.videos.into_iter().fold(HashMap::new(), |mut acc, v| {
        //         let d = ffprobe_duration(&v).unwrap_or(0.0);
        //         let key = format!("{:.prec$}", d, prec = self.config.duration_precision);
        //         acc.entry(key).or_default().push(v);
        //         acc
        //     });

        // for (bucket, files) in grouped {
        //     info!(bucket=%bucket, count=%files.len(), "Processing bucket");

        //     if files.len() == 1 {
        //         ops.push(crate::mover::Operation::Singleton(files[0].clone()));
        //         continue;
        //     }

        //     // Collect fingerprints with file paths and durations for caching
        //     let fps_with_meta: Vec<_> = files
        //         .par_iter()
        //         .map(|f| {
        //             let d = ffprobe_duration(f).unwrap_or(0.0);
        //             let fp = fingerprint_video(f, d);
        //             (f.clone(), d, fp)
        //         })
        //         .collect();

        //     // Update cache with newly computed fingerprints
        //     for (path, duration, fp) in &fps_with_meta {
        //         if let Some(fp) = fp
        //             && let Ok(_) = update_cache_fingerprint(
        //                 &mut self.cache,
        //                 path.clone(),
        //                 *duration,
        //                 crate::fingerprint::hash_to_hex(&fp.q25),
        //                 crate::fingerprint::hash_to_hex(&fp.q50),
        //                 crate::fingerprint::hash_to_hex(&fp.q75),
        //             )
        //         {
        //             info!(path=%path.display(), "Cached fingerprint");
        //         }
        //     }

        //     // Extract just the fingerprints for clustering
        //     let fps: Vec<Option<crate::fingerprint::VideoFingerprint>> =
        //         fps_with_meta.iter().map(|(_, _, fp)| fp.clone()).collect();

        //     let mut clusters: Vec<(Vec<PathBuf>, crate::fingerprint::VideoFingerprint)> = vec![];

        //     for (i, f) in files.iter().enumerate() {
        //         let fp = match &fps[i] {
        //             Some(x) => x,
        //             None => continue,
        //         };

        //         let mut placed = false;

        //         for (cluster, rep_fp) in &mut clusters {
        //             if distance(fp, rep_fp) <= self.config.hash_threshold {
        //                 cluster.push(f.clone());
        //                 placed = true;
        //                 break;
        //             }
        //         }

        //         if !placed {
        //             clusters.push((vec![f.clone()], fp.clone()));
        //         }
        //     }

        //     for (cluster, rep_fp) in clusters.into_iter() {
        //         let hash_repr = crate::fingerprint::hash_to_hex(&rep_fp.q50)[0..12].to_string();
        //         info!(bucket=%bucket, hash=%hash_repr, cluster_size=%cluster.len(), "Found cluster");

        //         if cluster.len() == 1 {
        //             ops.push(crate::mover::Operation::Singleton(cluster[0].clone()));
        //         } else {
        //             let dir = format!("video_{}_{}", bucket, hash_repr);
        //             ops.push(crate::mover::Operation::Cluster(dir, cluster));
        //         }
        //     }
        // }

        // for img in self.images {
        //     if let Some(image) = image::ImageReader::open(&img)
        //         .ok()
        //         .and_then(|reader| reader.decode().ok())
        //     {
        //         if image.width() * image.height() < self.config.min_image_pixels {
        //             ops.push(crate::mover::Operation::Delete(img));
        //             continue;
        //         }
        //     }

        //     ops.push(crate::mover::Operation::Image(img));
        // }

        info!(ops=%ops.len(), "Planner: finalize complete");

        (ops, self.cache)
    }
}

#[instrument(skip(files, config, ops))]
fn finalize_images(files: Vec<PathBuf>, config: &Args, ops: &mut Vec<crate::mover::Operation>) {
        let mut uf = UnionFind::new(files.len());

    debug!(?files, "images");
    let images = files.par_iter()
    .filter_map(|path| {
        let image = image::ImageReader::open(path)
        .inspect_err(|e| error!(?e, ?path, "failed to open"))
        .ok()?
        .with_guessed_format()
        .inspect_err(|e| error!(?e, ?path, "failed to deduce format"))
        .ok()?
        .decode()
        .inspect_err(|e| error!(?e, ?path, "failed to decode"))
        .ok()?;
        let fp = crate::media::image::fingerprint::generate(image)
        .inspect_err(|e| error!(?e, ?path, "failed to fingerprint"))
        .ok()?;
        Some( crate::media::image::fingerprint::ImageRecord {
            path: path.clone(),
            width: 0,
            height: 0,
            fingerprint: fp,
        })
    }).collect::<Vec<_>>();
    let mut bktree = crate::bktree::BkTree::new();
    for (id, image) in images.iter().enumerate() {
        bktree.insert(image.fingerprint, id);
    }
    let max_distance = 15;
    for (id, image) in images.iter().enumerate() {
                let mut matches = Vec::new();

        bktree.search(&image.fingerprint, max_distance, &mut matches);
        debug!(?image, ?matches, "mktree match");
for &other_idx in matches {
    if id == other_idx {
        continue;
    }

    let d = images[id]
        .fingerprint
        .distance(&images[other_idx].fingerprint);

    if d <= max_distance {
        uf.union(id, other_idx);
    }
}    }
    debug!(?uf, "union find");
        let mut groups: HashMap<usize, Vec<usize>> =
        HashMap::new();

    for idx in 0..images.len() {
        let root = uf.find(idx);
        groups
            .entry(root)
            .or_default()
            .push(idx);
    }

    let groups = groups.into_values()
    .filter(|g| g.len() > 1)
    .map(|ids| ids.iter().map(|id| images[*id].path.clone()).collect::<Vec<_>>())
    .collect::<Vec<_>>();

    for (id, group) in groups.iter().enumerate() {
        info!(id, ?group, "group");
    }
}

#[instrument(skip(videos, config, cache, ops))]
fn finalize_videos(
    videos: Vec<PathBuf>,
    config: &Args,
    cache: &mut HashMap<PathBuf, crate::cache::CachedVideo>,
    ops: &mut Vec<crate::mover::Operation>,
) {
    let grouped: HashMap<String, Vec<PathBuf>> =
        videos.into_iter().fold(HashMap::new(), |mut acc, v| {
            let d = ffprobe_duration(&v).unwrap_or(0.0);
            let key = format!("{:.prec$}", d, prec = config.duration_precision);
            acc.entry(key).or_default().push(v);
            acc
        });

    for (bucket, files) in grouped {
        info!(bucket=%bucket, count=%files.len(), "Processing bucket");

        if files.len() == 1 {
            ops.push(crate::mover::Operation::Singleton(files[0].clone()));
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
                    cache,
                    path.clone(),
                    *duration,
                    fp.q25.to_hex(),
                    fp.q50.to_hex(),
                    fp.q75.to_hex(),
                )
            {
                info!(path=%path.display(), "Cached fingerprint");
            }
        }

        // Extract just the fingerprints for clustering
        let fps: Vec<Option<crate::media::video::fingerprint::VideoFingerprint>> =
            fps_with_meta.iter().map(|(_, _, fp)| fp.clone()).collect();

        let mut clusters: Vec<(Vec<PathBuf>, crate::media::video::fingerprint::VideoFingerprint)> = vec![];

        for (i, f) in files.iter().enumerate() {
            let fp = match &fps[i] {
                Some(x) => x,
                None => continue,
            };

            let mut placed = false;

            for (cluster, rep_fp) in &mut clusters {
                if distance(fp, rep_fp) <= config.hash_threshold {
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
            let hash_repr = rep_fp.q50.to_hex()[0..12].to_string();
            info!(bucket=%bucket, hash=%hash_repr, cluster_size=%cluster.len(), "Found cluster");

            if cluster.len() == 1 {
                ops.push(crate::mover::Operation::Singleton(cluster[0].clone()));
            } else {
                let dir = format!("video_{}_{}", bucket, hash_repr);
                ops.push(crate::mover::Operation::Cluster { target: dir, files: cluster });
            }
        }
    }
}
