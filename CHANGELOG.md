# Changelog

All notable changes to Invariant will be documented in this file.

This project follows [Semantic Versioning](https://semver.org/).

---

## [Unreleased]

### Added

- **C#** (`.cs`, `.csx`). A type's module id is its namespace-qualified name
  with its type parameters (`Acme.Orders.Result<T>`), so the parts of a
  partial class across files are one module and `Result` and `Result<T>` are
  two. Methods, constructors (named for the type, which is what `new T()`
  calls), finalizers, operators (`op_Addition`), properties and indexers with
  bodies, event accessors, local functions and top-level statements (the
  file's `Main`) are functions; auto-properties carry no code and are left
  out. Accessibility follows C#'s defaults, `internal` included. `using`
  directives and base types are dependencies; `: base(…)` calls the base
  class's constructor. `[Fact]`, `[Theory]`, `[Test]`, `[TestMethod]` and
  `[TestCase]` mark tests, as do `*.Tests` project directories.
- **`annotation(Id, Name, Args)`** for C# attributes on types and members, so
  `[HttpGet("orders/{id}")]` routes and `[Authorize]` can be queried.
- **.NET project files** (`.csproj`, `.fsproj`, `.vbproj`, `.props`,
  `.targets`, `.sln`), read as XML rather than parsed: each project is a
  module depending on its packages (`Name@Version`, or `Name` when the
  version is an unexpanded `$(Property)`), project references, assembly
  references and target frameworks; a solution depends on its projects.

### Changed

- tree-sitter 0.25 (from 0.24). Newer grammars, C#'s among them, are built
  for language ABI 15, which 0.24 refused. Extraction for the existing
  languages is unchanged on the testbed.

### Measured

On the testbed's C# corpus (Newtonsoft.Json, OrchardCore; 7,752 files),
99.4% of files parse without errors, 29,062 functions, 14 duplicate ids
(types of one name and namespace in different assemblies); ground truth 100%
precision and recall for C# and for project files.

---

## [0.8.0] - 2026-11-10

### Changed (breaking)

- **Function ids are scoped by module.** They were bare `name_arity`, so every
  module's `new_0` or `save_1` collapsed into one node. Ids are now scoped by
  module (classes and Ruby modules by file), stay unique within a file (a line
  suffix on a rebinding), and every function carries `function_file/2`. Facts
  stored by earlier versions use the old ids; re-lens with `--full`.
- **`lens` uploads the facts it extracts, not your source.** Each file's facts
  go into the `code_<repo>` logic-cell namespace (or `--namespace`) under their
  own `lens_<path>` tag, replaced when the file's content hash changes and
  retracted when the file is gone. `.invariant/lens_state.json` records the
  hashes, so later runs skip unchanged files (`--full` re-uploads all).
  Sending source for LLM intent is now opt-in with `--enrich`. `ignore_patterns`
  apply to the walk.

### Added

- `query` and `review` read the namespace `lens` last wrote; `--namespace`
  overrides.
- A query result DataGrout returns as a preview plus a cache reference is read
  back in full (paged with `frame.slice`), not cut at the preview.
- `test(Func, '', unit|test_helper)` facts for `#[test]` functions, test-named
  functions in test files and plunit clauses, so `orphans` stops listing tests.
- `.mjs`, `.cjs`, `.mts` and `.cts` files.

### Improved extraction

Measured on a 32-repository testbed against hand-checked ground truth
(precision/recall before and after): TypeScript 100/93 to 100/100, TSX 82/41
to 100/100, JavaScript 100/55 to 100/100, Python 78/90 to 100/100, Go 82/73 to
100/100, Elixir 91/69 to 100/93, Rust 77/45 to 97/82, Ruby recall 86 to 89.

- **Rust:** items of inline `mod` blocks, trait default methods, and one
  dependency per item of a `use` tree (`use a::{b, c::d}` depends on `a.b`
  and `a.c.d`).
- **Go:** methods scoped by receiver type; type declarations are types, not
  dependencies.
- **Elixir:** guard heads, `defmacro`/`defguard`/`defdelegate`, one fact per
  multi-clause function, nested module names, one-line `do:` bodies; special
  forms are not calls.
- **JavaScript/TypeScript:** functions named by their binding, nested named
  functions, inline callbacks' calls credited to the enclosing function,
  namespaces as scopes, `new X` as a call, `require()` and side-effect
  imports.
- **Python:** nested `def`s are functions of their own.
- **Ruby:** bare method calls (a name that is not a parameter or local), and
  `include` reads only constant arguments.
- Atoms that are not plain identifiers are quoted.

### Fixed

- `init --token` no longer drops the token when `~/.conduit` holds an identity
  for another account; when that identity is refused, it connects with the
  token and saves it.

### Performance

- `lens` analyses every file first, then uploads four at a time instead of
  waiting on each file's round trips in turn.

---

## [0.7.0] - 2026-07-21

### Added

- **PHP support — `Language::Php` + `analyze_php`** — Invariant now lenses PHP source (`.php` / `.phtml`) into the standard fact vocabulary, using the official [`tree-sitter-php`](https://github.com/tree-sitter/tree-sitter-php) grammar in its **HTML-intermixed** variant (`LANGUAGE_PHP`, not `LANGUAGE_PHP_ONLY`), so WordPress-style template files — HTML with `<?php ?>` islands — lens as well as pure class files. Mapping:
  - **standalone functions** and **class methods** both emit `function` facts; method **visibility** comes from the `visibility_modifier` child (`public` / `private` / `protected`), defaulting to `public` when unmodified, per PHP semantics;
  - **classes / interfaces / traits / enums** emit `module` facts (labelled `class:Name`, `interface:Name`, …) and scope their methods, mirroring the Ruby class handling; `extends` / `implements` / trait-`use` emit `depends_on`;
  - a file-level `namespace Foo;` (no body) **re-scopes** the remaining siblings; `namespace Foo { ... }` scopes its body;
  - `use Foo\Bar;` and grouped `use Foo\{A, B};` (namespace_use_declaration) plus `include` / `include_once` / `require` / `require_once` with literal paths emit `depends_on`;
  - **calls** come from `function_call_expression` (plain / qualified name — dynamic `$callback()` calls are skipped), `member_call_expression` / `nullsafe_member_call_expression` (method name), `scoped_call_expression` (emitted as `Scope::name`), and `object_creation_expression` (constructor → class name);
  - **parameters** flow through the shared extractor with their `$names` and declared types (`formal_parameters` was already in `find_params_node`).
  - Module names use `::` separators (not PHP's `\`) so downstream suffix-on-separator name matching (e.g. manifold-nav's `nav_callee_name_matches`, which recognises `.` and `:`) works unchanged.
- **PHP tests** — `test_analyze_php_simple` (visibility incl. default-public, `$this->` / static / constructor / plain calls, use + require deps, `$param` facts), `test_analyze_php_html_template` (WordPress-style HTML+PHP file), and `test_analyze_php_namespace_and_traits` (file-level namespace re-scope, interface/trait modules, grouped use-imports) in `analyzer.rs`; `test_parse_php` / `test_parse_php_html_intermixed` / `test_language_php_from_extension` in `parser.rs`.

### Notes

- Pinned `tree-sitter-php = "0.23"` (0.23.11): the 0.24.x grammar line is generated with language ABI 15, which the workspace's `tree-sitter 0.24` runtime rejects (max ABI 14). 0.23 matches the ABI and the vintage of the other grammar crates.
- Anonymous functions and arrow functions are intentionally not emitted as `function` facts; calls inside them attribute to the enclosing named function (the common WordPress `add_action('init', function () { ... })` shape).
- DataGrout's `invariant.code_lens@1` tool accepts `language: "php"` in the same release (DG-side chunked-LLM lens path; the two lens paths stay in step).

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
