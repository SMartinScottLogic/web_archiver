use std::collections::HashMap;

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
pub struct WithTask {
    /// The fetch task metadata (url_id, url, depth, priority, discovered_from)
    pub task: FetchTask,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PageMetadata {
    pub status_code: u16,
    pub content_type: Option<String>,
    pub fetch_time: u64,
    pub title: Option<String>,
    pub document_metadata: Option<Vec<HashMap<String, String>>>,
}

#[cfg(test)]
mod tests {
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
}
