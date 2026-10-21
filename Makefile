export PATH := $(HOME)/.cargo/bin:/opt/homebrew/opt/rustup/bin:/usr/local/opt/rustup/bin:$(PATH)

# macOS Make 3.81 needs env to resolve tools using the exported PATH.
CARGO := env cargo

.PHONY: dev doctor check test e2e build smoke-macos fmt audit

dev:
	pnpm dev

doctor:
	pnpm check:env

fmt:
	$(CARGO) fmt --all
	pnpm format

# Fast gate for formatting, linting, types, unit tests, and package builds.
check:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings
	pnpm check

test:
	$(CARGO) test --workspace --locked
	pnpm test

e2e:
	pnpm test:e2e

build:
	pnpm build

# Packaged lifecycle check that requires macOS Accessibility permission.
smoke-macos: build
	open -n target/release/bundle/macos/Picket.app
	command osascript scripts/smoke-macos.applescript "$(CURDIR)/target/release/picket-app"

# Dependency policy and known-vulnerability checks.
audit:
	$(CARGO) deny check
	pnpm audit --prod
