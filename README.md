# Invariant

**Semantic code analysis for the AI era**

[![CI](https://github.com/datagrout/invariant/actions/workflows/ci.yml/badge.svg)](https://github.com/datagrout/invariant/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/invariant-cli.svg?label=invariant-cli)](https://crates.io/crates/invariant-cli)
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

# New to DataGrout? Create a free account and register automatically:
invariant onboard

# Already have an account — initialize with your server URL:
invariant init --url https://gateway.datagrout.ai/servers/{uuid}/mcp --token <your-token>

# Analyze your codebase (extracts structural facts + uploads to Invariant)
invariant lens

# Query for issues (via DataGrout Invariant)
invariant query orphans
invariant query test_gaps
invariant query intent_mismatches

# Analyze changes against a stated goal (git-native)
invariant diff --goal "add rate limiting"                     # staged changes
invariant diff HEAD~1 --goal "add rate limiting"              # last commit
invariant diff main..HEAD --goal "refactor auth"              # branch diff

# One-shot review: lens + diff + query on changed files
invariant review --goal "add rate limiting"
```

After the first `invariant init`, Invariant prefers a persisted mTLS identity in `~/.conduit/`. If the gateway rejects identity bootstrap, Invariant falls back to the saved bearer token so subsequent runs still authenticate automatically.

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
│  DataGrout Invariant (server)         │
│                                       │
│  semantic enrichment (LLM)            │
│  consequence queries                  │
│  intent analysis                      │
│  cross-repo aggregation               │
└───────────────────────────────────────┘
```

Invariant performs fast, local structural extraction using tree-sitter. It understands Python, Rust, TypeScript, TSX, JavaScript, Go, Elixir, and Ruby ASTs natively. Facts are uploaded to DataGrout Invariant via the Conduit SDK for server-side semantic enrichment — LLM-powered intent classification, security analysis, and consequence reasoning.

## Installation

```bash
# From source
cargo install --path invariant-cli

# Or run directly
cargo run -p invariant-cli -- lens src/
```

## Commands

### `invariant onboard`

Create a free DataGrout account and register an agent identity — no prior account or URL needed. Runs the full onramp flow: account creation → OAuth credentials → mTLS identity bootstrap → saves config ready for `invariant lens`.

```bash
invariant onboard                          # interactive (human)
invariant onboard --agent --name my-agent  # non-interactive (CI / autonomous)
```

When `invariant init` is run with no URL configured and stdin is a terminal, it will offer to run onboarding inline.

### `invariant init`

Initialize Invariant for the current repository. Bootstraps an mTLS identity on first run when the gateway supports it, otherwise saves the bearer token for future fallback auth.

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

Run semantic queries via DataGrout Invariant.

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

Analyze code changes for goal alignment. Supports multiple input modes:

```bash
# Git-native (default — works from any git repo)
invariant diff --goal "add user auth"                     # staged changes vs HEAD
invariant diff HEAD~1 --goal "add user auth"              # last commit
invariant diff main..HEAD --goal "refactor auth"          # branch diff
invariant diff abc123 --goal "fix billing"                # specific commit

# Patch/stdin
invariant diff --patch changes.patch --goal "add auth"
git diff | invariant diff --stdin --goal "add auth"

# Legacy file mode (for non-git contexts)
invariant diff --before v1.py --after v2.py --goal "add user auth"

# Filter to specific files
invariant diff HEAD~1 --goal "add auth" -f src/auth.rs
```

### `invariant review`

One-shot workflow: lens changed files, diff-analyze them, and run queries.

```bash
invariant review --goal "add rate limiting"                # staged changes
invariant review HEAD~1 --goal "add rate limiting"         # last commit
invariant review main..HEAD --goal "refactor auth"         # branch review
invariant review --goal "fix billing" -q orphans -q test_gaps
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
| Ruby       | tree-sitter| `.rb`, `.rake`, `.gemspec` |

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
