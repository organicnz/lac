# Homebrew bundle for Mac Studio M5 Ultra LAC build
# Run: brew bundle install
# Every entry verified with `brew info` / `brew info --cask` Sep 2026.
# mlx-vlm has no formula — install via: uv tool install mlx-vlm / pip install mlx-vlm

# GUI apps (casks)
cask "ghostty"              # 1.3.1, native Metal terminal
cask "zed"                  # 1.19.x, Rust+GPU editor+terminal
cask "lm-studio"            # 0.4.24, GUI model browser + local server
cask "orbstack"             # 2.x, containers (~0.3GB idle vs Docker 1.5GB)
cask "ngrok"                # localhost tunnel if ever needed

# Core dev tools (formulae)
brew "helix"                # Rust zero-config modal editor
brew "neovim"               # terminal editor
brew "fish"                 # Rust autosuggest + syntax
brew "starship"             # Rust prompt
brew "zellij"               # Rust tmux-alternative
brew "nushell"              # Rust structured shell
brew "ripgrep"              # gitignore-respecting grep
brew "fd"                   # gitignore-aware find
brew "bat"                  # cat+syntax highlight
brew "eza"                  # ls fork with icons
brew "zoxide"               # frecency cd
brew "dust"                 # intuitive du
brew "duf"                  # df with colors/table
brew "bottom"               # top/htop + GPU graphs
brew "git-delta"            # diff/syntax-highlight pager
brew "hyperfine"            # statistical benchmarking
brew "tokei"                # parallel LoC counter
brew "jujutsu"              # jj binary, Rust git evolution
brew "lazygit"              # best Git TUI
brew "uv"                   # Astral Python env (replaces pip/poetry)
brew "mise"                 # node/python version pinning
brew "rustup"               # rustup-init; toolchains via `rustup update stable`
# brew "rust"               # NOT used — toolchains via rustup instead

# Python runtimes
brew "python@3.14"          # current 2026

# LLM inference stacks
brew "ollama"               # :11434 v1 endpoint (formula = headless server)
brew "llama.cpp"            # Metal on by default, llama-server/cli
brew "mlx-lm"               # mlx_lm.server, fastest decode on Apple Silicon
# mlx-vlm has NO formula — install with: uv pip install --system mlx-vlm
