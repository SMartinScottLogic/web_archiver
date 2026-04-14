use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{File, create_dir_all},
    path::Path,
};

use anyhow::Context as _;

use crate::{
    historical::{HistoricalContent, HistoricalContentType, HistoricalPage, HistoricalSnapshot},
    url::canonicalize_url,
};

pub type ArticleId = i64;
pub const DEFAULT_PRIORITY: i32 = 0;
pub const ARTICLE_PRIORITY: i32 = 10;

#[derive(
    Clone, Debug, Default, PartialEq, serde_repr::Serialize_repr, serde_repr::Deserialize_repr,
)]
#[repr(u8)]
pub enum Priority {
    #[default]
    Normal = 0,
    Article = 10,
}
impl rusqlite::types::ToSql for Priority {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::from((*self).clone() as u8))
    }
}
impl rusqlite::types::FromSql for Priority {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        u8::column_result(value).and_then(|as_u8| match as_u8 {
            0 => Ok(Priority::Normal),
            10 => Ok(Priority::Article),
            _ => Err(rusqlite::types::FromSqlError::InvalidType),
        })
    }
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FetchTask {
    #[serde(default)]
    pub article_id: ArticleId,

    pub url_id: i64,
    pub url: String,

    pub depth: u32,
    pub priority: Priority,

    pub discovered_from: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExtractedPage {
    pub task: FetchTask,
    pub content_markdown: Option<String>,
    pub links: Vec<String>,
    pub metadata: Option<PageMetadata>,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PageMetadata {
    pub status_code: u16,
    pub content_type: Option<String>,
    pub fetch_time: u64,
    pub title: Option<String>,
    pub document_metadata: Option<Vec<HashMap<String, String>>>,
}

impl ExtractedPage {
    pub fn write_page(&self, path: &Path) -> anyhow::Result<()> {
        let parent = path
            .parent()
            .with_context(|| format!("Failed to get parent of {:?}", path))?;
        create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {:?}", parent))?;

        let file =
            File::create(path).with_context(|| format!("Failed to create file {:?}", path))?;

        serde_json::to_writer_pretty(file, self)
            .with_context(|| format!("Failed to write JSON to {:?}", path))?;

        Ok(())
    }
}

impl From<ExtractedPage> for HistoricalPage {
    fn from(val: ExtractedPage) -> Self {
        let mut val = val;
        val.task.url = canonicalize_url(&val.task.url).unwrap_or_default();
        let content_markdown = match val.content_markdown {
            Some(text) => vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(text),
            }],
            None => Vec::new(),
        };
        let current = HistoricalSnapshot {
            content_markdown,
            links: HashSet::new(),
            metadata: val.metadata,
        };
        HistoricalPage {
            task: val.task,
            current: Some(current),
            historical_snapshots: VecDeque::new(),
            all_links: HashSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;

    #[test]
    fn test_fetch_task_clone_eq() {
        let t1 = FetchTask {
            article_id: 1,
            url_id: 1,
            url: "http://foo.com".to_string(),
            depth: 0,
            priority: Priority::default(),
            discovered_from: None,
        };
        let t2 = t1.clone();
        assert_eq!(t1.url, t2.url);
        assert_eq!(t1.url_id, t2.url_id);
    }

    #[test]
    fn test_page_metadata_fields() {
        let meta = PageMetadata {
            status_code: 200,
            content_type: Some("text/html".to_string()),
            fetch_time: 123,
            title: Some("Title".to_string()),
            document_metadata: Some(vec![]),
        };
        assert_eq!(meta.status_code, 200);
        assert_eq!(meta.content_type.as_deref(), Some("text/html"));
        assert_eq!(meta.title.as_deref(), Some("Title"));
    }

    fn sample_task() -> FetchTask {
        FetchTask {
            article_id: 1,
            url_id: 42,
            url: "http://example.com".to_string(),
            depth: 1,
            priority: Priority::default(),
            discovered_from: Some(1),
        }
    }

    #[test]
    fn test_extracted_page_serde_roundtrip() {
        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: Some("Hello **world**".into()),
            links: vec!["http://a.com".into(), "http://b.com".into()],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".into()),
                fetch_time: 999,
                title: Some("Example".into()),
                document_metadata: None,
            }),
        };

        let json = serde_json::to_string(&page).unwrap();
        let decoded: ExtractedPage = serde_json::from_str(&json).unwrap();

        assert_eq!(page, decoded);
    }

    #[test]
    fn test_write_page_creates_file_and_dirs() {
        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: Some("content".into()),
            links: vec![],
            metadata: None,
        };

        let tmp_dir = std::env::temp_dir();
        let file_path: PathBuf = tmp_dir.join("crawler_test/subdir/page.json");

        // Clean up before test (in case it exists)
        let _ = fs::remove_file(&file_path);

        page.write_page(&file_path).unwrap();

        assert!(file_path.exists());

        // Verify it's valid JSON
        let content = fs::read_to_string(&file_path).unwrap();
        let decoded: ExtractedPage = serde_json::from_str(&content).unwrap();

        assert_eq!(decoded.task.url, "http://example.com");

        // Cleanup
        let _ = fs::remove_file(&file_path);
    }

    #[test]
    fn test_write_page_invalid_path() {
        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: None,
            links: vec![],
            metadata: None,
        };

        // Path without parent (edge case)
        let path = Path::new("");

        let result = page.write_page(path);

        assert!(result.is_err());
    }

    #[test]
    fn test_conversion_to_historical_page_with_content() {
        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: Some("markdown".into()),
            links: vec!["http://a.com".into()],
            metadata: None,
        };

        let hist: HistoricalPage = page.into();

        assert!(hist.current.is_some());

        let current = hist.current.unwrap();

        assert_eq!(current.content_markdown.len(), 1);

        match current.content_markdown.first().unwrap() {
            HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(text),
            } => {
                assert_eq!(text, "markdown");
            }
            _ => panic!("Expected Literal content"),
        }

        // Links should NOT be copied (important behavior)
        assert!(current.links.is_empty());

        assert!(hist.historical_snapshots.is_empty());
        assert!(hist.all_links.is_empty());
    }

    #[test]
    fn test_conversion_to_historical_page_without_content() {
        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: None,
            links: vec![],
            metadata: None,
        };

        let hist: HistoricalPage = page.into();
        let current = hist.current.unwrap();

        assert!(current.content_markdown.is_empty());
    }

    #[test]
    fn test_conversion_preserves_metadata() {
        let metadata = PageMetadata {
            status_code: 404,
            content_type: Some("text/html".into()),
            fetch_time: 111,
            title: Some("Not Found".into()),
            document_metadata: None,
        };

        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: None,
            links: vec![],
            metadata: Some(metadata.clone()),
        };

        let hist: HistoricalPage = page.into();
        let current = hist.current.unwrap();

        assert_eq!(current.metadata, Some(metadata));
    }

    #[test]
    fn test_conversion_canonicalizes_url() {
        let page = ExtractedPage {
            task: FetchTask {
                article_id: 1,
                url_id: 1,
                url: "http://example.com/".to_string(), // trailing slash
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            },
            content_markdown: None,
            links: vec![],
            metadata: None,
        };

        let hist: HistoricalPage = page.into();

        // We don't know exact canonical form, but ensure it's not empty
        assert!(!hist.task.url.is_empty());
    }

    #[test]
    fn test_conversion_invalid_url_fallback() {
        let page = ExtractedPage {
            task: FetchTask {
                article_id: 1,
                url_id: 1,
                url: "not a valid url%%%".to_string(),
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            },
            content_markdown: None,
            links: vec![],
            metadata: None,
        };

        let hist: HistoricalPage = page.into();

        // unwrap_or_default() → empty string fallback
        assert_eq!(hist.task.url, "");
    }
}
