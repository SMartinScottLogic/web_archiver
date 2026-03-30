use std::path::Path;

use map_macro::hash_map;
use nom::combinator::peek;
use nom::multi::many0;
use nom::sequence::pair;
use nom::{Parser as _, combinator::recognize};
use std::collections::HashMap;

use common::types::{ExtractedPage, FetchTask, PageMetadata};

use nom::bytes::complete::tag;
use nom::character::complete::not_line_ending;
use nom::{
    IResult,
    character::complete::{line_ending, space0},
    multi::many1,
    sequence::terminated,
};
use tracing::debug;

use crate::parse_unambiguous_date;

fn parse_story(input: &str) -> IResult<&str, (String, String, HashMap<String, String>)> {
    let (input, _) = blank_lines(input)?;
    let (input, title) = title(input)?;

    let (input, _) = blank_lines(input)?;
    let (input, author) = author(input)?;

    let (input, _) = blank_lines(input)?;
    let (input, meta) = metadata(input)?;

    Ok((input, (title.to_string(), author.to_string(), meta)))
}

fn to_extracted_page(
    default_fetch_time: u64,
    (residue, (title, author, meta)): (&str, (String, String, HashMap<String, String>)),
) -> ExtractedPage {
    debug!("title: {}", title);
    debug!("author: {}", author);
    debug!("meta: {:?}", meta);
    debug!("residue");
    debug!("=======");

    let fetch_time = meta.get("Packaged").cloned().unwrap_or_default();
    let fetch_time = parse_unambiguous_date(&fetch_time).unwrap_or(default_fetch_time);

    let urls = meta
        .iter()
        .filter(|(k, _v)| k.to_lowercase().ends_with("url"))
        .map(|(_k, v)| v)
        .cloned()
        .collect();
    let story_url = meta.get("Story URL").unwrap().to_string();
    let tags = meta
        .iter()
        .filter(|(k, _v)| k.to_lowercase().ends_with("tags"))
        .map(|(_k, v)| v.to_lowercase())
        .fold(String::new(), |mut acc, s| {
            if !acc.is_empty() {
                acc.push(' '); // add a space before appending next item
            }
            acc.push_str(&s);
            acc
        });

    let url_id = 0;
    let discovered_from = None;

    let page = ExtractedPage {
        content_markdown: Some(residue.to_string()),
        metadata: Some(PageMetadata {
            status_code: 200,
            content_type: None,
            fetch_time,
            title: Some(title),
            document_metadata: Some(vec![hash_map! {"keywords".to_string() => tags}]),
        }),

        links: urls,
        task: FetchTask {
            url_id,
            url: story_url,
            depth: u32::MAX,
            priority: 0,
            discovered_from,
        },
    };
    debug!("page: {:?}", page);
    page
}

pub fn read_file(path: &Path, default_fetch_time: u64) -> anyhow::Result<ExtractedPage> {
    let file = std::fs::read_to_string(path)?
        // Normalise line endings (to UNIX format)
        .replace("\r\n", "\n");

    parse_story(&file)
    .map(|v| to_extracted_page(default_fetch_time, v))
    .map_err(|_error| anyhow::Error::msg(format!("reading {}", path.display())))
}

fn blank_line(input: &str) -> IResult<&str, &str> {
    terminated(space0, line_ending).parse(input)
}

fn blank_lines(input: &str) -> IResult<&str, Vec<&str>> {
    many1(blank_line).parse(input)
}

fn title(input: &str) -> IResult<&str, &str> {
    let (input, _) = space0(input)?;
    let (input, t) = not_line_ending(input)?;
    let (input, _) = line_ending(input)?;
    Ok((input, t.trim()))
}

fn author(input: &str) -> IResult<&str, &str> {
    let (input, _) = space0(input)?;
    let (input, _) = tag("by ")(input)?;
    let (input, name) = not_line_ending(input)?;
    let (input, _) = line_ending(input)?;
    Ok((input, name.trim()))
}

fn key(input: &str) -> IResult<&str, &str> {
    recognize(pair(
        nom::bytes::complete::take_while1(|c: char| c.is_uppercase() || c == ' '),
        nom::bytes::complete::take_while(|c: char| c.is_alphabetic() || c == ' '),
    ))
    .parse(input)
}

fn key_value_line(input: &str) -> IResult<&str, (&str, &str)> {
    let (input, key) = key(input)?;
    let (input, _) = tag(":")(input)?;
    let (input, value) = not_line_ending(input)?;
    let (input, _) = line_ending(input)?;

    Ok((input, (key.trim(), value.trim())))
}

fn continuation_line(input: &str) -> IResult<&str, &str> {
    // Only match if the next line is NOT a key
    if peek(key).parse(input).is_ok() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }

    let (input, line) = not_line_ending(input)?;
    let (input, _) = line_ending(input)?;

    Ok((input, line.trim()))
}

fn key_value_multiline(input: &str) -> IResult<&str, (String, String)> {
    let (mut input, (key, first_value)) = key_value_line(input)?;
    let mut value = first_value.to_string();

    while let Ok((next_input, line)) = continuation_line(input) {
        if !line.is_empty() {
            if !value.is_empty() {
                value.push(' ');
            }
            value.push_str(line);
        }
        input = next_input;
    }

    Ok((input, (key.to_string(), value)))
}

fn metadata(input: &str) -> IResult<&str, HashMap<String, String>> {
    let (input, pairs) = many0(key_value_multiline).parse(input)?;
    Ok((input, pairs.into_iter().collect()))
}
