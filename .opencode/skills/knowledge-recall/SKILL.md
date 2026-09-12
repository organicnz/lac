---
name: Knowledge Recall
description: Consult persistent codebase knowledge before starting new tasks
---

## What it does

Turns `index-embeddings` output into agent memory. Before starting a task, embeds the task description, cosine-matches against `/Volumes/AIModels/hf/index.json`, and injects top-3 relevant files + prior lessons into context. Gives qwen3.8-27B long-term memory across sessions without growing the KV cache.

Without this, every session starts blind — the agent re-discovers the same auth patterns, the same antipatterns, the same fixes.

## Workflow

### 1. Ensure index exists

```bash
opencode2 run 'Use the index-embeddings skill'
# Builds /Volumes/AIModels/hf/index.json + .embed.json sidecars
```

### 2. Recall before task

```bash
opencode2 run 'Use the knowledge-recall skill --task "fix timing attack in login function"'
```

The skill will:
1. Embed the task string (via Ollama embed or mlx-embed)
2. Cosine-similarity against `/Volumes/AIModels/hf/index.json`
3. Return top-3 matches with file paths + similarity scores
4. Read those files, extract relevant sections (max 2000 tokens total)
5. Prepend to agent context as `[recalled knowledge]` block
6. Also grep `/Volumes/AIModels/hf/lessons/*.md` for matching keywords, append hits

### 3. Record lesson after task (closes the loop)

```bash
opencode2 run 'Use the knowledge-recall skill --record --lesson "login timing: use constant-time compare, never early-return on user lookup"'
# Appends to /Volumes/AIModels/hf/lessons/<date>.md
```

Lesson format (`lessons/<date>.md`):
```markdown
## 2026-09-10 — login timing attack
- Pattern: early-return on unknown user leaks existence via timing
- Fix: constant-time compare + uniform delay, always hash even for unknown users
- Files: Sources/Auth/Login.swift, Tests/Auth/LoginTests.swift
- Found by: @reviewer round 1, applied round 1
```

## Config

```json
"skills": {
  "knowledge-recall": {
    "index_path": "/Volumes/AIModels/hf/index.json",
    "lessons_dir": "/Volumes/AIModels/hf/lessons",
    "top_k": 3,
    "max_recall_tokens": 2000
  }
}
```

## When to use

- **Start of every non-trivial task**: `--task "<description>"` before `opencode2 run`
- **After completing a task with reusable insight**: `--record --lesson "..."`
- **Weekly**: re-run `index-embeddings` to refresh index after code changes

## Integration

Works with:
- `index-embeddings` — recall consumes the index; record feeds lessons back
- `loop-orchestrator` — implement phase starts with `--task` recall automatically
- `auditor` — audit findings auto-recorded as lessons (antipattern → fix pairs)
- `agent-resume` — recalled files list saved in state for audit trail
