---
name: Auditor
description: Static code audit — correctness, patterns, test coverage
---

## What it does

Runs a structured audit of the codebase and returns a markdown report covering:

1. **Syntax / compilation** — does the project compile / load?
2. **Idiom compliance** — project-specific patterns (e.g. SwiftUI @State, Rust `Result<T,E>`, Python type hints)
3. **Missing tests** — files without corresponding `*test.*` or `*_test.` siblings
4. **Common antipatterns** — force-unwrap `!`, `DispatchQueue.main.async` in Swift, `unwrap()` in Rust, bare `except:` in Python
5. **Import hygiene** — unused imports, circular deps
6. **Agent edit log** — summary of recent OpenCode tool calls (if snapshots enabled)

## Workflow

```bash
opencode2 run 'Use the auditor skill'
```

The skill will:

1. **Detect project language** from file extensions in cwd
2. **Run language-appropriate checks**:
   - Swift: `swift build` (if Xcode CLT present), `swiftc -parse-as-documentation`
   - Rust: `cargo check`
   - Python: `ruff check`, `py_compile`
   - General: `git diff --stat` vs last commit, `tokei` LoC counts
3. **Scan for antipatterns** using regex + language-specific rules
4. **Output** `AUDIT_REPORT.md` at project root

## Output format (`AUDIT_REPORT.md`)

```markdown
# Code Audit — <timestamp>

## Overview
- Language(s): Swift, Rust, Python
- Total files: 128
- Lines of code: 42,310 (via tokei)
- Last commit: abc123 "feat: login flow"

## Antipatterns Found
| # | File | Pattern | Severity |
|---|------|---------|----------|
| 1 | Sources/App.swift | `forceUnwrap!` on line 44 | high |
| 2 | utils/helpers.py | bare `except:` on line 12 | medium |

## Missing Tests
| Directory | Files | % with tests |
|---|---|---|
| Sources/ | 42 | 12% |
| Tests/ | 8 | 100% |

## Recommendations
1. Replace `x!` with `x.mapError { ... }` or `if let x = x { ... }`
2. Add tests for `auth/token.swift` — currently no `*test.*` sibling
3. Remove `DispatchQueue.main.async` from non-UI path in `networking.swift`

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
    "system": "Focus on Swift correctness, Rust idioms, Python type safety.",
    "permissions": [{ "action": "edit", "resource": "*", "effect": "deny" }]
  }
}
```

Or add custom regex patterns via skill frontmatter `metadata`: