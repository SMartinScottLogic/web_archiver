# Rust Web Archiver / Crawler

A high-performance, resumable web archiver and crawler written in Rust. Designed for large-scale, rules-driven web archiving with clean Markdown extraction and robust state persistence.

## Features
- **Async pipeline** using Tokio for high concurrency
- **Per-domain crawl limits** and rules
- **Seed URLs and allowed domains** configurable via YAML
- **Crawl state persisted in SQLite** (frontier, URLs, history, etc.)
- **One JSON file per archived page** (clean Markdown, metadata)
- **Resumable**: crash-safe, can restart from last state
- **Batch link ingestion** and deduplication
- **Configurable number of fetch workers** (CLI or config)
- **Structured logging** with `tracing`

## Quick Start

1. **Install Rust** (if not already):
   https://rustup.rs/

2. **Clone the repo:**
   ```sh
   git clone https://github.com/SMartinScottLogic/web_archiver.git
   cd web_archiver
   ```

3. **Edit `config.yaml`:**
   - Set `allowed_domains`, `seed_urls`, and `workers` as needed.

4. **Run the crawler:**
   ```sh
   cargo run --release
   # or override workers:
   cargo run --release -- --workers 8
   ```

5. **Archives** are written to `archive/<domain>/<year>/<month>/<hash>.json`

## Configuration (`config.yaml`)

```yaml
allowed_domains:
  - www.example.com
  - blog.example.com
workers: 4
seed_urls:
  - "https://www.example.com/start"
```

- `allowed_domains`: Only these domains will be crawled.
- `workers`: Default number of concurrent fetch workers (can be overridden by CLI `--workers`).
- `seed_urls`: Initial URLs to start crawling from.

## Architecture

```
SQLite Frontier
      │
      ▼
Frontier Manager
      │
      ▼
Fetch Workers
      │
      ▼
Extractor / Parser
      │
      ├── Content → JSON archive
      │
      └── Links → Link Ingestor
                  │
                  ▼
               SQLite Frontier
```

- **Frontier**: SQLite-backed queue, atomic claim for fetchers
- **Workers**: Fetch pages concurrently
- **Extractor**: Parses HTML, extracts clean Markdown, finds links
- **Storage**: Writes JSON archive files

## Indexing

A separate binary (`bins/indexer.rs`) can index the archive and produce a CSV mapping URLs to JSON files.

## Extending
- Add per-domain rules (XPath, robots.txt, etc.)
- Implement crawl trap protection
- Add more sophisticated politeness/concurrency controls
