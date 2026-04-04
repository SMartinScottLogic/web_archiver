# Rust Web Archiver / Crawler

![Rust](https://img.shields.io/badge/Rust-1.70+-orange)
![Build](https://img.shields.io/badge/build-passing-brightgreen)
![License](https://img.shields.io/badge/license-MIT-blue)

A high-performance, resumable web archiver and crawler written in Rust. Designed for large-scale, rules-driven web archiving with clean Markdown extraction, robust state persistence, and downstream indexing pipelines.

---

## Table of Contents
- [Features](#features)
- [Quick Start](#quick-start)
- [Configuration](#configuration-configyaml)
- [Architecture](#architecture)
- [Indexing Pipelines](#indexing-pipelines)
- [Vector Indexing Setup](#vector-indexing-setup)
- [Extending](#extending)

---

## Features
- Async pipeline using Tokio for high concurrency
- Per-domain crawl limits and rules
- YAML-configured hosts and seed URLs
- SQLite-backed crawl state (frontier, history, deduplication)
- One JSON file per archived page (Markdown + metadata)
- Fully resumable (crash-safe)
- Structured logging with `tracing`

### Post-processing
- `archive_indexer`: CSV mapping of URL → archive file
- `vector_indexer`: Embedding pipeline → Qdrant

---

## Quick Start

### 1. Install Rust
https://rustup.rs/

### 2. Clone repo
```sh
git clone https://github.com/SMartinScottLogic/web_archiver.git
cd web_archiver
```

### 3. Configure
Edit `config.yaml`

### 4. Run crawler
```sh
cargo run --bin web_archiver --release
```

### 5. Output
```
archive/<domain>/<year>/<month>/<hash>.json
```

---

## Configuration (`config.yaml`)

```yaml
hosts:
  - name: Example
    domains:
      - www.example.com
      - blog.example.com

workers: 4

seed_urls:
  - "https://www.example.com/start"
```

---

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

## Flow

```
SQLite Frontier → Frontier Manager → Workers → Extractor
     ↑                                               ↓
     └──────────── Link ingestion / deduplication ───┘
```

- Frontier: SQLite-backed queue with atomic claims
- Workers: Concurrent fetchers
- Extractor: HTML → Markdown + links
- Storage: JSON archive files

---

## Indexing Pipelines

### Archive Indexer
```sh
cargo run --bin archive_indexer --release
```

### Vector Indexer
```sh
cargo run --bin vector_indexer --release
```

### Vector Pipeline Architecture

```
JSON Archive
     │
     ▼
Chunker (Markdown → segments)
     │
     ▼
Embedding Model (ONNX Runtime)
     │
     ▼
Vector Store (Qdrant)
```

Pipeline steps:
1. Read JSON archive
2. Chunk Markdown
3. Generate embeddings
4. Store vectors

---

## Vector Indexing Setup

### Install ONNX Runtime
```bash
VERSION=1.24.3 \
wget https://github.com/microsoft/onnxruntime/releases/download/v$VERSION/onnxruntime-linux-x64-$VERSION.tgz \
tar xf onnxruntime-linux-x64-$VERSION.tgz \
sudo cp onnxruntime-linux-x64-$VERSION/lib/libonnxruntime.so /usr/local/lib/ \
sudo ldconfig
```

If needed:
```bash
echo "/usr/local/lib" | sudo tee /etc/ld.so.conf.d/local.conf
sudo ldconfig
LD_LIBRARY_PATH=/usr/local/lib:$LD_LIBRARY_PATH
```

### Run Qdrant
```bash
docker run -p 6333:6333 -p 6334:6334 qdrant/qdrant
```

---
## TODO
- Command line parsing logic made consistent across apps
- Review command line help
- Recrawl logic
- Retry failures
- Schema cleanup and consistency checks
- Reword directory layout - too many in files in {YEAR}/{MONTH} folder [I think this is buried in the rambling planning chat]
- Understand the ChatGPT download json layout

## Extending
- Domain-specific extraction rules
- Crawl trap detection
- Advanced politeness strategies
- Hybrid search (keyword + vector)

---

## License
MIT (add LICENSE file)

---

_Last updated: 2026-03-17_
