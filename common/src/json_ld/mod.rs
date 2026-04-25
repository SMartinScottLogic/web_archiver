use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::iter::FromIterator;

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
#[serde(untagged)]
pub enum JsonLd {
    Article(Article),
    WebPage(WebPage),
    Array(Vec<JsonLd>),
    Unknown(Value),
}

impl JsonLd {
    pub fn flatten(self) -> JsonLd {
        match self {
            Self::Array(arr) => {
                let mut result = Vec::new();
                for item in arr {
                    item.collect_into(&mut result);
                }
                Self::Array(result)
            }
            _ => self,
        }
    }

    fn collect_into(self, out: &mut Vec<Self>) {
        match self {
            Self::Array(arr) => {
                for item in arr {
                    item.collect_into(out);
                }
            }
            _ => out.push(self),
        }
    }
}

impl FromIterator<JsonLd> for JsonLd {
    fn from_iter<I: IntoIterator<Item = JsonLd>>(iter: I) -> Self {
        JsonLd::Array(iter.into_iter().collect())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct Article {
    #[serde(rename = "@context")]
    pub context: Option<Value>,

    #[serde(rename = "@type")]
    pub type_field: OneOrMany<String>,

    pub headline: Option<String>,
    pub description: Option<String>,

    #[serde(rename = "datePublished")]
    pub date_published: Option<String>,

    #[serde(rename = "dateModified")]
    pub date_modified: Option<String>,

    pub author: Option<OneOrMany<Author>>,

    pub publisher: Option<Organization>,

    #[serde(rename = "mainEntityOfPage")]
    pub main_entity_of_page: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct WebPage {
    #[serde(rename = "@context")]
    pub context: Option<Value>,

    #[serde(rename = "@type")]
    pub type_field: OneOrMany<String>,

    pub name: Option<String>,
    pub description: Option<String>,

    pub url: Option<String>,

    #[serde(rename = "datePublished")]
    pub date_published: Option<String>,

    #[serde(rename = "dateModified")]
    pub date_modified: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum Author {
    Person(Person),
    Organization(Organization),
    Name(String),
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct Person {
    #[serde(rename = "@type")]
    pub type_field: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct Organization {
    #[serde(rename = "@type")]
    pub type_field: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum StringOrObject<T> {
    String(String),
    Object(T),
}

pub fn parse(json_text: &str) -> anyhow::Result<JsonLd> {
    let value: Value = serde_json::from_str(json_text)?;

    parse_value(value)
}

fn parse_value(value: Value) -> anyhow::Result<JsonLd> {
    match &value {
        Value::Object(obj) => {
            if let Some(t) = obj.get("@type") {
                let type_str = t.to_string();

                if type_str.contains("Article") {
                    let article: Article = serde_json::from_value(value)?;
                    // handle article
                    Ok(JsonLd::Article(article))
                } else if type_str.contains("WebPage") {
                    let page: WebPage = serde_json::from_value(value)?;
                    // handle webpage
                    Ok(JsonLd::WebPage(page))
                } else {
                    Ok(JsonLd::Unknown(value))
                }
            } else {
                Ok(JsonLd::Unknown(value))
            }
        }
        Value::Array(arr) => arr
            .iter()
            .cloned()
            .map(parse_value)
            .collect::<Result<Vec<_>, _>>()
            .map(JsonLd::Array),
        _ => Ok(JsonLd::Unknown(value)),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn flatten_single_unknown() {
        let value: Value = serde_json::from_str(
            r#"{"string":"s", "num": 1, "missing": null, "obj": {"k": "v"}, "array": [1,2,3]}"#,
        )
        .unwrap();
        let json_ld = JsonLd::Unknown(value.clone());
        assert_eq!(JsonLd::Unknown(value), json_ld.flatten());
    }

    #[test]
    fn flatten_single_webpage() {
        let webpage = WebPage {
            context: Some("https://schema.org".into()),
            type_field: OneOrMany::One("WebPage".into()),
            name: Some("Example page".into()),
            description: Some("This is an example page".into()),
            url: Some("https://example.com".into()),
            date_published: Some("yesterday".into()),
            date_modified: Some("today".into()),
        };
        assert_eq!(JsonLd::WebPage(webpage.clone()), JsonLd::WebPage(webpage));
    }

    #[test]
    fn flatten_single_article() {
        let article = Article {
            context: Some("https://schema.org".into()),
            type_field: OneOrMany::One("Article".into()),
            headline: Some("title".into()),
            description: Some("description".into()),
            date_published: Some("2026-01-01T00:00:00.000Z".into()),
            date_modified: None,
            author: Some(OneOrMany::One(Author::Person(Person {
                type_field: Some("Person".into()),
                name: Some("Author Name".into()),
            }))),
            publisher: Some(Organization {
                type_field: Some("Organization".into()),
                name: Some("org".into()),
            }),
            main_entity_of_page: Some(Value::Null),
        };
        assert_eq!(JsonLd::Article(article.clone()), JsonLd::Article(article));
    }

    #[test]
    fn flatten_empty_array() {
        assert_eq!(
            JsonLd::Array(Vec::new()),
            JsonLd::Array(Vec::new()).flatten()
        );
    }

    #[test]
    fn flatten_empty_nested_arrays() {
        assert_eq!(
            JsonLd::Array(Vec::new()),
            JsonLd::Array(vec![
                JsonLd::Array(vec![]),
                JsonLd::Array(vec![JsonLd::Array(vec![]), JsonLd::Array(vec![]),]),
                JsonLd::Array(vec![]),
            ])
            .flatten()
        );
    }

    #[test]
    fn flatten_nested_arrays() {
        let value: Value = serde_json::from_str(
            r#"{"string":"s", "num": 1, "missing": null, "obj": {"k": "v"}, "array": [1,2,3]}"#,
        )
        .unwrap();

        assert_eq!(
            JsonLd::Array(vec![
                JsonLd::Unknown(value.clone()),
                JsonLd::Unknown(value.clone()),
                JsonLd::Unknown(value.clone()),
                JsonLd::Unknown(value.clone())
            ]),
            JsonLd::Array(vec![
                JsonLd::Array(vec![JsonLd::Unknown(value.clone())]),
                JsonLd::Array(vec![
                    JsonLd::Array(vec![JsonLd::Unknown(value.clone())]),
                    JsonLd::Array(vec![JsonLd::Unknown(value.clone())]),
                ]),
                JsonLd::Array(vec![JsonLd::Unknown(value.clone())]),
            ])
            .flatten()
        );
    }

    #[test]
    fn parse_webpage() {
        let webpage_str = r#"{
  "@context": "https://schema.org",
  "@type": "WebPage",
  "name": "title",
  "description": "tagline",
  "contentRating": "PG",
  "isFamilyFriendly": true,
  "audience": {
    "@type": "PeopleAudience",
    "suggestedMinAge": 10,
    "requiredMinAge": 5
  },
  "mainEntityOfPage": {
    "@type": "WebPage",
    "@id": "https://www.example.com/"
  },
  "inLanguage": "en",
  "publisher": {
    "@type": "Organization",
    "name": "Example",
    "logo": {
      "@type": "ImageObject",
      "url": "https://example.com/image.png"
    }
  }
}"#;
        let json_ld = parse(webpage_str).unwrap();
        assert!(matches!(json_ld, JsonLd::WebPage(_)));
    }

    #[test]
    fn parse_article() {
        let article_str = r#"{
  "@context": "https://schema.org",
  "@type": "Article",
  "headline": "title",
  "description": "tagline",
  "author": {
    "@type": "Person",
    "name": "Test Author",
    "url": "https://www.example.com/authors/Test",
    "image": "https://www.example.com/images/Test.png"
  },
  "datePublished": "2026-01-30T00:00:00.000Z",
  "genre": [ "Example", "Test", "Infrastructure" ],
  "keywords": [ "example", "test", "page" ],
  "contentRating": "PG",
  "isFamilyFriendly": true,
  "audience": {
    "@type": "PeopleAudience",
    "suggestedMinAge": 12,
    "requiredMinAge": 10
  },
  "mainEntityOfPage": {
    "@type": "WebPage",
    "@id": "https://www.example.com/"
  },
  "articleSection": "Test",
  "wordCount": 42,
  "inLanguage": "en",
  "aggregateRating": {
    "@type": "AggregateRating",
    "ratingValue": 4.75,
    "bestRating": 5,
    "worstRating": 1,
    "ratingCount": 12
  },
  "interactionStatistic": [
    {
      "@type": "InteractionCounter",
      "interactionType": {
        "@type": "http://schema.org/ReadAction"
      },
      "userInteractionCount": 253
    },
    {
      "@type": "InteractionCounter",
      "interactionType": {
        "@type": "http://schema.org/CommentAction"
      },
      "userInteractionCount": 19
    },
    {
      "@type": "InteractionCounter",
      "interactionType": {
        "@type": "http://schema.org/LikeAction"
      },
      "name": "Added to Favorites",
      "userInteractionCount": 3
    }
  ],
  "publisher": {
    "@type": "Organization",
    "name": "Example",
    "logo": {
      "@type": "ImageObject",
      "url": "https://www.example.com/example.png"
    }
  }
}"#;
        let json_ld = parse(article_str).unwrap();
        assert!(matches!(json_ld, JsonLd::Article(_)));
    }

    #[test]
    fn parse_string() {
        let str = r#""string""#;
        let expected = serde_json::json!("string");
        let json_ld = parse(str).unwrap();
        assert!(matches!(json_ld, JsonLd::Unknown(v) if v == expected));
    }

    #[test]
    fn parse_array() {
        let a = r#"["string"]"#;
        let expected = vec![JsonLd::Unknown(serde_json::json!("string"))];
        let json_ld = parse(a).unwrap();
        assert!(matches!(json_ld, JsonLd::Array(v) if v == expected));
    }

    #[test]
    fn parse_other_object() {
        let o = r#"{"type": "test"}"#;
        let expected = serde_json::json!({"type":"test"});
        let json_ld = parse(o).unwrap();
        println!("expected: {:?}, json_ld: {:?}", expected, json_ld);
        assert!(matches!(json_ld, JsonLd::Unknown(v) if v == expected));
    }

    #[test]
    fn parse_other_json_type() {
        let o = r#"{"@type": "Test"}"#;
        let expected = serde_json::json!({"@type":"Test"});
        let json_ld = parse(o).unwrap();
        println!("expected: {:?}, json_ld: {:?}", expected, json_ld);
        assert!(matches!(json_ld, JsonLd::Unknown(v) if v == expected));
    }
}
