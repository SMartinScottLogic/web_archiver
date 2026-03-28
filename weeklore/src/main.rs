mod chunk;
mod fetch;
mod llm;
mod models;
mod pipeline;
mod report;

use std::{
    fs::File,
    io::{BufRead as _, BufReader},
};

use clap::Parser;
use llm::LlmClient;

use crate::models::ProcessedArticle;
#[derive(Parser, Debug)]
#[clap(
    name = "weeklore",
    version = "0.1.1",
    about = "Analyse a weekly url list, produce an executive summary"
)]
struct Args {
    /// Host to use for LLM access
    #[clap(short, long, default_value = "http://localhost:11434")]
    llm_host: String,

    /// Name for output markdown file
    #[clap(short, long, default_value = "report.md", value_name = "OUTPUT")]
    output: String,

    /// File containing urls for analysis
    #[clap(short, long)]
    url_file: Option<String>,

    /// Urls to analyse
    #[clap(value_name = "URL")]
    urls: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let llm = LlmClient::new(&args.llm_host);

    let chunk_prompt = include_str!("prompts/chunk.txt");
    let reduce_prompt = include_str!("prompts/reduce.txt");
    let classify_prompt = include_str!("prompts/classify_suggest.txt");
    let exec_prompt = include_str!("prompts/executive_brief.txt");

    let mut results = Vec::new();

    for url in &args.urls {
        if let Some(r) = process_url(&llm, url, chunk_prompt, reduce_prompt, classify_prompt).await
        {
            results.push(r)
        }
    }

    if let Some(filename) = args.url_file
        && let Ok(file) = File::open(filename)
    {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            for word in line.split_whitespace() {
                if let Some(r) =
                    process_url(&llm, word, chunk_prompt, reduce_prompt, classify_prompt).await
                {
                    results.push(r)
                }
            }
        }
    }

    let report = pipeline::build_report(&llm, results, exec_prompt).await?;

    std::fs::write(&args.output, report)?;

    println!("Report written to {}", args.output);

    Ok(())
}

async fn process_url(
    llm: &LlmClient,
    url: &str,
    chunk_prompt: &str,
    reduce_prompt: &str,
    classify_prompt: &str,
) -> Option<ProcessedArticle> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        None
    } else {
        match pipeline::process_url(llm, url, chunk_prompt, reduce_prompt, classify_prompt).await {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("Failed to process url '{}': {:?}", url, e);
                None
            }
        }
    }
}
