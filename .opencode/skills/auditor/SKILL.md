---
name: Auditor
description: Static code audit — correctness, patterns, test coverage
---

## What it does

Runs a structured audit of the codebase and returns a markdown report covering:

1. **Syntax / compilation** — does the project compile / load (`cargo test`, `swift test`)?
2. **Idiom compliance** — project-specific patterns (e.g. SwiftUI `@State`, `@MainActor`, Liquid Glass tokens, Rust `Result<T,E>`, std-only zero-dep backend)
3. **Missing tests** — files without corresponding unit test coverage
4. **Common antipatterns** — force-unwrap `!`, `DispatchQueue.main.async` in Swift, unhandled `unwrap()` in Rust, non-Rust scripts in backend
5. **Import hygiene** — unused imports, circular dependencies
6. **Agent edit log** — summary of recent OpenCode tool calls (if snapshots enabled)

## Workflow

```bash
opencode2 run 'Use the auditor skill'
```

The skill will:

1. **Detect project language** from file extensions in cwd (Swift and Rust)
2. **Run language-appropriate checks**:
   - Swift (LAC Studio): `swift build`, `swift test`
   - Rust (Backend Suite): `cargo test --manifest-path rust-src/Cargo.toml`, `cargo clippy`
   - General: `git diff --stat` vs last commit, `tokei` LoC counts
3. **Scan for antipatterns** using regex + language-specific rules
4. **Output** `AUDIT_REPORT.md` at project root

## Output format (`AUDIT_REPORT.md`)

```markdown
# Code Audit — <timestamp>

## Overview
- Language(s): Swift (Client: LAC Studio), Rust (Backend Suite: std-only)
- Total files: 128
- Lines of code: 42,310 (via tokei)
- Last commit: abc123 "feat: login flow"

## Antipatterns Found
| # | File | Pattern | Severity |
|---|------|---------|----------|
| 1 | Sources/App.swift | `forceUnwrap!` on line 44 | high |
| 2 | rust-src/src/lac.rs | unhandled `unwrap()` on line 120 | medium |

## Missing Tests
| Directory | Files | % with tests |
|---|---|---|
| SwiftUI/Sources/ | 9 | 100% |
| rust-src/src/ | 8 | 100% |

## Recommendations
1. Replace `x!` with `if let x = x { ... }` or `guard let`
2. Keep backend 100% pure Rust (`std` only, zero external crates)
3. Enforce Apple Liquid Glass tokens on all SwiftUI views

## Recent Agent Edits (last 7 days)
- `/set-api-key` — added local Ollama provider config
- `local-verify` skill run — confirmed model stack reachable
- `index-embeddings` skill run — built codebase vector index
```

## Config

Customize antipatterns per language in `opencode.jsonc`:

```json
"agents": {
  "auditor": {
    "system": "Focus on Swift correctness, Liquid Glass HIG compliance, and Rust std idioms.",
    "permissions": [{ "action": "edit", "resource": "*", "effect": "deny" }]
  }
}
```

Or add custom regex patterns via skill frontmatter `metadata`: