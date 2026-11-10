export PATH := $(HOME)/.cargo/bin:/opt/homebrew/opt/rustup/bin:/usr/local/opt/rustup/bin:$(PATH)

# macOS Make 3.81 needs env to resolve tools using the exported PATH.
CARGO := env cargo

.PHONY: dev doctor check test e2e eval build smoke-macos fmt audit

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

eval:
	$(CARGO) run -p picket-cli --locked -- eval --mock

# Release signing identity. With it in the keychain, builds keep one code signature,
# so macOS privacy grants such as Full Disk Access survive rebuilds; without it they
# are signed ad hoc. See docs/setup/development.md.
SIGNING_IDENTITY := Picket Self-Signed Code Signing

build:
	@if security find-identity -p codesigning 2>/dev/null | grep -q "$(SIGNING_IDENTITY)"; then \
		APPLE_SIGNING_IDENTITY="$(SIGNING_IDENTITY)" pnpm build; \
	else \
		pnpm build; \
	fi

# Packaged lifecycle check that requires macOS Accessibility permission.
smoke-macos: build
	open -n target/release/bundle/macos/Picket.app
	command osascript scripts/smoke-macos.applescript "$(CURDIR)/target/release/picket-app"

# Dependency policy and known-vulnerability checks.
audit:
	$(CARGO) deny check
	pnpm audit --prod
