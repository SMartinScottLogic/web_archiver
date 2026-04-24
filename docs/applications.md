# Workspace Overview

| Crate            | Type   | Purpose                              | Depends On             |
|------------------|--------|--------------------------------------|------------------------|
| web_archiver     | binary | Fetch and store web content          | common                 |
| archive_indexer  | binary | Build CSV index of archive           | common                 |
| vector_indexer   | binary | Populate vector DB from archive      | common, vector_common  |
| hybrid_search    | binary | Perform keyword + vector search      | common, vector_common  |
| legacy_converter | binary | Convert legacy data formats          | common                 |
| common           | lib    | Shared utilities                     | -                      |
| vector_common    | lib    | Shared vector/embedding logic        | -                      |

# Details
## web_archiver
- Type: binary
- Purpose: Fetch and store web content
- Inputs: config.yaml
- External data: Internet, constrained by host config in config 
- Outputs: archive directory
- Used in: ingestion stage
- Example:
  ```bash
  web_archiver --workers 30 --user-agent "Test Bot v0.2.1" --archive-dir archive --db archive.db
  ```

## archive_indexer
- Type: binary
- Purpose: Build url index from archived articles
- Inputs: archive directory
- Outputs: url -> filepath index
- Used in: indexing stage
- Example:
  ```bash
  archive_indexer <ARCHIVE_ROOT> <OUTPUT_CSV>
  ```

## vector_indexer
- Type: binary
- Purpose: Build archive index in vector DB
- Inputs: archive directory
- Outputs: Vector DB from document embeddings
- Used in: indexing stage
- Example:
  ```bash
  archive_indexer --archive-dir <ARCHIVE_ROOT> --collection <VECTOR_DB_COLLECTION>
  ```

## hybrid_search
- Type: binary
- Purpose: Perform hybrid search of vector DB
- Inputs: archive directory
- Outputs: Document matching query
- Used in: query stage
- Example:
  ```bash
  hybrid_search --collection <VECTOR_DB_COLLECTION> [QUERY]
  ```

## legacy_converter
- Type: binary
- Purpose: Convert archive files in legacy formats
- Inputs: 
  - legacy archive directory
  - config.yaml
- Outputs: articles in archive directory
- Used in: modernisation stage
- Example:
  ```bash
  legacy_converter --delete-source <LEGACY_ARCHIVE>
  ```

## libraries
`common`: General shared utilities
`vector_common`: utilities specifically for embeddings and vector math (distinct from general utilities in `common`)
