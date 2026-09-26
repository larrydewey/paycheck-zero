.PHONY: run test test-rust test-e2e lint release

run:            ## Start the server on http://127.0.0.1:8080 (SQLite file ./paycheckzero.db)
	cargo run -p paycheckzero-web

test: lint test-rust test-e2e   ## Everything: clippy, unit/integration tests, Playwright

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test-rust:
	cargo test --workspace

test-e2e:
	cd e2e && npm ci --no-audit --no-fund && npx playwright install chromium firefox webkit && npm test

release:
	cargo build --release -p paycheckzero-web
