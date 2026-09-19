# Rust Web Archiver / Crawler

![Rust](https://img.shields.io/badge/Rust-1.70+-orange)
![Build](https://img.shields.io/badge/build-passing-brightgreen)
![License](https://img.shields.io/badge/license-MIT-blue)

A high-performance, modular web archiver and hybrid search system written in Rust. It supports large-scale, rules-driven crawling, clean Markdown extraction, and downstream indexing pipelines including vector search.

---

## Table of Contents
- [Overview](#overview)
- [Features](#features)
- [Workspace Structure](#workspace-structure)
- [Quick Start](#quick-start)
- [Configuration](#configuration-configyaml)
- [Architecture](#architecture)
- [Pipeline Workflow](#pipeline-workflow)
- [Indexing Pipelines](#indexing-pipelines)
- [Vector Indexing Setup](#vector-indexing-setup)
- [Extending](#extending)
- [TODO](#todo)
- [License](#license)

---

## Overview

This project provides an end-to-end pipeline for:
- Crawling and archiving web content
- Persisting structured data
- Building keyword and vector indexes
- Performing hybrid (semantic + keyword) search

It is composed of multiple binaries and shared libraries, enabling flexible and scalable workflows.

---

## Features

### Core Crawling
- Async pipeline using Tokio for high concurrency
- Per-domain crawl limits and rules
- YAML-configured hosts and seed URLs
- SQLite-backed crawl state (frontier, deduplication, history)
- Fully resumable and crash-safe
- Structured logging with `tracing`

### Storage Format
- One JSON file per archived article (Markdown + metadata)
- Multi-page support within a single file

### Indexing & Search
- CSV-based archive indexing
- Embedding pipeline with vector database support
- Hybrid search (keyword + vector similarity)

---

## Workspace Structure

| Crate            | Type   | Purpose                              | Depends On             |
|------------------|--------|--------------------------------------|------------------------|
| web_archiver     | binary | Fetch and store web content          | common                 |
| archive_indexer  | binary | Build CSV index of archive           | common                 |
| vector_indexer   | binary | Populate vector DB from archive      | common, vector_common  |
| hybrid_search    | binary | Perform keyword + vector search      | common, vector_common  |
| legacy_converter | binary | Convert legacy data formats          | common                 |
| common           | lib    | Shared utilities                     | -                      |
| vector_common    | lib    | Shared vector/embedding logic        | -                      |

---

## Quick Start

### 1. Install Rust
https://rustup.rs/

### 2. Clone repo
git clone https://github.com/SMartinScottLogic/web_archiver.git
cd web_archiver

### 3. Configure
Edit `config.yaml`

### 4. Run crawler
cargo run --bin web_archiver --release

### 5. Output
archive/<domain>/<url_path>.json

---

## Configuration (`config.yaml`)

hosts:
  - name: Example
    domains:
      - www.example.com
      - blog.example.com

workers: 4

seed_urls:
  - "https://www.example.com/start"

---

## Architecture

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

---

## Pipeline Workflow

1. Ingestion → web_archiver  
2. Indexing → archive_indexer + vector_indexer  
3. Query → hybrid_search  

---

## Vector Indexing Setup

docker run -p 6333:6333 -p 6334:6334 qdrant/qdrant

---

## Chrome for scraper

google-chrome --remote-debugging-port=9222 --user-data-dir=./chrome-profile

---

## License

MIT

_Last updated: 2026-04-24_
# Throughput diagnostics

The archiver has separate stages for frontier dispatch, HTTP fetching, HTML
extraction, routing, and archive/database writes. Use the structured tracing
events to identify which stage limits throughput before increasing
`workers`.

Run comparable workloads with a release build:

```bash
RUST_LOG=web_archiver=debug cargo run --release -p web_archiver -- --workers 4
```

For CPU attribution, install [`cargo-flamegraph`](https://github.com/flamegraph-rs/flamegraph)
and record a release crawl:

```bash
cargo flamegraph --release -p web_archiver -- --workers 4
```

The resulting `flamegraph.svg` shows synchronous CPU work, such as HTML
parsing, Markdown conversion, URL canonicalization, SQLite, and filesystem
work. It does not explain time spent waiting for network responses. Use the
fetch and extractor span timings together with queue capacities for that
question. Debug builds are useful for correctness but distort throughput and
CPU profiles.

Stage timings are emitted by `tracing` span close events. Set
`RUST_LOG=web_archiver=debug` to see them. They are assigned to the debug
level deliberately, so normal `INFO` operation does not emit one timing log
for every page and interfere with throughput or shutdown. Each close event reports
`time.busy` (actively executing) and `time.idle` (suspended at `.await`), which
helps distinguish CPU work from network or channel backpressure without manual
`Instant` bookkeeping. The standard formatter reports individual spans; it
does not aggregate histograms or percentiles. Add a metrics layer only when
long-running aggregate rates or latency distributions are needed.

Media persistence is split into debug-level `save_content.filesystem` and
`save_content.database` child spans. These show whether time is spent creating
directories/writing the response body or updating SQLite after the download
has completed.

The SQLite completion path additionally emits debug-level
`sqlite_connection_lock` and `sqlite_mark_complete_article` spans. A large
`sqlite_connection_lock` time indicates contention on the process-wide
connection mutex; a large `sqlite_mark_complete_article` time after the lock
has been acquired indicates SQLite query/locking or storage latency.

Frontier progress counts emit debug-level `sqlite_count_fetched_connection_lock`,
`sqlite_count_fetched`, `sqlite_count_pending_connection_lock`, and
`sqlite_count_pending` spans, so progress-query contention can be compared
with media completion updates.

For async scheduling problems, use Tokio's console instrumentation separately;
it answers task wake/sleep and poll-latency questions, while a flamegraph
answers CPU attribution. Do not infer multi-core CPU utilization from the
number of Tokio tasks: the extractor and router each currently have a single
consumer.
