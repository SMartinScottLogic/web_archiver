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

## License

MIT

_Last updated: 2026-04-24_
