## Typical Workflow

1. Ingest data
   - web_archiver
     - produces: archive/
     - reads: Internet

2. Build indexes
   - archive_indexer
     - produces: CSV file (url -> filename)
     - reads: archive/
   - vector_indexer
     - populates: vector database
     - reads: archive/

3. Query
   - hybrid_search
     - produces: Articles matching user query
     - reads: vector database
