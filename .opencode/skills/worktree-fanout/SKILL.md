---
name: Worktree Fanout
description: Create git worktrees for parallel feature branches
---

## What it does

Automates `jj` (Rust git-evolution) + `git worktree` patterns for:
- Per-feature branches that don't require `git checkout`
- Safe switching between WIP states
- Colocated worktrees via `jj git init --colocate`

Useful when the agent needs to:
- Try a risky refactor in a separate worktree
- A/B test two implementations
- Keep a "stable" worktree for edits while experimenting

## Workflow

### 1. Initialize jj colocated (once per repo)

```bash
cd /path/to/repo
jj git init --colocate
# This creates .jj/ directory alongside .git/
```

### 2. Create a fanout worktree

```bash
opencode2 run 'Use the worktree-fanout skill --feature auth'
# Creates: jj feature/auth  (new branch, new worktree at ./feature/auth)
#         jj git worktree add ./feature/auth feature/auth
```

### 3. Switch between worktrees

```bash
# List all fanout worktrees
jj worktrees list

# Switch to a different feature
jj worktrees select feature/api

# Return to main
jj worktrees select main
```

### 4. Destroy when done

```bash
jj worktrees drop feature/auth
# Cleans up the worktree directory and branch refs
```

## Config

Default worktree base directory can be set in `opencode.jsonc`:

```json
"worktree": {
  "directory": "../worktrees"
}
```

Or override per-skill:

```
opencode2 run 'Use the worktree-fanout skill --base /Users/organic/work/AI/LAC/worktrees'
```

## Safety

- Worktrees are always created as **colocated** with `.jj/` for speed
- Never run `git worktree add` directly — use `jj` to avoid `.git` corruption
- `jj converge` before dropping a worktree to avoid orphan refs