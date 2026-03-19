# Changelog

All notable changes to Invariant will be documented in this file.

This project follows [Semantic Versioning](https://semver.org/).

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
