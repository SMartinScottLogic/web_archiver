use std::{
    fs::{File, create_dir_all, read_to_string},
    path::Path,
};

use anyhow::Result;
use archive_indexer::{Args, run};
use common::types::{FetchTask, WithTask};
use itertools::Itertools as _;
use tempfile::TempDir;

fn create_archive(archive_root: &Path, files: &[WithTask]) -> Result<()> {
    create_dir_all(archive_root)?;
    for (id, file_data) in files.iter().enumerate() {
        let name = archive_root.join(format!("file_{id}.json"));
        println!("wrote {:?} to {:?}", file_data, name);
        let file = File::create_new(name)?;
        serde_json::to_writer(file, file_data)?;
    }
    Ok(())
}

fn task(id: i64, url: &str) -> WithTask {
    WithTask {
        task: FetchTask {
            article_id: id,
            url_id: id,
            url: url.into(),
            depth: 0,
            priority: common::types::Priority::Normal,
            discovered_from: None,
        },
    }
}

#[test]
fn processes_files() -> Result<()> {
    // Create a directory inside of `env::temp_dir()`
    let tmp_dir = TempDir::new()?;
    let test_dir = tmp_dir.path().join("processes_files");
    let archive_root = test_dir.join("archive");
    create_archive(
        &archive_root,
        &[
            task(1, "https://example.com/article2"),
            task(2, "https://example.com/article1"),
            task(3, "https://example.com/article3"),
        ],
    )?;

    let csv_filename = test_dir.join("test.csv").to_str().unwrap().to_string();

    let args = Args {
        archive_root: archive_root.to_str().unwrap().to_string(),
        output_csv: csv_filename.clone(),
    };
    run(args)?;

    let binding = read_to_string(&csv_filename).unwrap();
    let data = binding
        .lines()
        .skip(1) // Skip header line
        .filter_map(|line| line.split('\t').collect_tuple::<(_, _, _)>())
        .map(|(a, b, c)| {
            (
                a.split('/').next_back().unwrap(),
                b.split('/').next_back().unwrap(),
                c.split('/').next_back().unwrap(),
            )
        })
        .inspect(|data| println!("{data:?}"))
        .collect::<Vec<_>>();

    assert_eq!(3, data.len());

    assert!(data.contains(&("file_0.json", "article2", "")));
    assert!(data.contains(&("file_1.json", "article1", "")));
    assert!(data.contains(&("file_2.json", "article3", "")));

    Ok(())
}
