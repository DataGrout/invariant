# Changelog

All notable changes to Invariant will be documented in this file.

This project follows [Semantic Versioning](https://semver.org/).

---

## [0.6.0] - 2026-05-27

### Added

- **Prolog support — `Language::Prolog` + `analyze_prolog`** — Invariant now lenses Prolog source (`.pl` / `.plt` / `.pro` / `.prolog`) into the same fact vocabulary as the imperative languages, powered by the MIT-licensed [`tree-sitter-prolog`](https://github.com/DataGrout/tree-sitter-grammars/tree/main/tree-sitter-prolog) grammar maintained alongside this repo. Mapping:
  - a **predicate** (name/arity) is the unit — one `function` fact per predicate, with an extra `function_line` per additional clause (predicates are deduped across their clauses);
  - **head arguments** become positional `function_param` facts (type `unknown` — Prolog is untyped);
  - **body goals** become `calls_external` facts, recursing only through the control operators `, ; -> *-> |` and treating each goal's functor (including infix goals like `=` / `is`) as the callee;
  - `:- module(Name, Exports)` drives **visibility** (exported predicates are `public`, others `private`; with no module directive everything is `public`);
  - `:- use_module(...)` / `:- ensure_loaded(...)` / `:- consult(...)` become `depends_on` facts (`library(lists)` unwraps to `lists`);
  - ProbLog annotated clauses (`0.5::heads :- toss.`) resolve to the annotated head predicate (`heads/0`).
- **Prolog tests** — `prolog_predicates_params_calls_and_visibility` and `prolog_problog_annotation_resolves_head_predicate` in `analyzer.rs`, plus `test_parse_prolog` / `test_language_prolog_from_extension` in `parser.rs`.

### Notes

- The grammar is a **superset** (ISO core + SWI dicts + ProbLog `::` + DCG). It was validated to parse **all 92** Prolog files under `data-grout` as well as [`logic-batteries`](https://github.com/DataGrout/logic-batteries) with zero error nodes. Dialect detection (ISO-conformance, SWI-only constructs) is intentionally a downstream concern, not the grammar's.

---

## [0.5.0] - 2026-05-26

### Added

- **Parameter facts — `function_param(FuncId, Position, Name, Type)`** — the analyzer now emits one fact per function parameter, deterministically from tree-sitter, so downstream consumers get full call signatures (not just arity). `Position` is zero-based; `Name` is the parameter identifier; `Type` is the declared type text, or the atom `unknown` when the language/declaration omits it. Verified across Rust, Python, JavaScript, TypeScript, Go, and Ruby. The Rust `self` / `&self` receiver is captured as a parameter named `self`. (`emit_params` / `find_params_node` / `extract_param` in `analyzer.rs`.)
- **Cross-language parameter tests** — `rust_function_params_capture_name_and_type`, `rust_self_receiver_is_captured_then_typed_params`, `python_function_params_capture_names_untyped_are_unknown`, and `cross_language_function_params` (JS / TS / Go / Ruby, including the TypeScript colon-stripping case).

### Fixed

- **Clean call-graph callees — no more multi-line garbage** — `analyze_calls` previously captured the full node text of a chained-call expression, so a callee could come out as `"Self::git_output(&[...]\n  .map"` (newlines, parens, the whole expression). Method / field / attribute / selector calls now resolve to just the method name (plus a simple receiver where unambiguous), inner calls are captured by recursion, and a final `sanitize_callee` guard collapses any residual newline / paren / whitespace. Callee atoms are now clean single tokens. (`callee_name` / `is_simple_receiver` / `sanitize_callee`, +2 tests.)
- **TypeScript parameter types no longer keep the leading colon** — a `type_annotation` exposed as `: number` is normalised to the bare type `number`.

### Downstream impact

- Consumers re-lensing after this release will see new `function_param/4` facts and cleaner `calls_external` callee atoms. Re-running `manifold lens --upload --namespace <ns>` is sufficient — the per-file lens upload retracts the previous facts and asserts the fresh ones, so cleanup is automatic. In DataGrout, `nav_func_report/2` surfaces the parameters as an ordered `params` list and `manifold read_function` renders the signature line.

### Known limitations

- **Elixir parameters are not extracted.** An Elixir `def f(a, b)` parses as a `call` node without a `parameters` / `arguments` field, so `function_param` facts are not emitted for `.ex` sources. (DataGrout lenses Elixir via its native AST path, not invariant-core, so this does not affect the DG Elixir lens.)

---

## [0.4.1] - 2026-05-25

### Fixed

- **Rust impl methods no longer collide on `func_id`** — when two structs in the same module defined a method with the same name and arity (e.g. `impl Order { fn new }` and `impl OrderItem { fn new }`), the analyzer previously emitted the same `func_id` (`new_0`) for both, silently overwriting one in any downstream Prolog fact store keyed by id. Impl method ids are now qualified with the `Self` type: `order_new_0` and `orderitem_new_0`. Free (non-impl) functions are unchanged — `fn run` still emits `run_0`.

### Added

- **Regression test for struct-qualified function ids** — `test_rust_impl_methods_are_struct_qualified` in `analyzer.rs` covers the collision case (two structs with same-name methods), the same-arity cancel-method case, the free-function preservation case, and asserts no bare unqualified id leaks out for impl methods.

### Downstream impact

- Consumers that store lens facts keyed by `func_id` (e.g. DataGrout LC namespaces uploaded via `manifold lens --upload`) will see new ids for impl methods after re-lensing. Re-running `manifold lens --upload --namespace <ns>` is sufficient — the lens upload retracts the previous `tag: "lens"` facts and asserts the fresh ones in one batch, so cleanup is automatic.

---

## [0.4.0] - 2026-05-10

### Added

- **`invariant onboard` command** — create a DataGrout account and register an agent identity directly from the CLI, no prior account or URL needed. Interactive for humans; pass `--agent` to skip prompts for CI and autonomous pipelines.
- **Auto-prompting in `invariant init`** — when no URL is configured and stdin is a terminal, `init` now asks whether to create a free DataGrout account inline and runs the full onboarding + bootstrap flow if accepted.
- **`Bridge::onboard`** (`invariant-core`) — new method that performs the two-step DG onramp handshake (registration → token exchange → mTLS identity bootstrap) and returns a ready-to-use `Bridge` and the provisioned server URL.
- **Onboarding integration tests** — `invariant-core/tests/bridge_onboard_test.rs` covers the full `Bridge::onboard` flow, gated by `DG_GATEWAY_URL` env var for CI environments with live access.

### Changed

- **Conduit SDK** updated to 0.5.0 (adds `onramp` feature — `register_and_exchange`, `OnrampOptions`).

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
