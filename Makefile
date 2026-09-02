.PHONY: build test clippy fmt check ext-check presets-check install uninstall help
.PHONY: docker-build docker-test docker-clippy docker-fmt docker-check docker-ext-check

# Docker image name
DOCKER_IMAGE = pipedeck-dev
DOCKER_FLAGS = --rm -v "$$(pwd)":/src -v pipedeck-cargo:/cargo

help:
	@echo "PipeDeck build targets:"
	@echo ""
	@echo "  build       - Build Rust workspace (debug, in Docker)"
	@echo "  test        - Run Rust tests (in Docker)"
	@echo "  clippy      - Run clippy linter with -D warnings (in Docker)"
	@echo "  fmt         - Check code formatting (in Docker)"
	@echo "  check       - Run all checks (build, test, clippy, fmt, presets-check)"
	@echo "  ext-check   - Check extension JavaScript syntax (in Docker)"
	@echo "  presets-check - Validate EQ preset TOML files"
	@echo ""
	@echo "  install     - Install to system (Linux only, runs build + install.sh)"
	@echo "  uninstall   - Uninstall from system (Linux only)"
	@echo ""
	@echo "  docker-build    - Rebuild the dev Docker image"
	@echo "  docker-test     - Run tests in Docker"
	@echo "  docker-clippy   - Run clippy in Docker"
	@echo "  docker-fmt      - Check formatting in Docker"
	@echo "  docker-check    - Run all docker checks"
	@echo "  docker-ext-check - Check extension in Docker"

# Build targets
build: docker-build
	docker run $(DOCKER_FLAGS) $(DOCKER_IMAGE) cargo build --workspace

test: docker-build
	docker run $(DOCKER_FLAGS) $(DOCKER_IMAGE) cargo test --workspace

clippy: docker-build
	docker run $(DOCKER_FLAGS) $(DOCKER_IMAGE) cargo clippy --workspace -- -D warnings

fmt: docker-build
	docker run $(DOCKER_FLAGS) $(DOCKER_IMAGE) cargo fmt --check

presets-check:
	@echo "Validating EQ preset TOML files..."
	@for f in presets/*.toml; do \
		if [ -f "$$f" ]; then \
			python3 -c "import tomllib,os,sys; d=tomllib.load(open('$$f','rb')); name=d.get('name','(no name)'); bands=len(d.get('band',[])); print(f'  ✓ {os.path.basename(\"$$f\"):20} {name} ({bands} band(s))')"; \
		fi; \
	done

check: build test clippy fmt presets-check
	@echo "✓ All checks passed"

ext-check: docker-build
	docker run $(DOCKER_FLAGS) $(DOCKER_IMAGE) sh -c 'cd extension && for f in *.js; do gjs -c "import(\"./$f\")" 2>&1 | head -5; done' || echo "Note: extension not yet present (phase 2)"

# Docker image build
docker-build:
	@if ! docker inspect $(DOCKER_IMAGE) >/dev/null 2>&1; then \
		echo "Building Docker image $(DOCKER_IMAGE)..."; \
		docker build -t $(DOCKER_IMAGE) dev/; \
	fi

# Linux-only installation targets
install:
	@if [ "$$(uname -s)" != "Linux" ]; then \
		echo "Error: install.sh must run on Linux (target is Ubuntu 26.04)" >&2; \
		exit 1; \
	fi
	@echo "Building PipeDeck for installation..."
	cargo build --release --workspace
	@echo "Running installation script..."
	bash install.sh

uninstall:
	@if [ "$$(uname -s)" != "Linux" ]; then \
		echo "Error: uninstall.sh must run on Linux" >&2; \
		exit 1; \
	fi
	bash uninstall.sh

# Convenience aliases for docker commands (developer-facing)
docker-test: docker-build
	docker run $(DOCKER_FLAGS) $(DOCKER_IMAGE) cargo test --workspace

docker-clippy: docker-build
	docker run $(DOCKER_FLAGS) $(DOCKER_IMAGE) cargo clippy --workspace -- -D warnings

docker-fmt: docker-build
	docker run $(DOCKER_FLAGS) $(DOCKER_IMAGE) cargo fmt --check

docker-check: docker-build docker-test docker-clippy docker-fmt
	@echo "✓ All Docker checks passed"

docker-ext-check: docker-build
	docker run $(DOCKER_FLAGS) $(DOCKER_IMAGE) sh -c 'cd extension && for f in *.js; do gjs -c "import(\"./$f\")" 2>&1 | head -5; done' || echo "Note: extension not yet present (phase 2)"
