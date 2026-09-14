---
name: Code Assistant
description: Agentic code reasoning, refactoring, test generation, and bug audits via LAC Studio & CLI
---

## What it does

The **Code Assistant** provides dedicated, autonomous systems engineering and programming capabilities:
- **Refactoring & Modernization**: Refactors functions and structs to be clean, idiomatic, memory-safe, and zero-allocation.
- **Test Generation**: Generates comprehensive unit tests with edge-case and boundary verification.
- **Bug & Vulnerability Auditing**: Finds race conditions, memory leaks, panics, deadlocks, and unsafe bounds operations.
- **Apple Silicon Optimization**: Analyzes hot paths for cache locality, SIMD vectorization, and unified memory alignment.

## Usage Modes

### 1. In LAC Studio (macOS Native App)

1. Open **LAC Studio**.
2. Click **Code Assistant** in the sidebar under `WORKSPACE` (or press `⌘2`).
3. Paste or type your code into the left **Code Canvas** (supports Rust, Swift, Python, TypeScript, Go, C++, Shell, SQL).
4. Click any quick action capsule at the top:
   - `[Explain Code]`
   - `[Refactor / Clean]`
   - `[Generate Tests]`
   - `[Audit Bugs & Safety]`
   - `[Optimize (SIMD / Cache)]`
5. Or type custom instructions in the bottom prompt bar.
6. Click **"Apply to Editor"** to immediately merge refactored code back into the source canvas with 1-click undo.

### 2. From Terminal via Pure Rust CLI (`lac code`)

Run agentic coding prompts directly from your shell without opening an editor:

```bash
# Review and refactor an existing file
lac code -f src/main.rs "Refactor to be thread-safe with zero allocations"

# Generate unit tests
lac code -f src/lib.rs "Generate comprehensive tests covering empty inputs and boundary conditions"

# Pipe code from stdin
cat rust-src/src/lac_router.rs | lac code "Audit socket error handling and ensure SIGPIPE safety"

# Direct implementation instruction
lac code "Write a lock-free multi-producer single-consumer ring buffer in Rust"
```

## Model Selection

- Default: `mlx-community/Qwen3.8-27B-4bit`
- Recommended for pure coding: `Qwen/Qwen2.5-Coder-32B-Instruct` or `mlx-community/Qwen2.5-Coder-32B-Instruct-4bit`
- Override via environment variable:
  ```bash
  export LAC_CODE_MODEL="mlx-community/Qwen2.5-Coder-32B-Instruct-4bit"
  ```
