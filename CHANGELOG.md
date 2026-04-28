# Changelog

All notable changes to Invariant will be documented in this file.

This project follows [Semantic Versioning](https://semver.org/).

---

## [0.3.1] - 2026-04-27

### Fixed

- **Formatting** — fixed `cargo fmt` violations in test files that shipped with 0.3.0.

### Added

- **Expanded test coverage** — patch parser edge cases (renamed files, binary diffs, empty input, no-newline-at-EOF, quoted paths), `DiffMode` dispatch parsing, Ruby e2e test, cross-language consistency now covers Ruby and Elixir.
- **`DiffMode` enum** — extracted rev spec parsing into a testable `parse_diff_spec()` function.
- **`clean_repo_path` helper** — fixes URL normalization for remotes with trailing slashes.
- **Pre-publish gate** — `publish-crates.sh` now runs `cargo fmt --check` and `cargo test` before publishing, and polls the crates.io index instead of sleeping a fixed duration.

---

## [0.3.0] - 2026-04-27

### Added

- **Git-native diff analysis** — `invariant diff` now works directly with the git index and history. Supports staged changes (default), single commit (`HEAD~1`), and range diffs (`main..HEAD`) without needing explicit file paths.
- **Patch and stdin input** — `invariant diff --patch file.patch` and `git diff | invariant diff --stdin` allow diff analysis from patch files or piped output.
- **Multi-file diff output** — when a diff spans multiple files, each is analyzed independently with per-file alignment scores and a color-coded summary.
- **File filter** — `invariant diff -f src/auth.rs` scopes analysis to specific paths within a larger diff.
- **`invariant review` command** — one-shot workflow that lenses changed files, diff-analyzes them against a goal, and runs queries (defaults to `orphans` + `test_gaps`) in a single invocation.
- **`git` module** (`invariant-core`) — repo discovery, stable `owner/repo` identifier resolution from remote URLs (SSH, HTTPS, ssh://), HEAD commit detection, and staged/commit/range diff extraction via `git2`.
- **`patch` module** (`invariant-core`) — unified diff parser that reconstructs before/after file content from `git diff` output or `.patch` files. Handles added, deleted, modified, and renamed files with quoted path support.

### Changed

- **Repo identity** — `detect_repo_context` now normalizes remote URLs to `owner/repo` format instead of using the workdir folder name, producing stable identifiers across clones.
- **`diff` command interface** — the former `--before` / `--after` file mode is preserved as legacy but no longer the default. Git-native mode requires no file paths.
- **`git2` dependency** — moved from CLI-only to `invariant-core` to support the new `git` and `patch` modules.

---

## [0.2.2] - 2026-03-23

### Changed

- **DG identity bootstrap compatibility** — `invariant init` now aligns with the updated DataGrout / Conduit server-scoped DG identity bootstrap flow used by MCP server URLs.
- **Bearer-token fallback** — when DG mTLS bootstrap is unavailable, Invariant can validate and persist a bearer token fallback so subsequent runs still authenticate automatically.
- **Connection lifecycle** — bridge initialization and status reporting now reflect both DG-issued mTLS identities and saved bearer-token fallback configuration.
- **Documentation** — README now documents the bootstrap fallback behavior and how subsequent runs reuse the saved authentication state.

---

## [0.2.0] - 2026-03-19

### Added

- **Ruby language support** — full tree-sitter-based parsing and analysis for `.rb`, `.rake`, and `.gemspec` files. Extracts classes, modules, methods (instance and singleton), `require`/`require_relative` dependencies, `include`/`extend`/`prepend` mixins, and method call graphs.
- **Ruby visibility tracking** — correctly propagates `private`, `protected`, and `public` block-level modifiers to subsequent method definitions. Also supports the `private :method_name` form for per-method overrides.

### Changed

- Invariant now supports 8 languages: Python, Rust, TypeScript, TSX, JavaScript, Go, Elixir, and Ruby.

---

## [0.1.0] - 2026-03-02

Initial public release.

### Core

- **Tree-sitter-powered structural extraction** — fast, local AST parsing for Python, Rust, TypeScript, TSX, JavaScript, Go, and Elixir.
- **Prolog fact generation** — extracts `module`, `function`, `function_line`, `function_visibility`, `depends_on`, `calls_external` facts from source code.
- **SHA-256 checksumming** — deterministic content hashing for cache invalidation and change detection.

### CLI

- **`invariant init`** — bootstrap mTLS identity and connect to a DataGrout server.
- **`invariant lens [paths...]`** — scan and extract structural facts from code files with language filtering, ignore pattern support, and optional local-only mode.
- **`invariant query <query>`** — run semantic queries via DataGrout Invariant (orphans, test_gaps, intent_mismatches, dependency_cycles, security_concerns, hotspots, summary).
- **`invariant diff`** — analyze code changes for goal alignment.
- **`invariant status`** — show connection, identity, and configuration status.

### Bridge

- **Conduit SDK integration** — mTLS-authenticated uploads to DataGrout Invariant for server-side semantic enrichment.
- **Auto-discovery** — identity cascade from override dir → env vars → `~/.conduit/` → `.conduit/`.
