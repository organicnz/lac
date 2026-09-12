---
name: Index Embeddings
description: Ingest codebase into local embeddings store for semantic search
---

## What it does

Builds a local vector index of the codebase using `nomic-ai/nomicembed-text` (or similar 384/768-dim model via Ollama) so that the agent can answer:

- "Find where X is initialized"
- "Show me all error-handling patterns"
- "What does this function do?" (semantic, not keyword)

Uses `ollama embed` or `mlx-embed` to generate text embeddings, stored as JSON sidecars alongside source files.

## Workflow

### 1. Ensure embedding model available

```bash
# Pull via Ollama (embed models are excluded from normal discovery,
# so we pull explicitly if supported, or use a small compatible model)
ollama pull nomic-ai/nomicembed-text

# Or via mlx:
uv tool install mlx-embed  # if available
```

### 2. Run the indexer

```bash
# From project root:
opencode2 run 'Use the index-embeddings skill'
```

The skill will:
1. Walk `**/*.{swift,md,rs,py,ts,js,go}` (configurable)
2. For each file, generate an embedding of the first 800 chars (or full file if < 200 chars)
3. Write a `.embed.json` companion: `{ "embedding": [...], "source": "path/to/file", "chars": 800 }`
4. Create `/Volumes/AIModels/hf/index.json` as the search index

### 3. Query

After indexing, ask the agent:

```
Find where the auth token is set. Use semantic search.
```

The agent will:
1. Embed the query
2. Cosine-similarity against `/Volumes/AIModels/hf/index.json`
3. Return top-3 matching `.embed.json` paths
4. Read those files and surface the matches

## Config

Adjust in `opencode.jsonc` or via env:

```json
"skills": {
  "index-embeddings": {
    "extensions": [".swift", ".md", ".rs", ".py", ".js", ".go"],
    "max_chars": 800,
    "index_path": "/Volumes/AIModels/hf/index.json"
  }
}
```