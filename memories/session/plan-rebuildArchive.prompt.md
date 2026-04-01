# Plan: Rebuild Archive - Consolidate Snapshots by URL

## TL;DR
**Scope**: Workspace-wide migration to replace ExtractedPage with HistoricalPage as the canonical archive type. HistoricalPage consolidates all snapshots for a given URL into a single historical record, eliminating the current hash-sharded single-snapshot design.

**Initial Phase**: Build an archive rebuilding tool that reads all existing ExtractedPage files (hash-sharded), consolidates snapshots by URL, merges multi-page articles (identified by ?page params) within the same fetch date, deduplicates links across all history, and outputs HistoricalPage format.

**Recommended approach**: Parse all archive files into an in-memory map keyed by (domain, url_without_page_param), merging pages by date, then serialize to HistoricalPage format preserving all original snapshot data. Establish compatibility layer to support both formats during workspace migration.

## Design Decisions
- **Output structure**: New JSON format `HistoricalPage` with `{url, historical_snapshots: [...], all_links: [...]}` preserving all original ExtractedPage fields within snapshots. **This replaces ExtractedPage as the canonical archive unit.**
- **Archive format**: Moving from hash-sharded single-snapshot files to consolidated-by-URL files. New format: `archive/{domain}/{path}/{url_hash}.json` (not `{hash}___{date}.json`).
- **Multi-page merging**: Combine all ?page=X URLs into a single snapshot per date, merging content_markdown and combining links (page order preserved in merged markdown)
- **Link deduplication**: Per-URL across all dates (deduplicate the final `all_links` list)
- **Compatibility layer**: During migration, support reading both ExtractedPage and HistoricalPage formats. Use feature flags or adapter traits.
- **Purpose**: Enable workspace-wide consolidation of archive access patterns, simplifying retrieval, indexing, and analysis by URL instead of by snapshot date

## Implementation: 3-Phase Migration Strategy

### Current Status
**✓ Phase 1 & 1.5 Complete** (2026-04-01)
- HistoricalPage and HistoricalSnapshot types fully implemented with optimizations
- all_links: HashSet<String> with custom serializers for deterministic sorted JSON output
- HistoricalSnapshot::links field marked with #[serde(skip_serializing)] to avoid redundancy
- PageReader trait created with implementations for both ExtractedPage and HistoricalPage
- 27 unit tests passing (6 historical, 8 page, 13 other)
- Entire workspace compiles cleanly with no errors or warnings
- Ready for Phase 2: Rebuild tool development

### Phase 0: Decision Gates & Migration Path ✓ DECIDED
**Decisions Made:**

0a. **Crawler behavior: DIRECT GENERATION**
   - web_archiver will generate HistoricalPages directly at crawl time
   - Consolidation happens post-fetch as part of the store pipeline
   - *Implication*: More complex crawler logic, but simplifies downstream (no offline rebuild needed for new crawls)

0b. **Archive migration: CLEAN BREAK (Full migration)**
   - All existing ExtractedPage archives will be migrated to HistoricalPage format
   - Old hash-sharded structure → new consolidated-by-URL structure
   - *Implication*: Simpler codebase post-migration, one-time storage cost, no legacy format support needed

0c. **Compatibility strategy: ADAPTER TRAIT PATTERN**
   - Create `PageReader` trait implemented by both ExtractedPage and HistoricalPage
   - Crates will use trait bounds instead of direct types
   - *Implication*: More elegant than feature flags, enables parallel testing, requires refactoring all consumers

0d. **Crate migration order: SEQUENTIAL (read-only first)**
   - Sequence: archive_indexer → vector_indexer → search → web_archiver
   - Each step validates new format before moving on
   - web_archiver (crawler) migrates last since it changes the write path
   - *Implication*: Lower risk, clear validation gates, but slower overall timeline

### Phase 1: Data Model Design ✓ COMPLETE

1. ✓ Created `common/src/historical.rs` with:
   - `HistoricalPage` struct: url, historical_snapshots (sorted by fetch_time), all_links (HashSet<String>)
   - `HistoricalSnapshot` struct: wraps task, content_markdown, links (Vec, skip_serializing), metadata from ExtractedPage
   - Methods: `add_snapshot()` (auto-deduplicates links via HashSet), `consolidate_links()` (rebuilds from scratch if needed), `write_page()` (pretty JSON)
   - Conversion: `ExtractedPage → HistoricalSnapshot` via `from_extracted_page()` and `From` trait impl
   - Custom serialization: `serialize_sorted_links()` outputs sorted JSON array from HashSet, `deserialize_links()` reconstructs HashSet
   - Serialization optimization: HistoricalSnapshot::links marked with #[serde(skip_serializing)] to avoid JSON redundancy
   - All tests passing (6 unit tests for HistoricalPage/Snapshot behavior)

2. ✓ Created `common/src/page.rs` with:
   - `PageReader` trait: url(), snapshots(), all_links() → Vec<String> (sorted), fetch_time(), latest_fetch_time()
   - Implemented for **both** ExtractedPage and HistoricalPage with proper trait semantics
   - Helper trait `ExtractedPageExt` with `as_snapshots()` for compatibility
   - HistoricalPage impl returns sorted links from HashSet; ExtractedPage impl returns cloned single snapshot
   - All tests passing (8 unit tests for both trait implementations + serialization validation)

3. ✓ Updated `common/src/lib.rs`:
   - Added `pub mod historical;` and `pub mod page;` exports
   - Both modules now available to all crates in workspace

### Phase 1.5: Compatibility & Format Support ✓ COMPLETE

1a. ✓ Created `historical.rs` module with HistoricalPage and HistoricalSnapshot types
   - HistoricalPage: struct with url, historical_snapshots (Vec sorted by fetch_time), all_links (HashSet<String>)
   - HistoricalSnapshot: struct wrapping task, content_markdown, links (Vec, skip_serializing), metadata
   - Serializable with serde (Serialize, Deserialize) with custom serializers for links
   - JSON format preserves all original data except redundant snapshot links field (skip_serializing)

1b. ✓ Implemented format conversion utilities in `historical.rs`:
   - `HistoricalSnapshot::from_extracted_page(page)` — converts ExtractedPage to HistoricalSnapshot
   - `impl From<ExtractedPage> for HistoricalSnapshot` — trait impl
   - `HistoricalPage::add_snapshot()` — auto-deduplicates links into HashSet during add
   - `HistoricalPage::consolidate_links()` — rebuilds HashSet from scratch if needed
   - Conversion tested and validated (27 tests passing)

1c. ✓ Updated `lib.rs` to export historical and page modules
   - Both modules now available to all crates in workspace

1d. ✓ Implemented PageReader trait for both types
   - Enables crate migration without concrete type dependencies
   - Ready for Phase 3 sequential crate migrations

### Phase 2: Offline Rebuild Tool (rebuild_archive binary) — IN PROGRESS

**Objective**: Walk existing archive, consolidate by URL, merge multi-page snapshots, output HistoricalPage format

**Current Status**: Phase 2a-d substantially complete with 25 unit tests passing

2a. ✓ **Build `ArchiveReader` struct** (COMPLETE):
   - Module: rebuild_archive/src/archive_reader.rs
   - Takes archive root path and output root path as constructor parameters
   - Walks directory tree using `WalkDir` with same_file_system filter
   - Reads ExtractedPage files and deserializes them gracefully
   - `read_all_pages()` returns Vec<(PathBuf, Result<ExtractedPage, String>)> with error handling
   - 2 unit tests for creation and stats management

2b-c. ✓ **URL normalization and in-memory aggregation** (COMPLETE):
   - Modules: rebuild_archive/src/url_utils.rs and rebuild_archive/src/aggregator.rs
   - normalize_url_for_merge() + extract_page_number() for URL processing
   - ArchiveAggregator with HashMap<AggregateKey, Vec<PageEntry>>
   - Main loop: read pages → aggregate by (domain, normalized_url) → track page numbers
   - 17 tests (8 url_utils + 6 aggregator + 1 settings + 2 archive_reader)

2d. ✓ **Multi-page merging** (COMPLETE):
   - Module: rebuild_archive/src/multi_page_merger.rs
   - merge_pages_by_date() groups pages by fetch_time and merges within date groups
   - Sorts pages by page number (ascending) for deterministic order
   - Concatenates content with separators and page markers (e.g., "## Page 2")
   - Deduplicates links while preserving order
   - Returns MergedSnapshot with base_page, merged_content, merged_links, page_count
   - 6 new tests for merging logic, sorting, deduplication, metadata preservation
   - Main loop now orchestrates aggregation → merging with detailed logging

**Test Status**: 25 tests passing (2 archive_reader + 8 url_utils + 6 aggregator + 6 multi_page_merger + 1 settings + 2 others)

2b. ✓ **Implement URL normalization** (COMPLETE):
   - Function `normalize_url_for_merge(url: &str) -> String` removes ?page=X, ?offset=Y, ?p=X, etc.
   - Function `extract_page_number(url: &str) -> Option<u32>` extracts page number if present
   - Filters common pagination params: page, p, offset, start, begin, idx, begin_idx, from, _start, _skip, limit, pn
   - Uses url::Url for robust parsing and manipulation
   - 8 unit tests for URL normalization (removing various params, preserving others, edge cases)
   - Status: COMPLETE

2c. ✓ **Build in-memory aggregation** (COMPLETE):
   - Type: `HashMap<AggregateKey, Vec<PageEntry>>` where AggregateKey = (domain, normalized_url)
   - PageEntry wraps (ExtractedPage, Option<u32>) to track page number during multi-page merging
   - ArchiveAggregator struct to manage the HashMap with add_page(), unique_urls(), total_pages()
   - Main loop aggregates pages as they're read from archive
   - 11 unit tests for aggregation (grouping, separation, page extraction)
   - Status: COMPLETE

2d. ✓ **Implement multi-page merging per (normalized_url, fetch_date)** (COMPLETE):
   - Function `merge_pages_by_date(pages: &[PageEntry]) -> HashMap<u64, MergedSnapshot>`
   - Groups pages by fetch_time, sorts by page number (or URL if no page number)
   - Concatenates content with clear separators and page markers ("## Page N")
   - Deduplicates links while preserving order (HashSet to track seen, Vec for output)
   - Returns MergedSnapshot with base_page, merged_content, merged_links, page_count
   - 6 unit tests for single/multi-page merging, link deduplication, sorting, metadata preservation
   - Status: COMPLETE

2e. **Aggregate all links per URL**:
   - Collect all links across all snapshots for each URL
   - Deduplicate (use HashSet, maintain insertion order or sort)
   - *depends on 2d*

### Phase 2.5: Output Serialization

2f. **Determine output path mapping**:
   - Design new filename convention: `/archive/{domain}/{path}/{url_hash}.json` (replacing hash-sharded design)
   - Reuse existing `Archiver` trait from common to generate consistent output paths
   - Ensure output structure mirrors input archive (domain/path) but uses HistoricalPage format

2g. **Serialize and persist**:
   - Use serde_json to serialize HistoricalPage to JSON
   - Create output directory structure if needed (mkdirs)
   - Write to file with pretty-printing for readability
   - Log warnings for any data loss or merge conflicts
   - *depends on 2f, 2e*

### Phase 2.5: Main Orchestration & Error Handling

2h. **Update `main()` in rebuild_archive**:
   - Accept CLI args: `--archive-dir <path>` `--output-dir <path>` `--format-version <1|2>` (legacy vs new)
   - Use config/figment for optional config file override
   - Instantiate ArchiveReader and run pipeline
   - Log progress with counters: files read, URLs consolidated, pages merged per URL, snapshots aggregated
   - Report summary at end (total URLs, snapshots before/after, total distinct links, merge statistics)
   - Handle deserialization errors gracefully (log warnings with filename, skip and continue)
   - Add debug flag: `--validate` to spot-check output without writing (for testing)
   - *depends on all prior steps*

### Phase 3: Crate-by-Crate Migration (Workspace Adoption)

**Phased Crate Migration (Sequential):**

3a. **Foundation: Adapter Trait Layer** \u2713 ALREADY COMPLETE (from Phase 1.5)
   - `PageReader` trait already implemented in `common/src/page.rs`
   - Implemented for **both** ExtractedPage and HistoricalPage
   - Allows crates to accept `impl PageReader` instead of concrete types
   - Trait impl tested and validated: round-trip conversions, serialization compat
   - **No additional work needed for this step**

3b. **Step 1: Migrate archive_indexer** (read-only)
   - Refactor to accept `impl PageReader` instead of `ExtractedPage`
   - Update index generation to work with HistoricalPage snapshots
   - Create mirrored test indices from ExtractedPages and HistoricalPages
   - Verify indices are identical or semantically equivalent
   - Validation gate: indexing produces same results

3c. **Step 2: Migrate vector_indexer** (read-only)
   - Refactor to accept `impl PageReader`
   - Update embedding generation to handle HistoricalPage snapshots
   - Test: embeddings from HistoricalPages ≈ from ExtractedPages (semantic equivalence)
   - Validation gate: vector indices are semantically similar

3d. **Step 3: Migrate search components** (read-only)
   - Refactor query/retrieval to work with `impl PageReader`
   - Update URL deduplication and link handling for consolidated URLs
   - Test: query results from HistoricalPages match ExtractedPages
   - Validation gate: search accuracy preserved

3e. **Step 4: Migrate web_archiver (crawler)** (write path)
   - Implement crawl pipeline to generate HistoricalPages directly:
     - Fetch single page → create ExtractedPage → consolidate into HistoricalPage
     - Handle multi-page articles during crawl: detect page params, consolidate immediately
   - Refactor store layer to write HistoricalPage instead of ExtractedPage
   - Update database schema if needed (url_id → url_hash)
   - Test: crawl→store→index→search end-to-end
   - Validation gate: full pipeline works with new format

3f. **Final: Archive Cleanup**
   - Remove ExtractedPage from codebase (no longer needed)
   - **Keep PageReader trait** — it's foundational and enables clean abstraction even after ExtractedPage is gone
   - Remove old archive files (migrated to new format)
   - Update documentation to reflect HistoricalPage as canonical format

## Verification & Testing (Phase 2 Release)

### Unit Tests (rebuild_archive)
- Test `normalize_url_for_merge()` with various query params (?page=1, ?offset=10, ?id=123, etc.)
- Test multi-page merging: verify N pages with same date merge into 1 snapshot
- Test link deduplication: verify duplicates removed while order preserved
- Test JSON serialization/deserialization roundtrip (HistoricalPage ↔ JSON)
- Test error handling: malformed JSON, missing fields, truncated files

### Integration Test (rebuild_archive)
- Create small test archive (3-4 URLs with 2-3 snapshots each, 2+ multi-page)
- Run `rebuild_archive --archive-dir test_in --output-dir test_out --validate`
- Verify output directory structure matches expected path scheme
- Spot-check JSON output:
  - `all_links` deduplicated and sorted
  - `historical_snapshots` sorted by fetch_time ascending
  - Multi-page merges show combined markdown with separators
  - No data loss (all original snapshots represented)

### Smoke Test (Phase 3 adoption)
- Run archive_indexer on test HistoricalPages, verify index generation
- Run archive_migrator: convert test ExtractedPages → HistoricalPages, verify diff
- Run vector_indexer on rebuilt archive, verify embeddings match original (approximately)

### Manual Validation (Full Archive)
- Run rebuild tool on live archive with `--validate` (no-write mode)
- Check stats: number of URLs, average snapshots per URL, consolidation ratio
- Spot-check 5-10 output files: verify sensible consolidation
- Compare file counts: expect significant reduction (multiple snapshots per URL → one file per URL)
- Verify link deduplication: spot-check that `all_links` count ≤ sum of snapshot links

## File Structure Reference

### Relevant Files
- `rebuild_archive/src/main.rs` — main entry point and orchestration (Phase 2)
- `rebuild_archive/Cargo.toml` — dependencies for serde, walkdir, etc.
- `common/src/historical.rs` — **NEW** HistoricalPage, HistoricalSnapshot, conversion utils (Phase 1.5)
- `common/src/lib.rs` — **update** to export rebuild module (Phase 1.5)
- `common/src/types.rs` — keep ExtractedPage for now (Phase 3 only for removal)
- `common/src/archiver.rs` — reference for output path generation, may need new variant for HistoricalPage paths
- `common/src/url.rs` — use `canonicalize_url()` and `hash_url()` for URL normalization
- `.github/copilot-instructions.md` — **update** to document Phase 0 decisions once made

### Current Archive Structure
```
archive/
└── {domain}/
    ├── {path-segments}/
    │   ├── {hash_0-2}/
    │   │   └── {hash_2-4}/
    │   │       └── {hash}___{YYYY-MM}.json
```

### Archive Content Format (ExtractedPage)
```json
{
  "task": {
    "url_id": number,
    "url": string,
    "depth": number,
    "priority": number,
    "discovered_from": null | string
  },
  "content_markdown": string,
  "links": [string],
  "metadata": {
    "status_code": number,
    "content_type": null | string,
    "fetch_time": number (unix timestamp),
    "title": string,
    "document_metadata": []
  }
}
```

## Scope Clarification

### ✓ Phase 1 Foundation (COMPLETE)
- Created HistoricalPage and HistoricalSnapshot data structures
- Created PageReader trait for abstraction over both types
- All compilation and tests passing

### Phase 2 Release (Rebuild Tool) - IN PROGRESS
- ✓ Phase 2a: ArchiveReader struct for reading hash-sharded archive (COMPLETE)
- ✓ Phase 2b: URL normalization (remove pagination params) (COMPLETE)
- ✓ Phase 2c: In-memory aggregation (HashMap by domain+normalized_url) (COMPLETE)
- ✓ Phase 2d: Multi-page merging (consolidate same URL/date into single snapshot) (COMPLETE)
- Phase 2e: Link deduplication and serialization to HistoricalPage
- Phase 2h: CLI orchestration and validation
- **One-time offline tool for archive migration**
- **25 tests passing** (2 archive_reader + 8 url_utils + 6 aggregator + 6 merger + 3 other)

### Phase 3+ (Workspace Migration - Sequential) - NOT STARTED
- ~~Create `PageReader` adapter trait (Phase 3a)~~ — **trait already created in Phase 1.5** ✓
- Migrate archive_indexer to use PageReader (Phase 3b)
- Migrate vector_indexer to use PageReader (Phase 3c)
- Migrate search components to use PageReader (Phase 3d)
- Migrate web_archiver crawler to generate HistoricalPages directly (Phase 3e)
- Retire old ExtractedPage format and code (Phase 3f)

**Decision: No feature flags; use adapter trait pattern throughout.**

**Decision: Direct crawler generation—web_archiver will consolidate during crawl, not offline.**

**Decision: Full archive migration to HistoricalPage—no support for mixed formats.**

## Future Considerations (Beyond Scope)
- Compression of archived snapshots (gzip if space-critical)
- Temporal analysis queries (change frequency, link churn, etc.)
- Link validation/status checking (fetch date vs current link validity)
- Incremental rebuilds (only rebuild changed/new URLs)
- Archive deduplication (shared content across versions)
