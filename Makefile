# LAC — Local Agentic Coding Makefile
# Target: Apple Silicon Macs (best on high-RAM Studio) | Qwen 3.8 27B

.PHONY: all default build test status doctor route route-daemon serve-mlx serve-llama tui studio dashboard bench tune loop-init loop-list worker worker-drain daemon-install daemon-uninstall daemon-status stop ps logs config scripts bootstrap install clean help

.DEFAULT_GOAL := default

# Default mode: 24/7 Autonomous Worker
default: worker

# Binaries
LAC_BIN = ./rust-src/target/release/lac
ROUTER_BIN = ./rust-src/target/release/lac-router
TUI_BIN = ./rust-src/target/release/lac-tui

all: build

build:
	@echo "🔨 Building LAC Rust toolsuite (release)..."
	@cd rust-src && cargo build --release
	@echo "✅ Build complete."

test:
	@echo "🧪 Running Rust test suite..."
	@cd rust-src && cargo test
	@if command -v swift >/dev/null 2>&1; then \
		echo "🍎 Running Swift test suite (LAC Studio)..."; \
		cd SwiftUI && swift test; \
	fi

app:
	@echo "🍎 Packaging LAC Studio native macOS app..."
	@cd SwiftUI && ./package-app.sh

status:
	@$(LAC_BIN) status

doctor:
	@$(LAC_BIN) doctor

route:
	@echo "🌐 Starting LAC Unified Gateway on :8000..."
	@$(ROUTER_BIN)

route-daemon:
	@echo "🌐 Starting LAC Gateway in background (:8000)..."
	@$(LAC_BIN) route --daemon

serve-mlx:
	@echo "🚀 Starting MLX inference engine (Q4 native MTP on :8080)..."
	@$(LAC_BIN) serve mlx

serve-llama:
	@echo "🦙 Starting llama-server (Q8_0 quality on :8081)..."
	@$(LAC_BIN) serve llama

tui:
	@$(TUI_BIN)

studio:
	@echo "🖥️ Launching LAC Studio macOS app..."
	@$(LAC_BIN) studio

dashboard: studio


bench:
	@$(LAC_BIN) bench

tune:
	@$(LAC_BIN) tune

stop:
	@$(LAC_BIN) stop

ps:
	@$(LAC_BIN) ps

logs:
	@$(LAC_BIN) logs router

config:
	@$(LAC_BIN) config

scripts:
	@echo "🦀 Compiling standalone Rust launchers (plain rustc, no deps)..."
	@mkdir -p scripts/bin
	@for s in serve-mlx serve-llama pull-models; do \
		rustc -O scripts/$$s.rs -o scripts/bin/$$s && echo "  built scripts/bin/$$s"; \
	done

bootstrap: build
	@echo "🥾 Running idempotent machine bootstrap (Rust)..."
	@$(LAC_BIN) bootstrap

loop-init:
	@$(LAC_BIN) loop init

loop-list:
	@$(LAC_BIN) loop list

worker: | build
	@$(LAC_BIN) worker

worker-drain: | build
	@$(LAC_BIN) worker --drain

daemon-install: install
	@$(LAC_BIN) daemon install

daemon-uninstall:
	@$(LAC_BIN) daemon uninstall

daemon-status:
	@$(LAC_BIN) daemon status

install: build
	@echo "📦 Installing full LAC suite to ~/.local/bin..."
	@mkdir -p $$HOME/.local/bin
	@install -m 755 $(LAC_BIN) $$HOME/.local/bin/lac
	@install -m 755 $(ROUTER_BIN) $$HOME/.local/bin/lac-router
	@install -m 755 $(TUI_BIN) $$HOME/.local/bin/lac-tui
	@install -m 755 ./rust-src/target/release/bootstrap $$HOME/.local/bin/lac-bootstrap
	@install -m 755 ./rust-src/target/release/bootstrap $$HOME/.local/bin/bootstrap
	@install -m 755 ./rust-src/target/release/serve-mlx $$HOME/.local/bin/lac-serve-mlx
	@install -m 755 ./rust-src/target/release/serve-mlx $$HOME/.local/bin/serve-mlx
	@install -m 755 ./rust-src/target/release/serve-llama $$HOME/.local/bin/lac-serve-llama
	@install -m 755 ./rust-src/target/release/serve-llama $$HOME/.local/bin/serve-llama
	@install -m 755 ./rust-src/target/release/pull-models $$HOME/.local/bin/lac-pull-models
	@install -m 755 ./rust-src/target/release/pull-models $$HOME/.local/bin/pull-models
	@install -m 755 ./rust-src/target/release/kv-manage $$HOME/.local/bin/lac-kv-manage
	@install -m 755 ./rust-src/target/release/kv-manage $$HOME/.local/bin/kv-manage
	@echo "✅ Installed 100% pure Rust binaries to ~/.local/bin: lac, lac-router, lac-tui, serve-mlx, serve-llama, pull-models, kv-manage, bootstrap"

clean:
	@cd rust-src && cargo clean
	@rm -rf SwiftUI/.build SwiftUI/.swiftpm scripts/bin
	@echo "🧹 Clean complete."

help:
	@echo "LAC (Local Agentic Coding) Command Runner:"
	@echo "  make (or make worker) Run default 24/7 autonomous worker daemon"
	@echo "  make worker-drain   Run autonomous worker until queue is empty and exit"
	@echo "  make daemon-install Install and start 24/7 worker & gateway as macOS LaunchAgents"
	@echo "  make daemon-status  Check launchd service status"
	@echo "  make daemon-uninstall Stop and remove launchd background services"
	@echo "  make status         Display real-time telemetry (RAM, thermals, ports)"
	@echo "  make doctor         Run end-to-end diagnostic checks"
	@echo "  make route-daemon   Start unified router on :8000 (background)"
	@echo "  make serve-mlx      Start MLX engine (:8080)"
	@echo "  make serve-llama    Start llama-server (:8081)"
	@echo "  make tui            Launch terminal control center"
	@echo "  make studio         Launch LAC Studio native macOS app (⌘1-⌘5)"
	@echo "  make dashboard      Alias for make studio"
	@echo "  make bench          Run latency/speed benchmark"
	@echo "  make tune           Rank all live lanes by throughput"
	@echo "  make stop           Stop router started by lac (engines left running)"
	@echo "  make ps             List lac processes + port table"
	@echo "  make logs           Tail router log"
	@echo "  make config         Print effective configuration"
  @echo "  make scripts        Compile standalone Rust launchers (scripts/*.rs)"
  @echo "  make bootstrap      Build suite + run idempotent machine bootstrap"
  @echo "  make build          Build release binaries"
  @echo "  make install        Install binaries to ~/.local/bin"
  @echo "  make test           Run cargo unit tests"
  @echo "  make clean          Remove Rust + Swift build outputs + scripts/bin"
  @echo "  make app            Package LAC Studio macOS app"
  @echo "  make loop-init      Install Kanban queue + loop templates to ~/todo"
  @echo "  make loop-list      List available loop workflows"
