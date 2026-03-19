# Invariant

**Semantic code analysis for the AI era**

[![CI](https://github.com/datagrout/invariant/actions/workflows/ci.yml/badge.svg)](https://github.com/datagrout/invariant/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/invariant-cli.svg)](https://crates.io/crates/invariant-cli)
[![crates.io](https://img.shields.io/crates/v/invariant-core.svg?label=invariant-core)](https://crates.io/crates/invariant-core)
[![license](https://img.shields.io/badge/license-Elastic--2.0-blue.svg)](./LICENSE)

Invariant is a fast, multi-language code analysis tool that extracts structural facts from your codebase and connects to [DataGrout](https://datagrout.ai) for semantic analysis. It enables:

- **Consequential analysis** — What breaks if we merge this PR?
- **Semantic time machine** — Query code properties across commits
- **Agent feedback loop** — Real-time validation for AI-generated code
- **Architectural governance** — Encode and enforce team invariants

**[Read the paper](https://labs.datagrout.ai/papers/consequential_analysis)** · **[Library & docs](https://library.datagrout.ai)** · **[Live demo](https://app.datagrout.ai/showcase/invariant)**

## Quick Start

```bash
# Build
cargo build --release

# Initialize (bootstraps mTLS identity automatically)
invariant init --url https://gateway.datagrout.ai/servers/{uuid}/mcp --token <your-token>

# Analyze your codebase (extracts structural facts + uploads to Prism)
invariant lens

# Query for issues (via DataGrout Prism)
invariant query orphans
invariant query test_gaps
invariant query intent_mismatches

# Analyze a diff against a stated goal
invariant diff --before old.py --after new.py --goal "add rate limiting"
```

After the first `invariant init`, no token or API key is needed again — the mTLS identity is persisted to `~/.conduit/` and auto-discovered on subsequent runs.

## How It Works

```
┌───────────────────────────────────────┐
│  Invariant (local)                    │
│                                       │
│  tree-sitter AST → structural facts   │
│  file scanning, ignore patterns       │
└──────────────┬────────────────────────┘
               │ Conduit SDK (mTLS)
               ↓
┌───────────────────────────────────────┐
│  DataGrout Prism (server)             │
│                                       │
│  semantic enrichment (LLM)            │
│  consequence queries                  │
│  intent analysis                      │
│  cross-repo aggregation               │
└───────────────────────────────────────┘
```

Invariant performs fast, local structural extraction using tree-sitter. It understands Python, Rust, TypeScript, TSX, JavaScript, Go, and Elixir ASTs natively. Facts are uploaded to DataGrout Prism via the Conduit SDK for server-side semantic enrichment — LLM-powered intent classification, security analysis, and consequence reasoning.

## Installation

```bash
# From source
cargo install --path invariant-cli

# Or run directly
cargo run -p invariant-cli -- lens src/
```

## Commands

### `invariant init`

Initialize Invariant for the current repository. Bootstraps an mTLS identity on first run.

```bash
invariant init --url https://gateway.datagrout.ai/servers/{uuid}/mcp --token <token>
```

### `invariant lens [paths...]`

Extract structural facts from code files.

```bash
invariant lens                    # Analyze entire repo
invariant lens src/ lib/          # Analyze specific directories
invariant lens --language python  # Filter by language
invariant lens --language elixir  # Elixir support
invariant lens --local-only       # Extract without uploading
invariant -v lens                 # Verbose output (debug logging)
```

### `invariant query <query>`

Run semantic queries via DataGrout Prism.

```bash
invariant query orphans              # Functions never called
invariant query test_gaps            # Untested public functions
invariant query intent_mismatches    # Functions whose behavior doesn't match their name
invariant query dependency_cycles    # Circular dependencies
invariant query security_concerns   # Potential security issues
invariant query hotspots             # High-complexity, high-change-rate functions
invariant query summary              # Aggregate statistics
```

### `invariant diff`

Analyze code changes for goal alignment.

```bash
invariant diff --before v1.py --after v2.py --goal "add user auth"
```

### `invariant status`

Show connection, identity, and configuration status.

## Supported Languages

| Language   | Parser     | Extensions       |
|------------|------------|------------------|
| Python     | tree-sitter| `.py`, `.pyw`    |
| Rust       | tree-sitter| `.rs`            |
| TypeScript | tree-sitter| `.ts`            |
| TSX        | tree-sitter| `.tsx`            |
| JavaScript | tree-sitter| `.js`, `.jsx`    |
| Go         | tree-sitter| `.go`            |
| Elixir     | tree-sitter| `.ex`, `.exs`    |

## Architecture

```
invariant/
├── invariant-core/    # Library: parser, analyzer, facts, bridge
└── invariant-cli/     # CLI binary
```

- **invariant-core** — Tree-sitter parsing, AST fact extraction, Conduit SDK bridge
- **invariant-cli** — Command-line interface with auto-enrollment, config persistence

## Development

```bash
cargo test                    # Run tests
cargo clippy                  # Lint
cargo build --release         # Build release binary
cargo doc --open              # API documentation
```

## Powered By

[DataGrout AI](https://datagrout.ai) — Intelligence Infrastructure for Autonomous Systems

## License

[Elastic License 2.0](./LICENSE)
