.PHONY: install
install: ## Install the virtual environment and install the pre-commit hooks
	@echo "🚀 Creating virtual environment using virtualenv"
	@uv run python -m virtualenv .venv
	@uv sync --group dev
	@uv run pre-commit install

.PHONY: check-python
check: ## Run code quality tools.
	@echo "🚀 Checking lock file consistency with 'pyproject.toml'"
	@uv lock --locked
	@echo "🚀 Linting code: Running pre-commit"
	@uv run pre-commit run -a
	@echo "🚀 Static type checking: Running mypy"
	@uv run mypy
	@echo "🚀 Checking for obsolete dependencies: Running deptry"
	@uv run deptry .

.PHONY: check-rust
check-rust: ## Run Rust specific checks
	@echo "🚀 Checking Rust formatting"
	@cargo fmt --all -- --check
	@echo "🚀 Running Rust clippy checks"
	@cargo clippy --all-targets --all-features -- -D warnings

.PHONY: check
check: check-python check-rust

.PHONY: build
build: clean-build ## Build wheel file
	@echo "🚀 Creating wheel file"
	@uvx maturin build --release -m py-ih-muse/Cargo.toml --out dist

.PHONY: develop
develop: ## Build and install the package in develop mode
	@echo "🚀 Building and installing package in develop mode"
	@uv run maturin develop --release -m py-ih-muse/Cargo.toml

.PHONY: test
test: develop ## Test the code with pytest
	@echo "🚀 Testing code: Running pytest"
	@uv run pytest --cov --cov-config=pyproject.toml --cov-report=xml

.PHONY: clean-build
clean-build: ## Clean build artifacts
	@echo "🚀 Removing build artifacts"
	@uv run python -c "import shutil; import os; shutil.rmtree('dist', ignore_errors=True); shutil.rmtree('build', ignore_errors=True); shutil.rmtree('py-ih-muse/target', ignore_errors=True);"

.PHONY: publish
publish: build ## Publish a release to PyPI.
	@echo "🚀 Publishing."
	@uvx twine upload dist/*

.PHONY: build-and-publish
build-and-publish: build publish ## Build and publish.

.PHONY: docs-test
docs-test: ## Test if documentation can be built without warnings or errors
	@uv run pip install -e ./py-ih-muse
	@uv run mkdocs build -s

.PHONY: docs
docs: ## Build and serve the documentation
	@uv run mkdocs serve

.PHONY: help
help:
	@uv run python -c "import re; \
	[[print(f'\033[36m{m[0]:<20}\033[0m {m[1]}') for m in re.findall(r'^([a-zA-Z_-]+):.*?## (.*)$$', open(makefile).read(), re.M)] for makefile in ('$(MAKEFILE_LIST)').strip().split()]"

.DEFAULT_GOAL := help
