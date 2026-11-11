//! Code analyzer that extracts semantic facts from ASTs

use crate::facts::{normalize_id, Fact, FactValue};
use crate::parser::{node_line, node_text, Language, Parser};
use crate::types::{AnalysisResult, AnalysisSummary, Error, Result};
use sha2::{Digest, Sha256};
use tree_sitter::Node;

/// Shared context threaded through all analysis functions.
struct Ctx<'a> {
    code: &'a [u8],
    filepath: &'a str,
    commit_sha: &'a str,
    facts: &'a mut Vec<Fact>,
    summary: &'a mut AnalysisSummary,
    /// Function ids already emitted, for languages where one function is
    /// written as several clauses (Elixir).
    seen_functions: std::collections::HashSet<String>,
    /// Ids handed out by `unique_id`, for languages where one name can be
    /// bound several times in a scope.
    taken_ids: std::collections::HashSet<String>,
}

impl Ctx<'_> {
    /// `id`, or `id` with its line appended when the id is already taken in
    /// this file: the same name bound twice in one scope (a TS namespace's
    /// `from`, a helper declared in each test callback, `perform` in many
    /// object literals) otherwise merged distinct functions into one.
    fn unique_id(&mut self, id: String, line: usize) -> String {
        let id = if self.taken_ids.contains(&id) {
            normalize_id(&format!("{}_l{}", id, line))
        } else {
            id
        };
        self.taken_ids.insert(id.clone());
        id
    }
}

/// Multi-language code analyzer powered by tree-sitter.
pub struct Analyzer {
    parser: Parser,
}

impl Analyzer {
    /// Create a new analyzer
    pub fn new() -> Result<Self> {
        Ok(Self {
            parser: Parser::new()?,
        })
    }

    /// Extract Prolog facts from source code.
    pub fn lens_code(
        &mut self,
        code: &str,
        language: Language,
        filepath: &str,
        commit_sha: &str,
    ) -> Result<AnalysisResult> {
        let mut facts = Vec::new();
        let mut summary = AnalysisSummary::default();

        // Project files are XML or plain text: nothing for tree-sitter to read.
        if language == Language::MsBuild {
            let ctx = &mut Ctx {
                code: code.as_bytes(),
                filepath,
                commit_sha,
                facts: &mut facts,
                summary: &mut summary,
                seen_functions: std::collections::HashSet::new(),
                taken_ids: std::collections::HashSet::new(),
            };
            analyze_msbuild(code, ctx);
            summary.loc = code.lines().count();
            return Ok(AnalysisResult {
                facts: facts.iter().map(|f| f.to_prolog()).collect(),
                summary,
                receipt: None,
            });
        }

        let tree = self.parser.parse(code, language)?;
        let root = tree.root_node();

        let ctx = &mut Ctx {
            code: code.as_bytes(),
            filepath,
            commit_sha,
            facts: &mut facts,
            summary: &mut summary,
            seen_functions: std::collections::HashSet::new(),
            taken_ids: std::collections::HashSet::new(),
        };

        match language {
            Language::Python => analyze_python(&root, ctx)?,
            Language::Rust => analyze_rust(&root, ctx)?,
            Language::TypeScript | Language::Tsx | Language::JavaScript => {
                analyze_javascript(&root, ctx)?
            }
            Language::Go => analyze_go(&root, ctx)?,
            Language::Elixir => analyze_elixir(&root, ctx)?,
            Language::Ruby => analyze_ruby(&root, ctx)?,
            Language::Php => analyze_php(&root, ctx)?,
            Language::Prolog => analyze_prolog(&root, ctx)?,
            Language::CSharp => analyze_csharp(&root, ctx)?,
            Language::TSql => analyze_tsql(&root, ctx)?,
            Language::PowerShell => analyze_powershell(&root, ctx)?,
            Language::Java => analyze_java(&root, ctx)?,
            Language::Kotlin => analyze_kotlin(&root, ctx)?,
            Language::MsBuild => unreachable!("handled above"),
        }

        summary.loc = code.lines().count();

        let fact_strings: Vec<String> = facts.iter().map(|f| f.to_prolog()).collect();

        Ok(AnalysisResult {
            facts: fact_strings,
            summary,
            receipt: None,
        })
    }

    /// Compute SHA-256 checksum of code.
    pub fn compute_checksum(&self, code: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(code.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

// ========================================================================
// Helpers
// ========================================================================

/// An id scoped by its enclosing module or class id: `<scope>__<local>`.
/// Bare `name_arity` ids collided across files (every module's `new_0`),
/// merging unrelated functions into one node; classes are scoped by file
/// for the same reason.
fn scoped_id(scope: &str, local: &str) -> String {
    normalize_id(&format!("{}__{}", scope, local))
}

#[allow(clippy::too_many_arguments)]
fn emit_function(
    ctx: &mut Ctx<'_>,
    func_id: &str,
    module_id: &str,
    name: &str,
    arity: usize,
    visibility: &str,
    line: usize,
    fn_node: &Node<'_>,
) {
    ctx.facts.push(Fact::new(
        "function",
        vec![
            FactValue::String(func_id.to_string()),
            FactValue::String(module_id.to_string()),
            FactValue::String(name.to_string()),
            FactValue::Integer(arity as i64),
            FactValue::Atom(visibility.to_string()),
            FactValue::Integer(line as i64),
            FactValue::String(ctx.commit_sha.to_string()),
        ],
    ));

    ctx.facts.push(Fact::new(
        "function_line",
        vec![
            FactValue::String(func_id.to_string()),
            FactValue::Integer(line as i64),
        ],
    ));

    // The file directly: a method's module is its class, whose module/5 path
    // is `class:Name`, so the module join cannot locate it.
    ctx.facts.push(Fact::new(
        "function_file",
        vec![
            FactValue::String(func_id.to_string()),
            FactValue::String(ctx.filepath.to_string()),
        ],
    ));

    ctx.facts.push(Fact::new(
        "function_visibility",
        vec![
            FactValue::String(func_id.to_string()),
            FactValue::Atom(visibility.to_string()),
        ],
    ));

    emit_params(ctx, func_id, fn_node);

    if let Some(kind) = test_kind(ctx.filepath, name, fn_node, ctx.code) {
        // test(TestFuncID, TestedFuncID, Kind): what it tests isn't known here,
        // but marking it lets `orphans` stop listing every test as uncalled
        // (46 of 54 orphans in one live run were tests).
        ctx.facts.push(Fact::new(
            "test",
            vec![
                FactValue::String(func_id.to_string()),
                FactValue::String(String::new()),
                FactValue::Atom(kind.to_string()),
            ],
        ));
    }

    ctx.summary.functions += 1;
}

/// Whether a function is a test, and which kind: `unit` for a test the runner
/// calls, `test_helper` for any other function in a test file. A test is never
/// called from the code under test, so without this every test was an orphan.
fn test_kind(filepath: &str, name: &str, fn_node: &Node<'_>, code: &[u8]) -> Option<&'static str> {
    if has_test_attribute(fn_node, code) {
        return Some("unit");
    }

    // plunit: `test(Name) :- …` clauses between begin_tests and end_tests.
    if name == "test" && (filepath.ends_with(".pl") || filepath.ends_with(".plt")) {
        return Some("unit");
    }

    if !is_test_file(filepath) {
        return None;
    }

    let lower = name.to_ascii_lowercase();
    let test_named = lower.starts_with("test")
        || name.starts_with("Benchmark")
        || name.starts_with("Fuzz")
        || name.starts_with("Example");

    Some(if test_named { "unit" } else { "test_helper" })
}

/// Rust's `#[test]`, `#[tokio::test]`, `#[rstest]` and the like, on the
/// attribute items just before the function.
fn has_test_attribute(fn_node: &Node<'_>, code: &[u8]) -> bool {
    // C#: [Fact], [Theory], [Test], [TestMethod], [TestCase(...)] … are
    // attribute lists inside the method declaration.
    let mut cursor = fn_node.walk();
    for list in fn_node.children(&mut cursor) {
        if list.kind() != "attribute_list" {
            continue;
        }
        let mut lc = list.walk();
        for attr in list.named_children(&mut lc) {
            let name = attr
                .child_by_field_name("name")
                .map(|n| node_text(&n, code))
                .unwrap_or("");
            let short = name.rsplit('.').next().unwrap_or(name);
            let short = short.strip_suffix("Attribute").unwrap_or(short);
            if matches!(
                short,
                "Fact"
                    | "Theory"
                    | "Test"
                    | "TestMethod"
                    | "DataTestMethod"
                    | "TestCase"
                    | "TestCaseSource"
            ) {
                return true;
            }
        }
    }

    // Java and Kotlin: @Test and friends among the declaration's modifiers.
    if let Some(mods) = find_child_by_kind(fn_node, "modifiers") {
        let mut c = mods.walk();
        for a in mods.named_children(&mut c) {
            if !matches!(a.kind(), "marker_annotation" | "annotation") {
                continue;
            }
            let text = node_text(&a, code).trim_start_matches('@');
            let name = text.split(['(', ' ']).next().unwrap_or("");
            let short = name.rsplit('.').next().unwrap_or(name);
            if matches!(
                short,
                "Test" | "ParameterizedTest" | "RepeatedTest" | "TestFactory" | "TestTemplate"
            ) {
                return true;
            }
        }
    }

    let mut prev = fn_node.prev_named_sibling();
    while let Some(node) = prev {
        if node.kind() != "attribute_item" {
            break;
        }
        let text = node_text(&node, code);
        if text.contains("test") {
            return true;
        }
        prev = node.prev_named_sibling();
    }
    false
}

/// Test files by the conventions of the languages Invariant reads.
pub(crate) fn is_test_file(filepath: &str) -> bool {
    let path = filepath.replace('\\', "/");
    let segments: Vec<&str> = path.split('/').collect();
    let file = segments.last().copied().unwrap_or("");

    // .NET test projects are directories named like `Foo.Tests`,
    // `Foo.UnitTests` or `Foo.Test`.
    let in_test_dir = segments[..segments.len().saturating_sub(1)]
        .iter()
        .any(|s| {
            matches!(
                *s,
                "test" | "tests" | "spec" | "specs" | "__tests__" | "testing"
            ) || s.ends_with(".Tests")
                || s.ends_with(".Test")
                || s.ends_with("UnitTests")
                || s.ends_with("IntegrationTests")
        });

    let stem = file.split('.').next().unwrap_or(file);
    let test_named = file.contains(".test.")
        || file.contains(".Tests.")
        || file.contains(".spec.")
        || stem.starts_with("test_")
        || stem.ends_with("_test")
        || stem.ends_with("_tests")
        || stem.ends_with("_spec")
        || (stem.ends_with("Test") && stem.len() > 4)
        || (stem.ends_with("Tests") && stem.len() > 5)
        || stem == "conftest";

    in_test_dir || test_named
}

/// Emit `function_param(FuncId, Position, Name, Type)` for each parameter of a
/// function node.  Deterministic (tree-sitter), language-agnostic via a set of
/// common parameter-list node kinds + per-param name/type field heuristics.
/// Type is the atom `unknown` when the language/declaration omits it.
fn emit_params(ctx: &mut Ctx<'_>, func_id: &str, fn_node: &Node<'_>) {
    let Some(params) = find_params_node(fn_node) else {
        return;
    };

    let mut cursor = params.walk();
    let mut pos: i64 = 0;
    for child in params.named_children(&mut cursor) {
        match child.kind() {
            "comment" | "line_comment" | "block_comment" => continue,
            _ => {}
        }

        let (name, ty) = extract_param(&child, ctx.code);
        if name.is_empty() {
            continue;
        }

        let type_val = if ty.is_empty() {
            FactValue::Atom("unknown".to_string())
        } else {
            FactValue::String(ty)
        };

        ctx.facts.push(Fact::new(
            "function_param",
            vec![
                FactValue::String(func_id.to_string()),
                FactValue::Integer(pos),
                FactValue::String(name),
                type_val,
            ],
        ));
        pos += 1;
    }
}

/// Locate the parameter-list node for a function across languages.
fn find_params_node<'a>(fn_node: &Node<'a>) -> Option<Node<'a>> {
    fn_node
        .child_by_field_name("parameters")
        .or_else(|| fn_node.child_by_field_name("arguments"))
        .or_else(|| find_child_by_kind(fn_node, "parameters"))
        .or_else(|| find_child_by_kind(fn_node, "formal_parameters"))
        .or_else(|| find_child_by_kind(fn_node, "parameter_list"))
        .or_else(|| find_child_by_kind(fn_node, "method_parameters"))
        .or_else(|| find_child_by_kind(fn_node, "block_parameters"))
}

/// Extract a `(name, type)` pair from a single parameter node.  Heuristic and
/// defensive — falls back to the trimmed node text for the name and an empty
/// type when fields aren't present.
fn extract_param(node: &Node<'_>, code: &[u8]) -> (String, String) {
    // Rust receiver — `&self` / `self`.
    if node.kind() == "self_parameter" {
        return ("self".to_string(), String::new());
    }

    // A bare identifier parameter (untyped, e.g. Python `x`, Ruby `x`).
    if node.kind() == "identifier" {
        return (clean_param(node_text(node, code)), String::new());
    }

    let name = node
        .child_by_field_name("pattern")
        .or_else(|| node.child_by_field_name("name"))
        .map(|n| node_text(&n, code).to_string())
        .or_else(|| first_identifier_text(node, code))
        .unwrap_or_else(|| node_text(node, code).to_string());

    let ty = node
        .child_by_field_name("type")
        .or_else(|| find_child_by_kind(node, "type_annotation"))
        .map(|n| node_text(&n, code).to_string())
        .unwrap_or_default();

    // Some grammars (e.g. TypeScript) expose the type as a `type_annotation`
    // that includes the leading `:` — normalise it away so the type is the
    // bare type text (`number`, not `: number`).
    let ty = ty.trim().trim_start_matches(':').trim().to_string();

    (clean_param(&name), clean_param(&ty))
}

/// First descendant `identifier`'s text (DFS), if any.
fn first_identifier_text(node: &Node<'_>, code: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "identifier" {
            return Some(node_text(&child, code).to_string());
        }
        if let Some(found) = first_identifier_text(&child, code) {
            return Some(found);
        }
    }
    None
}

/// Collapse whitespace/newlines and cap length so a param name/type is a clean,
/// single-line token even when derived from messy source text.
fn clean_param(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|c| c == ',' || c == '(' || c == ')');
    if trimmed.len() > 80 {
        trimmed.chars().take(80).collect()
    } else {
        trimmed.to_string()
    }
}

fn emit_module(ctx: &mut Ctx<'_>, module_id: &str, module_name: &str, filepath: &str, line: usize) {
    ctx.facts.push(Fact::new(
        "module",
        vec![
            FactValue::String(module_id.to_string()),
            FactValue::String(module_name.to_string()),
            FactValue::String(filepath.to_string()),
            FactValue::Integer(line as i64),
            FactValue::String(ctx.commit_sha.to_string()),
        ],
    ));
    ctx.summary.modules += 1;
}

fn emit_dependency(ctx: &mut Ctx<'_>, module_id: &str, dep_name: &str, kind: &str, line: usize) {
    ctx.facts.push(Fact::new(
        "depends_on",
        vec![
            FactValue::String(module_id.to_string()),
            FactValue::String(dep_name.to_string()),
            FactValue::Atom(kind.to_string()),
            FactValue::Integer(line as i64),
        ],
    ));
    ctx.summary.dependencies += 1;
}

fn emit_call(ctx: &mut Ctx<'_>, caller_id: &str, called_func: &str, line: usize) {
    ctx.facts.push(Fact::new(
        "calls_external",
        vec![
            FactValue::String(caller_id.to_string()),
            FactValue::String("unknown".to_string()),
            FactValue::String(called_func.to_string()),
            FactValue::Integer(0),
            FactValue::Integer(line as i64),
        ],
    ));
    ctx.summary.calls += 1;
}

fn find_child_by_kind<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    (0..node.child_count())
        .filter_map(|i| node.child(i))
        .find(|c| c.kind() == kind)
}

fn param_count(node: &Node<'_>, field: &str) -> usize {
    node.child_by_field_name(field)
        .map(|n| n.named_child_count())
        .unwrap_or(0)
}

fn module_id_from_path(filepath: &str, ext: &str, sep: &str) -> (String, String) {
    let module_name = filepath.trim_end_matches(ext).replace(['/', '\\'], sep);
    let module_id = normalize_id(&module_name);
    (module_id, module_name)
}

// ========================================================================
// Python Analysis
// ========================================================================

fn analyze_python(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let (module_id, module_name) = module_id_from_path(ctx.filepath, ".py", ".");
    emit_module(ctx, &module_id, &module_name, ctx.filepath, 1);
    analyze_python_node(root, &module_id, ctx)
}

fn analyze_python_node(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                analyze_python_function(&child, module_id, module_id, ctx)?;
            }
            "class_definition" => {
                analyze_python_class(&child, ctx)?;
            }
            "import_statement" | "import_from_statement" => {
                analyze_python_import(&child, module_id, ctx)?;
            }
            _ => {
                analyze_python_node(&child, module_id, ctx)?;
            }
        }
    }

    Ok(())
}

/// `scope` scopes the id: the module or class, or the enclosing function for
/// a nested `def`.
fn analyze_python_function(
    node: &Node<'_>,
    module_id: &str,
    scope: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let name_node = node
        .child_by_field_name("name")
        .ok_or_else(|| Error::Parse("Function has no name".to_string()))?;
    let func_name = node_text(&name_node, ctx.code);
    let arity = param_count(node, "parameters");
    let line = node_line(node);
    // A name defined twice in one scope (conditional defs, redefinitions in
    // tests) is two functions.
    let func_id = ctx.unique_id(scoped_id(scope, &format!("{}_{}", func_name, arity)), line);

    let visibility = if func_name.starts_with("__") && func_name.ends_with("__") {
        "public"
    } else if func_name.starts_with("__") {
        "private"
    } else if func_name.starts_with('_') {
        "protected"
    } else {
        "public"
    };

    emit_function(
        ctx, &func_id, module_id, func_name, arity, visibility, line, node,
    );

    if let Some(body) = node.child_by_field_name("body") {
        analyze_python_body(&body, module_id, &func_id, ctx)?;
    }

    Ok(())
}

/// A function body: its calls, and nested `def`s as functions of their own.
/// Nested functions were never emitted and their calls were credited to the
/// outer function (django: 33,014 definitions, 31,242 emitted).
fn analyze_python_body(
    node: &Node<'_>,
    module_id: &str,
    func_id: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                analyze_python_function(&child, module_id, func_id, ctx)?;
                continue;
            }
            "class_definition" => {
                analyze_python_class(&child, ctx)?;
                continue;
            }
            "call" => {
                if let Some(f) = child.child_by_field_name("function") {
                    let callee = callee_name(&f, ctx.code);
                    if !callee.is_empty() {
                        emit_call(ctx, func_id, &callee, node_line(&child));
                    }
                }
            }
            _ => {}
        }
        analyze_python_body(&child, module_id, func_id, ctx)?;
    }

    Ok(())
}

fn analyze_python_class(node: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let name_node = node
        .child_by_field_name("name")
        .ok_or_else(|| Error::Parse("Class has no name".to_string()))?;
    let class_name = node_text(&name_node, ctx.code);
    let class_id = scoped_id(&normalize_id(ctx.filepath), class_name);

    emit_module(
        ctx,
        &class_id,
        class_name,
        &format!("class:{}", class_name),
        node_line(node),
    );

    if let Some(body) = node.child_by_field_name("body") {
        analyze_python_node(&body, &class_id, ctx)?;
    }

    Ok(())
}

fn analyze_python_import(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let text = node_text(node, ctx.code);

    if let Some(module_name) = text.strip_prefix("import ") {
        let module_name = module_name.split_whitespace().next().unwrap_or("");
        emit_dependency(ctx, module_id, module_name, "import", node_line(node));
    } else if text.starts_with("from ") {
        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.len() >= 2 {
            emit_dependency(ctx, module_id, parts[1], "import_from", node_line(node));
        }
    }

    Ok(())
}

// ========================================================================
// Rust Analysis
// ========================================================================

fn analyze_rust(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let (module_id, module_name) = module_id_from_path(ctx.filepath, ".rs", "::");
    emit_module(ctx, &module_id, &module_name, ctx.filepath, 1);

    analyze_rust_items(root, &module_id, ctx)
}

/// The items of a file, or of an inline `mod name { … }` body.
fn analyze_rust_items(container: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let mut cursor = container.walk();
    for node in container.children(&mut cursor) {
        match node.kind() {
            "function_item" => {
                analyze_rust_function(&node, module_id, None, ctx)?;
            }
            "struct_item" | "enum_item" | "trait_item" => {
                analyze_rust_type(&node, module_id, ctx)?;
            }
            "impl_item" => {
                analyze_rust_impl(&node, module_id, ctx)?;
            }
            "use_declaration" => {
                analyze_rust_use(&node, module_id, ctx)?;
            }
            "mod_item" => {
                analyze_rust_mod(&node, module_id, ctx)?;
            }
            _ => {}
        }
    }

    Ok(())
}

/// `impl_type_prefix` qualifies impl method IDs with the Self type name so that
/// `impl OrderItem { fn new }` → `orderitem_new_3` and `impl Order { fn new }` →
/// `order_new_3`, avoiding collisions when different structs define methods with
/// the same name and arity.  Pass `None` for top-level free functions.
fn analyze_rust_function(
    node: &Node<'_>,
    module_id: &str,
    impl_type_prefix: Option<&str>,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let name_node = node
        .child_by_field_name("name")
        .ok_or_else(|| Error::Parse("Function has no name".to_string()))?;
    let func_name = node_text(&name_node, ctx.code);
    let arity = param_count(node, "parameters");
    let func_id = match impl_type_prefix {
        Some(prefix) if !prefix.is_empty() => {
            scoped_id(module_id, &format!("{}_{}_{}", prefix, func_name, arity))
        }
        _ => scoped_id(module_id, &format!("{}_{}", func_name, arity)),
    };
    let line = node_line(node);

    let visibility = if has_pub_modifier(node, ctx.code) {
        "public"
    } else {
        "private"
    };

    emit_function(
        ctx, &func_id, module_id, func_name, arity, visibility, line, node,
    );

    if let Some(body) = node.child_by_field_name("body") {
        analyze_calls(&body, ctx.code, &func_id, ctx, "call_expression")?;
    }

    Ok(())
}

fn analyze_rust_type(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let type_name = node_text(&name_node, ctx.code);
        let type_id = scoped_id(module_id, type_name);
        emit_module(
            ctx,
            &type_id,
            type_name,
            &format!("type:{}", module_id),
            node_line(node),
        );

        // A trait's default methods have bodies and run like any other
        // method; they were never emitted.
        if node.kind() == "trait_item" {
            if let Some(body) = node.child_by_field_name("body") {
                let prefix = normalize_id(type_name);
                let mut cursor = body.walk();
                for child in body.children(&mut cursor) {
                    if child.kind() == "function_item"
                        && child.child_by_field_name("body").is_some()
                    {
                        analyze_rust_function(&child, module_id, Some(&prefix), ctx)?;
                    }
                }
            }
        }
    }

    Ok(())
}

fn analyze_rust_impl(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    // Extract the Self type being implemented (e.g. `Order` from `impl Order { … }`
    // or `AppState` from `impl AppState { … }`).  We use this as a prefix in the
    // func_id so that `impl OrderItem { fn new }` → `orderitem_new_3` and
    // `impl Order { fn new }` → `order_new_3`, avoiding silent collisions when
    // multiple structs define a method with the same name and arity.
    let type_prefix: Option<String> = node
        .child_by_field_name("type")
        .map(|n| normalize_id(node_text(&n, ctx.code)));

    let body = node.child_by_field_name("body").unwrap_or(*node);
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_item" {
            analyze_rust_function(&child, module_id, type_prefix.as_deref(), ctx)?;
        }
    }

    Ok(())
}

fn analyze_rust_use(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    // `use a::b::{c, d::e, self as f};` depends on a.b.c, a.b.d.e and a.b:
    // one target per item, read from the use tree. Splitting the text on
    // whitespace left targets like `regex_syntax.hir.{`.
    let Some(arg) = node.child_by_field_name("argument") else {
        return Ok(());
    };

    let mut targets = Vec::new();
    rust_use_targets(&arg, "", ctx.code, &mut targets);

    let line = node_line(node);
    for target in targets {
        if !target.is_empty() {
            emit_dependency(ctx, module_id, &target, "use", line);
        }
    }

    Ok(())
}

/// Expand a use tree into `a.b.c` targets under `prefix`.
fn rust_use_targets(node: &Node<'_>, prefix: &str, code: &[u8], out: &mut Vec<String>) {
    let join = |p: &str, s: &str| {
        let s = s.replace("::", ".");
        if p.is_empty() {
            s
        } else if s.is_empty() || s == "self" {
            p.to_string()
        } else {
            format!("{}.{}", p, s)
        }
    };

    match node.kind() {
        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .map(|p| node_text(&p, code).to_string())
                .unwrap_or_default();
            let base = join(prefix, &path);
            if let Some(list) = node.child_by_field_name("list") {
                rust_use_targets(&list, &base, code, out);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for item in node.named_children(&mut cursor) {
                rust_use_targets(&item, prefix, code, out);
            }
        }
        // `x as y`: the dependency is x.
        "use_as_clause" => {
            if let Some(path) = node.child_by_field_name("path") {
                rust_use_targets(&path, prefix, code, out);
            }
        }
        // `a::*`: the module a.
        "use_wildcard" => {
            let text = node_text(node, code);
            out.push(join(
                prefix,
                text.trim_end_matches("::*").trim_end_matches('*'),
            ));
        }
        _ => out.push(join(prefix, node_text(node, code))),
    }
}

fn analyze_rust_mod(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let Some(name_node) = node.child_by_field_name("name") else {
        return Ok(());
    };
    let mod_name = node_text(&name_node, ctx.code);

    match node.child_by_field_name("body") {
        // An inline `mod tests { … }` is code in this file: its items were
        // skipped, and the module was recorded as a dependency.
        Some(body) => {
            let sub_id = scoped_id(module_id, mod_name);
            emit_module(ctx, &sub_id, mod_name, ctx.filepath, node_line(node));
            analyze_rust_items(&body, &sub_id, ctx)
        }
        // `mod name;` declares a module in another file.
        None => {
            emit_dependency(ctx, module_id, mod_name, "mod", node_line(node));
            Ok(())
        }
    }
}

fn has_pub_modifier(node: &Node<'_>, code: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            return node_text(&child, code).starts_with("pub");
        }
    }
    false
}

// ========================================================================
// JavaScript/TypeScript Analysis
// ========================================================================

fn analyze_javascript(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let mut stem = ctx.filepath;
    for ext in [".jsx", ".tsx", ".mjs", ".cjs", ".mts", ".cts", ".js", ".ts"] {
        if let Some(s) = stem.strip_suffix(ext) {
            stem = s;
            break;
        }
    }
    let module_name = stem.replace(['/', '\\'], ".");
    let module_id = normalize_id(&module_name);
    emit_module(ctx, &module_id, &module_name, ctx.filepath, 1);

    analyze_js_node(root, &module_id, ctx)
}

/// Function-like nodes in the JS/TS grammars.
fn is_js_function(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "generator_function_declaration"
            | "method_definition"
            | "arrow_function"
            | "function_expression"
            | "function"
            | "generator_function"
    )
}

/// The name a function goes by: its own, or the one it is bound to. An arrow
/// function or function expression has no name of its own; it takes the
/// variable (`const add = (a, b) => …`), object key, assignment target
/// (`View.prototype.lookup = function …`) or class field it is assigned to.
/// None for an inline callback, whose calls belong to the function around it.
fn js_function_name(node: &Node<'_>, code: &[u8]) -> Option<String> {
    if let Some(n) = node.child_by_field_name("name") {
        return Some(node_text(&n, code).to_string());
    }

    let parent = node.parent()?;
    let bound = match parent.kind() {
        "variable_declarator" => parent
            .child_by_field_name("name")
            .filter(|n| n.kind() == "identifier"),
        "pair" => parent.child_by_field_name("key"),
        "assignment_expression" => parent.child_by_field_name("left").map(|left| {
            if left.kind() == "member_expression" {
                left.child_by_field_name("property").unwrap_or(left)
            } else {
                left
            }
        }),
        "public_field_definition" | "field_definition" | "property_definition" => parent
            .child_by_field_name("name")
            .or_else(|| parent.child_by_field_name("property")),
        "export_statement" => return Some("default".to_string()),
        _ => None,
    }?;

    let text = node_text(&bound, code)
        .trim_matches(['"', '\'', '`'])
        .trim();
    (!text.is_empty() && !text.contains(char::is_whitespace)).then(|| text.to_string())
}

fn analyze_js_node(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            // At module level an unnamed function (a test file's
            // `describe('…', () => …)`, an IIFE) has no function around it to
            // own its calls, so it is one, named by its line: unique, where
            // every one of them used to be `anonymous` (67 in one TSX file).
            kind if is_js_function(kind) => {
                let name = js_function_name(&child, ctx.code)
                    .unwrap_or_else(|| format!("anonymous_l{}", node_line(&child)));
                analyze_js_function(&child, module_id, module_id, &name, ctx)?
            }
            "class_declaration" | "class" => {
                analyze_js_class(&child, ctx)?;
            }
            "import_statement" => {
                analyze_js_import(&child, module_id, ctx)?;
            }
            // `namespace Range { export function from() }`: the namespace is
            // a scope, or every namespace's `from` is one id.
            "internal_module" | "module" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, ctx.code).to_string());
                match (name, child.child_by_field_name("body")) {
                    (Some(name), Some(body)) => {
                        let ns_id = scoped_id(module_id, &name);
                        emit_module(ctx, &ns_id, &name, ctx.filepath, node_line(&child));
                        analyze_js_node(&body, &ns_id, ctx)?;
                    }
                    _ => analyze_js_node(&child, module_id, ctx)?,
                }
            }
            "call_expression" => {
                analyze_js_require(&child, module_id, ctx);
                analyze_js_node(&child, module_id, ctx)?;
            }
            _ => {
                analyze_js_node(&child, module_id, ctx)?;
            }
        }
    }

    Ok(())
}

/// `scope` is the id the function's own id is scoped by: the module for a
/// top-level or class function, the enclosing function for a nested one.
fn analyze_js_function(
    node: &Node<'_>,
    module_id: &str,
    scope: &str,
    name: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let arity = param_count(node, "parameters");
    let line = node_line(node);
    let func_id = ctx.unique_id(scoped_id(scope, &format!("{}_{}", name, arity)), line);

    emit_function(ctx, &func_id, module_id, name, arity, "public", line, node);

    if let Some(body) = node.child_by_field_name("body") {
        analyze_js_body(&body, module_id, &func_id, ctx)?;
    }

    Ok(())
}

/// A function body: its calls, and the named functions declared inside it,
/// which become functions of their own. An unnamed callback stays part of the
/// body, so `arr.map(x => f(x))` still records the call to `f` here.
fn analyze_js_body(
    node: &Node<'_>,
    module_id: &str,
    func_id: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        let kind = child.kind();

        if is_js_function(kind) {
            if let Some(name) = js_function_name(&child, ctx.code) {
                analyze_js_function(&child, module_id, func_id, &name, ctx)?;
                continue;
            }
        }

        match kind {
            "call_expression" => {
                if let Some(f) = child.child_by_field_name("function") {
                    let callee = callee_name(&f, ctx.code);
                    if !callee.is_empty() {
                        emit_call(ctx, func_id, &callee, node_line(&child));
                    }
                }
                analyze_js_require(&child, module_id, ctx);
            }
            // `new X(...)` runs X's constructor.
            "new_expression" => {
                if let Some(c) = child.child_by_field_name("constructor") {
                    let callee = callee_name(&c, ctx.code);
                    if !callee.is_empty() {
                        emit_call(ctx, func_id, &callee, node_line(&child));
                    }
                }
            }
            "class_declaration" | "class" => {
                analyze_js_class(&child, ctx)?;
                continue;
            }
            _ => {}
        }

        analyze_js_body(&child, module_id, func_id, ctx)?;
    }

    Ok(())
}

/// CommonJS `require('x')` is a dependency, like an import.
fn analyze_js_require(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) {
    let Some(f) = node.child_by_field_name("function") else {
        return;
    };
    if f.kind() != "identifier" || node_text(&f, ctx.code) != "require" {
        return;
    }
    let Some(args) = node.child_by_field_name("arguments") else {
        return;
    };
    if let Some(arg) = args.named_child(0) {
        if arg.kind() == "string" {
            let target = node_text(&arg, ctx.code).trim_matches(['"', '\'', '`']);
            if !target.is_empty() {
                emit_dependency(ctx, module_id, target, "require", node_line(node));
            }
        }
    }
}

fn analyze_js_class(node: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let class_name = node_text(&name_node, ctx.code);
        let class_id = scoped_id(&normalize_id(ctx.filepath), class_name);

        emit_module(
            ctx,
            &class_id,
            class_name,
            &format!("class:{}", class_name),
            node_line(node),
        );

        if let Some(body) = node.child_by_field_name("body") {
            analyze_js_node(&body, &class_id, ctx)?;
        }
    }

    Ok(())
}

fn analyze_js_import(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    // The module is the import's `source` string: after `from`, or directly
    // for a side-effect import (`import "./x.scss"`), which had none.
    let source = node.child_by_field_name("source").map(|n| {
        node_text(&n, ctx.code)
            .trim_matches(['"', '\''])
            .to_string()
    });

    if let Some(imported_module) = source.filter(|s| !s.is_empty()) {
        emit_dependency(ctx, module_id, &imported_module, "import", node_line(node));
    }

    Ok(())
}

// ========================================================================
// Go Analysis
// ========================================================================

fn analyze_go(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let (module_id, module_name) = module_id_from_path(ctx.filepath, ".go", ".");
    emit_module(ctx, &module_id, &module_name, ctx.filepath, 1);

    let mut cursor = root.walk();
    for node in root.children(&mut cursor) {
        match node.kind() {
            "function_declaration" | "method_declaration" => {
                analyze_go_function(&node, &module_id, ctx)?;
            }
            "type_declaration" => {
                analyze_go_type(&node, &module_id, ctx)?;
            }
            "import_declaration" => {
                analyze_go_import(&node, &module_id, ctx)?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn analyze_go_function(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(&n, ctx.code).to_string(),
        None => return Ok(()),
    };

    let arity = param_count(node, "parameters");
    // A method is scoped by its receiver's type: `func (r JSON) Render` and
    // `func (r IndentedJSON) Render` were one id (gin render/json.go: 13
    // functions, 3 ids).
    let func_id = match go_receiver_type(node, ctx.code) {
        Some(recv) => scoped_id(module_id, &format!("{}_{}_{}", recv, name, arity)),
        None => scoped_id(module_id, &format!("{}_{}", name, arity)),
    };
    let line = node_line(node);

    let visibility = if name.starts_with(|c: char| c.is_uppercase()) {
        "public"
    } else {
        "private"
    };

    emit_function(
        ctx, &func_id, module_id, &name, arity, visibility, line, node,
    );

    if let Some(body) = node.child_by_field_name("body") {
        analyze_calls(&body, ctx.code, &func_id, ctx, "call_expression")?;
    }

    Ok(())
}

/// The receiver's type name of a Go method: `(r *Router[T])` → `router`.
fn go_receiver_type(node: &Node<'_>, code: &[u8]) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;
    let mut cursor = receiver.walk();
    let param = receiver
        .children(&mut cursor)
        .find(|c| c.kind() == "parameter_declaration")?;
    let ty = param.child_by_field_name("type")?;
    let text = node_text(&ty, code);
    let bare = text
        .trim_start_matches('*')
        .split('[')
        .next()
        .unwrap_or("")
        .trim();
    (!bare.is_empty()).then(|| normalize_id(bare))
}

/// A type declaration is a type of this package, like Rust's struct items;
/// it was recorded as a dependency on its own name.
fn analyze_go_type(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let mut cursor = node.walk();
    for spec in node.children(&mut cursor) {
        if !matches!(spec.kind(), "type_spec" | "type_alias") {
            continue;
        }
        if let Some(name_node) = spec.child_by_field_name("name") {
            let type_name = node_text(&name_node, ctx.code);
            let type_id = scoped_id(module_id, type_name);
            emit_module(
                ctx,
                &type_id,
                type_name,
                &format!("type:{}", module_id),
                node_line(&spec),
            );
        }
    }

    Ok(())
}

fn analyze_go_import(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let text = node_text(node, ctx.code);

    for line in text.lines() {
        if let Some(start) = line.find('"') {
            if let Some(end) = line[start + 1..].find('"') {
                let imported = &line[start + 1..start + 1 + end];
                emit_dependency(ctx, module_id, imported, "import", node_line(node));
            }
        }
    }

    Ok(())
}

// ========================================================================
// Elixir Analysis
// ========================================================================

fn analyze_elixir(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let (module_id, module_name) = module_id_from_path(ctx.filepath, ".ex", ".");
    emit_module(ctx, &module_id, &module_name, ctx.filepath, 1);
    analyze_elixir_node(root, &module_id, None, ctx)
}

/// Function-defining macros: each is one callable, whatever it is called.
const ELIXIR_DEFS: &[&str] = &[
    "def",
    "defp",
    "defmacro",
    "defmacrop",
    "defguard",
    "defguardp",
    "defdelegate",
];

fn analyze_elixir_node(
    node: &Node<'_>,
    module_id: &str,
    module_name: Option<&str>,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "call" => {
                if let Some(target) = child.child_by_field_name("target") {
                    let target_text = node_text(&target, ctx.code);
                    match target_text {
                        "defmodule" => analyze_elixir_defmodule(&child, module_name, ctx)?,
                        t if ELIXIR_DEFS.contains(&t) => {
                            analyze_elixir_function(&child, module_id, target_text, ctx)?
                        }
                        "import" | "use" | "alias" => {
                            analyze_elixir_dep(&child, module_id, target_text, ctx)?
                        }
                        _ => analyze_elixir_node(&child, module_id, module_name, ctx)?,
                    }
                } else {
                    analyze_elixir_node(&child, module_id, module_name, ctx)?;
                }
            }
            _ => {
                analyze_elixir_node(&child, module_id, module_name, ctx)?;
            }
        }
    }

    Ok(())
}

fn analyze_elixir_defmodule(
    node: &Node<'_>,
    parent: Option<&str>,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    // In tree-sitter-elixir, `defmodule Foo do...end` is a `call` node with children:
    //   target: identifier("defmodule")       [field]
    //   arguments: (alias("Foo"))             [no field, find by kind]
    //   do_block: do...end                    [no field, find by kind]

    let local_name = find_child_by_kind(node, "arguments")
        .and_then(|a| a.named_child(0))
        .map(|n| node_text(&n, ctx.code).to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    // A nested `defmodule Child` inside `Parent` is Parent.Child; the parent
    // was dropped, so two parents' `Helpers` merged into one module.
    let module_name = match parent {
        Some(p) => format!("{}.{}", p, local_name),
        None => local_name,
    };

    let module_id = normalize_id(&module_name);
    emit_module(
        ctx,
        &module_id,
        &module_name,
        &format!("defmodule:{}", module_name),
        node_line(node),
    );

    if let Some(do_block) = find_child_by_kind(node, "do_block") {
        analyze_elixir_node(&do_block, &module_id, Some(&module_name), ctx)?;
    }

    Ok(())
}

fn analyze_elixir_function(
    node: &Node<'_>,
    module_id: &str,
    kind: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    // `def add(a, b) do...end` is a call node:
    //   target: identifier("def")         [field]
    //   arguments: (call("add", "(a,b)")) [no field, find by kind]
    //   do_block: do...end                [no field, find by kind]

    let args = match find_child_by_kind(node, "arguments") {
        Some(a) => a,
        None => return Ok(()),
    };

    let head = match args.named_child(0) {
        Some(h) => h,
        None => return Ok(()),
    };

    // `def f(x) when is_integer(x)` puts the head under a `when` operator; it
    // was read whole, naming the function "f(x) when is_integer(x)", arity 0.
    let head = if head.kind() == "binary_operator" {
        head.child_by_field_name("left").unwrap_or(head)
    } else {
        head
    };

    let (func_name, arity) = if head.kind() == "call" {
        let name = head
            .child_by_field_name("target")
            .map(|n| node_text(&n, ctx.code).to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let params = find_child_by_kind(&head, "arguments")
            .map(|a| a.named_child_count())
            .unwrap_or(0);
        (name, params)
    } else {
        (node_text(&head, ctx.code).to_string(), 0)
    };

    let func_id = scoped_id(module_id, &format!("{}_{}", func_name, arity));
    let line = node_line(node);
    let visibility = if kind.ends_with('p') {
        "private"
    } else {
        "public"
    };

    // Each clause of a multi-clause function is its own `def`; the function
    // is emitted once, and every clause's calls are attached to it.
    if ctx.seen_functions.insert(func_id.clone()) {
        emit_function(
            ctx, &func_id, module_id, &func_name, arity, visibility, line, node,
        );
    }

    if let Some(do_block) = find_child_by_kind(node, "do_block") {
        analyze_elixir_calls(&do_block, ctx.code, &func_id, ctx)?;
    } else {
        // `def f(x), do: g(x)`: the body is a keyword argument after the
        // head, and its calls were never read.
        let mut cursor = args.walk();
        for arg in args.named_children(&mut cursor).skip(1) {
            analyze_elixir_calls(&arg, ctx.code, &func_id, ctx)?;
        }
    }

    Ok(())
}

fn analyze_elixir_dep(
    node: &Node<'_>,
    module_id: &str,
    kind: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let args = match find_child_by_kind(node, "arguments") {
        Some(a) => a,
        None => return Ok(()),
    };

    if let Some(first_arg) = args.named_child(0) {
        let dep_name = node_text(&first_arg, ctx.code);
        emit_dependency(ctx, module_id, dep_name, kind, node_line(node));
    }

    Ok(())
}

fn analyze_elixir_calls(
    node: &Node<'_>,
    code: &[u8],
    func_id: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(target) = child.child_by_field_name("target") {
                let called = node_text(&target, code);
                // Definitions and special forms are syntax, not calls.
                if !matches!(
                    called,
                    "def"
                        | "defp"
                        | "defmacro"
                        | "defmacrop"
                        | "defguard"
                        | "defguardp"
                        | "defdelegate"
                        | "defmodule"
                        | "import"
                        | "use"
                        | "alias"
                        | "require"
                        | "do"
                        | "end"
                        | "case"
                        | "cond"
                        | "if"
                        | "unless"
                        | "for"
                        | "with"
                        | "quote"
                        | "unquote"
                        | "unquote_splicing"
                        | "receive"
                        | "try"
                        | "fn"
                ) {
                    emit_call(ctx, func_id, called, node_line(&child));
                }
            }
        }
        analyze_elixir_calls(&child, code, func_id, ctx)?;
    }

    Ok(())
}

// ========================================================================
// Ruby Analysis
// ========================================================================

fn analyze_ruby(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let (module_id, module_name) = module_id_from_path(ctx.filepath, ".rb", "::");
    emit_module(ctx, &module_id, &module_name, ctx.filepath, 1);
    analyze_ruby_body(root, &module_id, ctx)
}

/// Iterates children sequentially, tracking Ruby visibility modifiers.
///
/// In Ruby, bare `private`/`protected`/`public` statements change the default
/// visibility for all subsequent method definitions in that scope. The form
/// `private :method_name` overrides visibility for a specific named method.
fn analyze_ruby_body(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let mut cursor = node.walk();
    let mut current_visibility = "public";
    let mut per_method_overrides: Vec<(String, String)> = Vec::new();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let text = node_text(&child, ctx.code);
                match text {
                    "private" | "protected" | "public" => {
                        current_visibility = match text {
                            "private" => "private",
                            "protected" => "protected",
                            _ => "public",
                        };
                    }
                    _ => {}
                }
            }
            "method" => {
                let method_name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, ctx.code).to_string());

                let visibility = method_name
                    .as_deref()
                    .and_then(|name| {
                        per_method_overrides
                            .iter()
                            .find(|(n, _)| n == name)
                            .map(|(_, v)| v.as_str())
                    })
                    .unwrap_or(current_visibility);

                analyze_ruby_method(&child, module_id, visibility, ctx)?;
            }
            "singleton_method" => {
                analyze_ruby_method(&child, module_id, "public", ctx)?;
            }
            "class" => {
                analyze_ruby_class(&child, ctx)?;
            }
            "module" => {
                analyze_ruby_module(&child, ctx)?;
            }
            "call" => {
                let call_name = child
                    .child(0)
                    .map(|n| node_text(&n, ctx.code).to_string())
                    .unwrap_or_default();

                match call_name.as_str() {
                    "private" | "protected" | "public" => {
                        if let Some(args) = find_child_by_kind(&child, "argument_list") {
                            // `private :method_name` — per-method override
                            let mut args_cursor = args.walk();
                            for arg in args.named_children(&mut args_cursor) {
                                let sym_text = node_text(&arg, ctx.code);
                                let method_name = sym_text.trim_start_matches(':');
                                per_method_overrides
                                    .push((method_name.to_string(), call_name.clone()));
                            }
                        } else {
                            // Bare `private` as a call node (shouldn't happen
                            // based on what we see, but handle defensively)
                            current_visibility = match call_name.as_str() {
                                "private" => "private",
                                "protected" => "protected",
                                _ => "public",
                            };
                        }
                    }
                    "require" | "require_relative" => {
                        analyze_ruby_require(&child, module_id, ctx)?;
                    }
                    "include" | "extend" | "prepend" => {
                        analyze_ruby_include(&child, module_id, ctx)?;
                    }
                    _ => {
                        analyze_ruby_body(&child, module_id, ctx)?;
                    }
                }
            }
            _ => {
                analyze_ruby_body(&child, module_id, ctx)?;
            }
        }
    }

    Ok(())
}

fn analyze_ruby_class(node: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let class_name = node
        .child_by_field_name("name")
        .map(|n| node_text(&n, ctx.code).to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let class_id = scoped_id(&normalize_id(ctx.filepath), &class_name);
    emit_module(
        ctx,
        &class_id,
        &class_name,
        &format!("class:{}", class_name),
        node_line(node),
    );

    if let Some(body) = node.child_by_field_name("body") {
        analyze_ruby_body(&body, &class_id, ctx)?;
    }

    Ok(())
}

fn analyze_ruby_module(node: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let module_name = node
        .child_by_field_name("name")
        .map(|n| node_text(&n, ctx.code).to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    // Scoped by file, as classes are: every concern's `ClassMethods` was one
    // module id across the codebase (rails: 206 cross-file duplicate ids).
    let module_id = scoped_id(&normalize_id(ctx.filepath), &module_name);
    emit_module(
        ctx,
        &module_id,
        &module_name,
        &format!("module:{}", module_name),
        node_line(node),
    );

    if let Some(body) = node.child_by_field_name("body") {
        analyze_ruby_body(&body, &module_id, ctx)?;
    }

    Ok(())
}

fn analyze_ruby_method(
    node: &Node<'_>,
    module_id: &str,
    visibility: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(&n, ctx.code).to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let arity = node
        .child_by_field_name("parameters")
        .map(|n| n.named_child_count())
        .unwrap_or(0);

    let func_id = scoped_id(module_id, &format!("{}_{}", name, arity));
    let line = node_line(node);

    emit_function(
        ctx, &func_id, module_id, &name, arity, visibility, line, node,
    );

    if let Some(body) = node.child_by_field_name("body") {
        let mut locals = std::collections::HashSet::new();
        if let Some(params) = node.child_by_field_name("parameters") {
            ruby_collect_identifiers(&params, ctx.code, &mut locals);
        }
        ruby_collect_locals(&body, ctx.code, &mut locals);
        analyze_ruby_calls(&body, ctx.code, &func_id, &locals, ctx)?;
    }

    Ok(())
}

/// Every identifier under `node`.
fn ruby_collect_identifiers(
    node: &Node<'_>,
    code: &[u8],
    out: &mut std::collections::HashSet<String>,
) {
    if node.kind() == "identifier" {
        out.insert(node_text(node, code).to_string());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        ruby_collect_identifiers(&child, code, out);
    }
}

/// The local variables of a method body: assignment targets and block,
/// lambda and rescue parameters. Ruby itself reads any other bare name as a
/// method call, and so does the call walker.
fn ruby_collect_locals(node: &Node<'_>, code: &[u8], out: &mut std::collections::HashSet<String>) {
    match node.kind() {
        "assignment" | "operator_assignment" => {
            if let Some(left) = node.child_by_field_name("left") {
                ruby_collect_identifiers(&left, code, out);
            }
        }
        "block_parameters" | "lambda_parameters" | "exception_variable" => {
            ruby_collect_identifiers(node, code, out);
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        ruby_collect_locals(&child, code, out);
    }
}

fn analyze_ruby_require(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let text = node_text(node, ctx.code);

    if let Some(start) = text.find(['"', '\'']) {
        let remaining = &text[start + 1..];
        if let Some(end) = remaining.find(['"', '\'']) {
            let required = &remaining[..end];
            let kind = if text.starts_with("require_relative") {
                "require_relative"
            } else {
                "require"
            };
            emit_dependency(ctx, module_id, required, kind, node_line(node));
        }
    }

    Ok(())
}

/// `include Foo`, `extend Bar::Baz`, `include(Qux)`: each constant argument
/// is a dependency. Only constants count: an RSpec matcher such as
/// `include("foo=;", "bar")` is a call, not a mixin, and splitting its text
/// on spaces emitted it as one; `include(Bar)` was dropped.
fn analyze_ruby_include(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let kind = node
        .child_by_field_name("method")
        .map(|m| node_text(&m, ctx.code).to_string())
        .unwrap_or_else(|| "include".to_string());

    let Some(args) = node.child_by_field_name("arguments") else {
        return Ok(());
    };

    let mut cursor = args.walk();
    for arg in args.named_children(&mut cursor) {
        if matches!(arg.kind(), "constant" | "scope_resolution") {
            let included = node_text(&arg, ctx.code).to_string();
            emit_dependency(ctx, module_id, &included, &kind, node_line(node));
        }
    }

    Ok(())
}

fn analyze_ruby_calls(
    node: &Node<'_>,
    code: &[u8],
    func_id: &str,
    locals: &std::collections::HashSet<String>,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "call" => {
                let method = child.child_by_field_name("method");
                if let Some(method) = method {
                    let called = node_text(&method, code);
                    if !matches!(
                        called,
                        "require"
                            | "require_relative"
                            | "include"
                            | "extend"
                            | "prepend"
                            | "attr_reader"
                            | "attr_writer"
                            | "attr_accessor"
                    ) {
                        emit_call(ctx, func_id, called, node_line(&child));
                    }
                }
                // Its receiver, arguments and block, but not the method name
                // again as a bare identifier.
                let mut inner = child.walk();
                for part in child.children(&mut inner) {
                    if Some(part.id()) != method.map(|m| m.id()) {
                        if part.kind() == "identifier" {
                            ruby_bare_call(&part, code, func_id, locals, ctx);
                        } else {
                            analyze_ruby_calls(&part, code, func_id, locals, ctx)?;
                        }
                    }
                }
                continue;
            }
            // A bare name that isn't a local is a call with no receiver or
            // arguments (`app`, `options`); these were never recorded.
            "identifier" => ruby_bare_call(&child, code, func_id, locals, ctx),
            // Definitions, not uses.
            "block_parameters" | "lambda_parameters" | "exception_variable" => continue,
            "assignment" | "operator_assignment" => {
                if let Some(right) = child.child_by_field_name("right") {
                    analyze_ruby_calls(&right, code, func_id, locals, ctx)?;
                }
                if child.kind() == "operator_assignment" {
                    if let Some(left) = child.child_by_field_name("left") {
                        analyze_ruby_calls(&left, code, func_id, locals, ctx)?;
                    }
                }
                continue;
            }
            _ => {}
        }
        analyze_ruby_calls(&child, code, func_id, locals, ctx)?;
    }

    Ok(())
}

fn ruby_bare_call(
    node: &Node<'_>,
    code: &[u8],
    func_id: &str,
    locals: &std::collections::HashSet<String>,
    ctx: &mut Ctx<'_>,
) {
    let name = node_text(node, code);
    if !locals.contains(name) && !matches!(name, "private" | "protected" | "public") {
        emit_call(ctx, func_id, name, node_line(node));
    }
}

// ========================================================================
// PHP Analysis
// ========================================================================
//
// Uses the HTML-intermixed `php` grammar so WordPress-style template files
// (HTML + `<?php ?>` islands) lens as well as pure-PHP class files.
//   - `function_definition` and `method_declaration` both emit `function`
//     facts; method visibility comes from the `visibility_modifier` child
//     (PHP defaults to public when unmodified);
//   - classes / interfaces / traits / enums emit `module` facts and scope
//     their methods, mirroring the Ruby class handling;
//   - a file-level `namespace Foo;` (no body) switches the module scope for
//     the remaining siblings; `namespace Foo { ... }` scopes its body;
//   - `use Foo\Bar;` (namespace_use_declaration) and include/require
//     expressions emit `depends_on`;
//   - calls come from `function_call_expression` (plain / qualified name),
//     `member_call_expression` / `nullsafe_member_call_expression` (method
//     name), `scoped_call_expression` (`Scope::name`), and
//     `object_creation_expression` (constructor → class name).
// Module names use `::` separators (not PHP's `\`) so downstream name
// matching (e.g. manifold-nav's suffix-on-separator rules) works unchanged.

fn analyze_php(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let (module_id, module_name) = module_id_from_path(ctx.filepath, ".php", "::");
    emit_module(ctx, &module_id, &module_name, ctx.filepath, 1);
    analyze_php_body(root, &module_id, ctx)
}

fn analyze_php_body(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let mut cursor = node.walk();
    // `namespace Foo;` without a body re-scopes the rest of the file.
    let mut current_module = module_id.to_string();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_definition" => {
                let ns_name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, ctx.code).to_string())
                    .unwrap_or_else(|| "global".to_string());
                let ns_id = normalize_id(&ns_name);
                emit_module(
                    ctx,
                    &ns_id,
                    &ns_name,
                    &format!("namespace:{}", ns_name),
                    node_line(&child),
                );
                if let Some(body) = child.child_by_field_name("body") {
                    analyze_php_body(&body, &ns_id, ctx)?;
                } else {
                    current_module = ns_id;
                }
            }
            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration" => {
                analyze_php_class(&child, ctx)?;
            }
            "function_definition" => {
                analyze_php_function(&child, &current_module, "public", ctx)?;
            }
            "namespace_use_declaration" => {
                analyze_php_use(&child, &current_module, ctx)?;
            }
            "include_expression"
            | "include_once_expression"
            | "require_expression"
            | "require_once_expression" => {
                analyze_php_include(&child, &current_module, ctx)?;
            }
            _ => {
                analyze_php_body(&child, &current_module, ctx)?;
            }
        }
    }

    Ok(())
}

fn analyze_php_class(node: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let class_name = node
        .child_by_field_name("name")
        .map(|n| node_text(&n, ctx.code).to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let class_id = scoped_id(&normalize_id(ctx.filepath), &class_name);
    let kind_label = node.kind().trim_end_matches("_declaration");
    emit_module(
        ctx,
        &class_id,
        &class_name,
        &format!("{}:{}", kind_label, class_name),
        node_line(node),
    );

    // `extends Base` / `implements Iface` → dependencies.
    if let Some(base) = find_child_by_kind(node, "base_clause") {
        emit_php_name_deps(&base, &class_id, "extends", ctx);
    }
    if let Some(impls) = find_child_by_kind(node, "class_interface_clause") {
        emit_php_name_deps(&impls, &class_id, "implements", ctx);
    }

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for member in body.children(&mut cursor) {
            match member.kind() {
                "method_declaration" => {
                    let visibility = php_visibility(&member, ctx.code);
                    analyze_php_function(&member, &class_id, visibility, ctx)?;
                }
                "use_declaration" => {
                    // Trait use inside a class body.
                    emit_php_name_deps(&member, &class_id, "use", ctx);
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Visibility of a class member from its `visibility_modifier` child;
/// PHP members default to public when unmodified.
fn php_visibility<'a>(node: &Node<'_>, code: &'a [u8]) -> &'a str {
    match find_child_by_kind(node, "visibility_modifier") {
        Some(m) => match node_text(&m, code) {
            t @ ("private" | "protected" | "public") => t,
            _ => "public",
        },
        None => "public",
    }
}

fn analyze_php_function(
    node: &Node<'_>,
    module_id: &str,
    visibility: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(&n, ctx.code).to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let arity = param_count(node, "parameters");
    let func_id = scoped_id(module_id, &format!("{}_{}", name, arity));
    let line = node_line(node);

    emit_function(
        ctx, &func_id, module_id, &name, arity, visibility, line, node,
    );

    if let Some(body) = node.child_by_field_name("body") {
        analyze_php_calls(&body, &func_id, module_id, ctx)?;
    }

    Ok(())
}

/// Emit a `depends_on` for every `name` / `qualified_name` under `node`
/// (covers base_clause, class_interface_clause, trait use, use-clauses).
fn emit_php_name_deps(node: &Node<'_>, module_id: &str, kind: &str, ctx: &mut Ctx<'_>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "name" | "qualified_name" => {
                emit_dependency(
                    ctx,
                    module_id,
                    node_text(&child, ctx.code),
                    kind,
                    node_line(&child),
                );
            }
            _ => emit_php_name_deps(&child, module_id, kind, ctx),
        }
    }
}

/// `use Foo\Bar;` / `use Foo\{A, B};` → one dependency per use-clause name.
fn analyze_php_use(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    emit_php_name_deps(node, module_id, "use", ctx);
    Ok(())
}

/// include/require expressions → dependency on the literal path (when the
/// argument is a string literal; dynamic paths are skipped).
fn analyze_php_include(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let kind = node.kind().trim_end_matches("_expression");
    let text = node_text(node, ctx.code);
    if let Some(start) = text.find(['"', '\'']) {
        let remaining = &text[start + 1..];
        if let Some(end) = remaining.find(['"', '\'']) {
            emit_dependency(ctx, module_id, &remaining[..end], kind, node_line(node));
        }
    }
    Ok(())
}

fn analyze_php_calls(
    node: &Node<'_>,
    func_id: &str,
    module_id: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_call_expression" => {
                if let Some(function) = child.child_by_field_name("function") {
                    // Skip dynamic calls through variables (`$callback()`).
                    if matches!(function.kind(), "name" | "qualified_name") {
                        emit_call(
                            ctx,
                            func_id,
                            node_text(&function, ctx.code),
                            node_line(&child),
                        );
                    }
                }
            }
            "member_call_expression" | "nullsafe_member_call_expression" => {
                if let Some(name) = child.child_by_field_name("name") {
                    emit_call(ctx, func_id, node_text(&name, ctx.code), node_line(&child));
                }
            }
            "scoped_call_expression" => {
                let scope = child
                    .child_by_field_name("scope")
                    .map(|n| node_text(&n, ctx.code).to_string())
                    .unwrap_or_default();
                if let Some(name) = child.child_by_field_name("name") {
                    let called = if scope.is_empty() {
                        node_text(&name, ctx.code).to_string()
                    } else {
                        format!("{}::{}", scope, node_text(&name, ctx.code))
                    };
                    emit_call(ctx, func_id, &called, node_line(&child));
                }
            }
            "object_creation_expression" => {
                let mut oc_cursor = child.walk();
                let class_name = child
                    .named_children(&mut oc_cursor)
                    .find(|n| matches!(n.kind(), "name" | "qualified_name"))
                    .map(|n| node_text(&n, ctx.code).to_string());
                if let Some(class_name) = class_name {
                    emit_call(ctx, func_id, &class_name, node_line(&child));
                }
            }
            "include_expression"
            | "include_once_expression"
            | "require_expression"
            | "require_once_expression" => {
                analyze_php_include(&child, module_id, ctx)?;
                continue;
            }
            _ => {}
        }
        analyze_php_calls(&child, func_id, module_id, ctx)?;
    }

    Ok(())
}

// ========================================================================
// C# Analysis
// ========================================================================
//
// A type's module id is its namespace-qualified name, not its file, so the
// parts of a partial class written across files are one module. Members are
// functions: methods, constructors (named for the type, which is what
// `new T()` calls), finalizers, operators, properties and indexers with
// bodies, event accessors and local functions. Auto-properties carry no code
// and are left out. Top-level statements are the file's `Main`. Attributes
// become `annotation(Id, Name, Args)`, so `[HttpGet("orders/{id}")]` routes
// can be found.

const CSHARP_TYPES: &[&str] = &[
    "class_declaration",
    "struct_declaration",
    "interface_declaration",
    "record_declaration",
    "record_struct_declaration",
    "enum_declaration",
    "delegate_declaration",
];

fn analyze_csharp(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let (file_id, file_name) = module_id_from_path(ctx.filepath, ".cs", ".");
    emit_module(ctx, &file_id, &file_name, ctx.filepath, 1);
    let mut main: Option<String> = None;
    analyze_csharp_scope(root, &file_id, "", &mut main, ctx)
}

fn dotted(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", prefix, name)
    }
}

/// The declarations of a compilation unit or namespace body. `ns` is the
/// enclosing namespace; a file-scoped namespace applies to the rest of the
/// file.
fn analyze_csharp_scope(
    node: &Node<'_>,
    file_id: &str,
    ns: &str,
    main: &mut Option<String>,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let mut ns = ns.to_string();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "using_directive" => csharp_using(&child, file_id, ctx),
            "file_scoped_namespace_declaration" => {
                if let Some(name) = child.child_by_field_name("name") {
                    ns = node_text(&name, ctx.code).to_string();
                }
            }
            "namespace_declaration" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, ctx.code).to_string())
                    .unwrap_or_default();
                if let Some(body) = child.child_by_field_name("body") {
                    analyze_csharp_scope(&body, file_id, &dotted(&ns, &name), main, ctx)?;
                }
            }
            "global_statement" => {
                // C# 9 top-level statements: the program's entry point.
                let id = main
                    .get_or_insert_with(|| {
                        let id = scoped_id(file_id, "Main_0");
                        emit_function(
                            ctx,
                            &id,
                            file_id,
                            "Main",
                            0,
                            "public",
                            node_line(&child),
                            &child,
                        );
                        id
                    })
                    .clone();
                analyze_csharp_calls(&child, &id, file_id, ctx)?;
            }
            k if CSHARP_TYPES.contains(&k) => analyze_csharp_type(&child, &ns, ctx)?,
            _ => {}
        }
    }
    Ok(())
}

/// `using A.B;`, `using static A.B;`, `global using A;`, `using X = A.B;`:
/// a dependency of the file on the namespace or type named.
fn csharp_using(node: &Node<'_>, file_id: &str, ctx: &mut Ctx<'_>) {
    let alias = node.child_by_field_name("name").map(|n| n.id());
    let mut cursor = node.walk();
    let target = node
        .named_children(&mut cursor)
        .filter(|c| Some(c.id()) != alias)
        .find(|c| {
            matches!(
                c.kind(),
                "identifier" | "qualified_name" | "generic_name" | "alias_qualified_name"
            )
        });
    if let Some(t) = target {
        let text = node_text(&t, ctx.code).to_string();
        emit_dependency(ctx, file_id, &text, "using", node_line(node));
    }
}

/// A type's name without type arguments: `Repo<T>` → `Repo`, `A.B<T>` → `A.B`.
fn csharp_type_name(node: &Node<'_>, code: &[u8]) -> String {
    let text = node_text(node, code);
    let text = text.split('<').next().unwrap_or(text);
    text.split_whitespace().collect::<Vec<_>>().join("")
}

fn csharp_modifiers(node: &Node<'_>, code: &[u8]) -> Vec<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|c| c.kind() == "modifier")
        .map(|c| node_text(&c, code).to_string())
        .collect()
}

/// C#'s accessibility, with the language's defaults: members of a class or
/// struct are private, of an interface public. `protected internal` and
/// `private protected` read as protected.
fn csharp_visibility(modifiers: &[String], in_interface: bool) -> &'static str {
    let has = |m: &str| modifiers.iter().any(|x| x == m);
    if has("public") {
        "public"
    } else if has("protected") {
        "protected"
    } else if has("internal") {
        "internal"
    } else if has("private") {
        "private"
    } else if in_interface {
        "public"
    } else {
        "private"
    }
}

/// `annotation(Id, Name, Args)` for each attribute on `node`.
fn csharp_annotations(node: &Node<'_>, id: &str, ctx: &mut Ctx<'_>) {
    let mut cursor = node.walk();
    let lists: Vec<Node<'_>> = node
        .children(&mut cursor)
        .filter(|c| c.kind() == "attribute_list")
        .collect();
    for list in lists {
        let mut lc = list.walk();
        let attrs: Vec<Node<'_>> = list
            .named_children(&mut lc)
            .filter(|a| a.kind() == "attribute")
            .collect();
        for attr in attrs {
            let name = attr
                .child_by_field_name("name")
                .map(|n| csharp_type_name(&n, ctx.code))
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let args = find_child_by_kind(&attr, "attribute_argument_list")
                .map(|a| {
                    let t = node_text(&a, ctx.code);
                    clean_param(t.trim_start_matches('(').trim_end_matches(')'))
                })
                .unwrap_or_default();
            ctx.facts.push(Fact::new(
                "annotation",
                vec![
                    FactValue::String(id.to_string()),
                    FactValue::String(name),
                    FactValue::String(args),
                ],
            ));
        }
    }
}

fn analyze_csharp_type(node: &Node<'_>, outer: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(&n, ctx.code).to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    // `Result` and `Result<T>` are different types: the type parameters are
    // part of the module's name and id.
    let generic = node
        .child_by_field_name("type_parameters")
        .or_else(|| find_child_by_kind(node, "type_parameter_list"))
        .map(|tp| {
            let mut c = tp.walk();
            let names: Vec<String> = tp
                .named_children(&mut c)
                .filter(|p| p.kind() == "type_parameter")
                .filter_map(|p| {
                    p.child_by_field_name("name")
                        .map(|n| node_text(&n, ctx.code).to_string())
                        .or_else(|| first_identifier_text(&p, ctx.code))
                })
                .collect();
            format!("<{}>", names.join(","))
        })
        .unwrap_or_default();
    let qualified = dotted(outer, &format!("{}{}", name, generic));
    let type_id = normalize_id(&qualified);
    let label = node.kind().trim_end_matches("_declaration");
    emit_module(
        ctx,
        &type_id,
        &qualified,
        &format!("{}:{}", label, qualified),
        node_line(node),
    );
    csharp_annotations(node, &type_id, ctx);

    // A class's base class is listed first; `: base(…)` calls its constructor.
    let mut first_base: Option<String> = None;
    if let Some(bases) = find_child_by_kind(node, "base_list") {
        let mut cursor = bases.walk();
        let names: Vec<(String, usize)> = bases
            .named_children(&mut cursor)
            .filter(|b| {
                matches!(
                    b.kind(),
                    "identifier"
                        | "qualified_name"
                        | "generic_name"
                        | "primary_constructor_base_type"
                )
            })
            .map(|b| {
                let n = if b.kind() == "primary_constructor_base_type" {
                    b.child_by_field_name("type").unwrap_or(b)
                } else {
                    b
                };
                (csharp_type_name(&n, ctx.code), node_line(&b))
            })
            .collect();
        for (base, line) in &names {
            emit_dependency(ctx, &type_id, base, "inherits", *line);
        }
        first_base = names.first().map(|(b, _)| b.clone());
    }

    let in_interface = node.kind() == "interface_declaration";
    let Some(body) = node.child_by_field_name("body") else {
        return Ok(());
    };
    let mut cursor = body.walk();
    let members: Vec<Node<'_>> = body.named_children(&mut cursor).collect();
    for member in members {
        let kind = member.kind();
        if CSHARP_TYPES.contains(&kind) {
            analyze_csharp_type(&member, &qualified, ctx)?;
            continue;
        }
        let member_name = || {
            member
                .child_by_field_name("name")
                .map(|n| node_text(&n, ctx.code).to_string())
        };
        // (name, arity, the nodes whose code belongs to the member)
        let (fname, arity): (String, usize) = match kind {
            "method_declaration" => (
                member_name().unwrap_or_else(|| "unknown".into()),
                param_count(&member, "parameters"),
            ),
            "constructor_declaration" => (name.clone(), param_count(&member, "parameters")),
            "destructor_declaration" => ("Finalize".into(), 0),
            "operator_declaration" => {
                let op = member
                    .child_by_field_name("operator")
                    .map(|n| node_text(&n, ctx.code).to_string())
                    .unwrap_or_default();
                (
                    csharp_operator_name(&op),
                    param_count(&member, "parameters"),
                )
            }
            "conversion_operator_declaration" => {
                let text = node_text(&member, ctx.code);
                let which = if text.contains("explicit") {
                    "op_Explicit"
                } else {
                    "op_Implicit"
                };
                (which.into(), param_count(&member, "parameters"))
            }
            "property_declaration" | "event_declaration" => {
                if !csharp_has_code(&member) {
                    continue;
                }
                (member_name().unwrap_or_else(|| "unknown".into()), 0)
            }
            "indexer_declaration" => {
                if !csharp_has_code(&member) {
                    continue;
                }
                ("Item".into(), param_count(&member, "parameters"))
            }
            _ => continue,
        };

        let line = node_line(&member);
        let func_id = ctx.unique_id(scoped_id(&type_id, &format!("{}_{}", fname, arity)), line);
        let visibility = csharp_visibility(&csharp_modifiers(&member, ctx.code), in_interface);
        emit_function(
            ctx, &func_id, &type_id, &fname, arity, visibility, line, &member,
        );
        csharp_annotations(&member, &func_id, ctx);

        for field in ["body", "value", "accessors"] {
            if let Some(code) = member.child_by_field_name(field) {
                analyze_csharp_calls(&code, &func_id, &type_id, ctx)?;
            }
        }
        // `: base(x)` calls the base class's constructor, `: this(x)` another
        // of this type's.
        if let Some(init) = find_child_by_kind(&member, "constructor_initializer") {
            let text = node_text(&init, ctx.code);
            let to_base = text
                .trim_start_matches(':')
                .trim_start()
                .starts_with("base");
            let target = match (&first_base, to_base) {
                (Some(b), true) => b.rsplit('.').next().unwrap_or(b).to_string(),
                (None, true) => "base".to_string(),
                (_, false) => name.clone(),
            };
            emit_call(ctx, &func_id, &target, node_line(&init));
            analyze_csharp_calls(&init, &func_id, &type_id, ctx)?;
        }
    }
    Ok(())
}

/// Whether a property, indexer or event declares code: an accessor with a
/// body or an expression body. `{ get; set; }` declares none.
fn csharp_has_code(node: &Node<'_>) -> bool {
    if node.child_by_field_name("value").is_some() {
        return true;
    }
    match node.child_by_field_name("accessors") {
        Some(list) => {
            let mut cursor = list.walk();
            let found = list
                .named_children(&mut cursor)
                .any(|a| a.child_by_field_name("body").is_some());
            found
        }
        None => false,
    }
}

/// .NET's names for user-defined operators (`op_Addition`), so overloads of
/// different operators are different functions.
fn csharp_operator_name(op: &str) -> String {
    let name = match op.trim() {
        "+" => "op_Addition",
        "-" => "op_Subtraction",
        "*" => "op_Multiply",
        "/" => "op_Division",
        "%" => "op_Modulus",
        "==" => "op_Equality",
        "!=" => "op_Inequality",
        "<" => "op_LessThan",
        ">" => "op_GreaterThan",
        "<=" => "op_LessThanOrEqual",
        ">=" => "op_GreaterThanOrEqual",
        "!" => "op_LogicalNot",
        "~" => "op_OnesComplement",
        "++" => "op_Increment",
        "--" => "op_Decrement",
        "&" => "op_BitwiseAnd",
        "|" => "op_BitwiseOr",
        "^" => "op_ExclusiveOr",
        "<<" => "op_LeftShift",
        ">>" => "op_RightShift",
        "true" => "op_True",
        "false" => "op_False",
        other => return format!("op_{}", normalize_id(other)),
    };
    name.to_string()
}

/// Calls in a member's code. Lambdas' calls belong to the member; a local
/// function is a function of its own, scoped by the member.
fn analyze_csharp_calls(
    node: &Node<'_>,
    func_id: &str,
    type_id: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    for child in children {
        match child.kind() {
            "invocation_expression" => {
                if let Some(callee) = child
                    .child_by_field_name("function")
                    .and_then(|f| csharp_callee(&f, ctx.code))
                {
                    emit_call(ctx, func_id, &callee, node_line(&child));
                }
            }
            "object_creation_expression" => {
                if let Some(t) = child.child_by_field_name("type") {
                    let callee = csharp_type_name(&t, ctx.code);
                    emit_call(ctx, func_id, &callee, node_line(&child));
                }
            }
            "local_function_statement" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, ctx.code).to_string())
                    .unwrap_or_else(|| "local".into());
                let arity = param_count(&child, "parameters");
                let line = node_line(&child);
                let local_id =
                    ctx.unique_id(scoped_id(func_id, &format!("{}_{}", name, arity)), line);
                emit_function(
                    ctx, &local_id, type_id, &name, arity, "private", line, &child,
                );
                if let Some(body) = child.child_by_field_name("body") {
                    analyze_csharp_calls(&body, &local_id, type_id, ctx)?;
                }
                continue;
            }
            _ => {}
        }
        analyze_csharp_calls(&child, func_id, type_id, ctx)?;
    }
    Ok(())
}

/// The callee of an invocation: `F(…)`, `F<T>(…)`, `x.F(…)`, `x?.F(…)`. A
/// simple receiver that names a type (`Helper.Twice`, `Console.WriteLine`) is
/// kept; an instance receiver is not part of the name.
fn csharp_callee(function: &Node<'_>, code: &[u8]) -> Option<String> {
    match function.kind() {
        "identifier" => Some(node_text(function, code).to_string()),
        "generic_name" => Some(csharp_type_name(function, code)),
        "member_access_expression" => {
            let name = function.child_by_field_name("name")?;
            let name = csharp_type_name(&name, code);
            let receiver = function.child_by_field_name("expression");
            match receiver {
                Some(r)
                    if r.kind() == "identifier"
                        && node_text(&r, code)
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_uppercase()) =>
                {
                    Some(format!("{}.{}", node_text(&r, code), name))
                }
                _ => Some(name),
            }
        }
        "member_binding_expression" => function
            .child_by_field_name("name")
            .map(|n| csharp_type_name(&n, code)),
        _ => None,
    }
}

// ========================================================================
// T-SQL Analysis
// ========================================================================
//
// Procedures, functions, views and triggers are functions of their schema's
// module (`dbo` when the name has none), annotated with their kind. A
// routine's calls are its EXEC targets and `name(…)` calls; its table access
// is `reads_table(Func, Table, Line)` (FROM, JOIN, comma-joined tables) and
// `writes_table(Func, Table, Line)` (INSERT, UPDATE, DELETE, MERGE,
// SELECT … INTO, TRUNCATE). `CREATE TABLE` makes the table a module.
// Statements outside any routine (deployment scripts, test setup) belong to
// the file's `script` function.

/// Words followed by `(` that are not calls: operators, clauses, constraint
/// and type syntax.
const TSQL_NOT_CALLS: &[&str] = &[
    "IN",
    "OVER",
    "AS",
    "AND",
    "OR",
    "NOT",
    "WHERE",
    "ON",
    "INTO",
    "TABLE",
    "KEY",
    "UNIQUE",
    "CHECK",
    "REFERENCES",
    "INCLUDE",
    "WITH",
    "OPTION",
    "INDEX",
    "DEFAULT",
    "FOREIGN",
    "PRIMARY",
    "CONSTRAINT",
    "VALUES",
    "EXISTS",
    "SET",
    "FROM",
    "JOIN",
    "APPLY",
    "BY",
    "IS",
    "THEN",
    "ELSE",
    "WHEN",
    "RETURNS",
    "OUTPUT",
    "PERCENT",
    "TOP",
    "PARTITION",
    "FOR",
    "XML",
    "PATH",
    "JSON",
    "CHAR",
    "VARCHAR",
    "NCHAR",
    "NVARCHAR",
    "BINARY",
    "VARBINARY",
    "DECIMAL",
    "NUMERIC",
    "FLOAT",
    "DATETIME2",
    "DATETIMEOFFSET",
    "TIME",
    "IDENTITY",
    "ROWS",
    "RANGE",
    "ROOT",
    "AUTO",
    "RAW",
    "EXPLICIT",
    "MAXDOP",
    "RECOMPILE",
];

/// Words that end a FROM list.
const TSQL_CLAUSE_WORDS: &[&str] = &[
    "WHERE",
    "GROUP",
    "ORDER",
    "HAVING",
    "ON",
    "OPTION",
    "SET",
    "OUTPUT",
    "UNION",
    "EXCEPT",
    "INTERSECT",
    "FOR",
    "WINDOW",
    "PIVOT",
    "UNPIVOT",
    "WHEN",
    "THEN",
    "USING",
];

/// `[dbo].[Orders]` → `dbo.Orders`; brackets and quotes off each part.
fn tsql_name(text: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '[' => {
                while let Some(d) = chars.next() {
                    if d == ']' {
                        if chars.peek() == Some(&']') {
                            chars.next();
                            cur.push(']');
                        } else {
                            break;
                        }
                    } else {
                        cur.push(d);
                    }
                }
            }
            '"' => {
                for d in chars.by_ref() {
                    if d == '"' {
                        break;
                    }
                    cur.push(d);
                }
            }
            '.' => parts.push(std::mem::take(&mut cur)),
            c if c.is_whitespace() => {}
            c => cur.push(c),
        }
    }
    parts.push(cur);
    parts.join(".")
}

struct TsqlScope<'s> {
    file_id: &'s str,
    script: Option<String>,
    schemas: std::collections::HashSet<String>,
}

fn analyze_tsql(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let (file_id, file_name) = module_id_from_path(ctx.filepath, ".sql", ".");
    emit_module(ctx, &file_id, &file_name, ctx.filepath, 1);
    let mut scope = TsqlScope {
        file_id: &file_id,
        script: None,
        schemas: std::collections::HashSet::new(),
    };
    let mut cursor = root.walk();
    let children: Vec<Node<'_>> = root.named_children(&mut cursor).collect();
    for child in children {
        match child.kind() {
            "create_procedure" | "create_function" | "create_view" | "create_trigger" => {
                tsql_routine(&child, &mut scope, ctx)
            }
            "batch_separator" | "sqlcmd_command" | "comment" => {}
            _ => {
                if child.kind() == "create_table" {
                    tsql_table_module(&child, &mut scope, ctx);
                }
                let id = match &scope.script {
                    Some(id) => id.clone(),
                    None => {
                        let id = scoped_id(scope.file_id, "script_0");
                        let module = scope.file_id.to_string();
                        emit_function(
                            ctx,
                            &id,
                            &module,
                            "script",
                            0,
                            "public",
                            node_line(&child),
                            &child,
                        );
                        scope.script = Some(id.clone());
                        id
                    }
                };
                tsql_walk(&child, &id, ctx);
            }
        }
    }
    Ok(())
}

/// The schema's module, emitted once per file: (module id, object name).
fn tsql_schema_module(
    qualified: &str,
    line: usize,
    scope: &mut TsqlScope<'_>,
    ctx: &mut Ctx<'_>,
) -> (String, String) {
    let parts: Vec<&str> = qualified.split('.').collect();
    let object = parts.last().copied().unwrap_or(qualified).to_string();
    let schema = if parts.len() >= 2 {
        parts[parts.len() - 2]
    } else {
        "dbo"
    };
    let schema = if schema.is_empty() { "dbo" } else { schema };
    let schema_id = normalize_id(schema);
    if scope.schemas.insert(schema_id.clone()) {
        emit_module(ctx, &schema_id, schema, &format!("schema:{}", schema), line);
    }
    (schema_id, object)
}

fn tsql_table_module(node: &Node<'_>, scope: &mut TsqlScope<'_>, ctx: &mut Ctx<'_>) {
    let Some(name) = node.child_by_field_name("name") else {
        return;
    };
    if name.kind() != "object_name" {
        return; // #temp and @table variables are not part of the schema
    }
    let qualified = tsql_name(node_text(&name, ctx.code));
    let (schema_id, object) = tsql_schema_module(&qualified, node_line(node), scope, ctx);
    let table_id = scoped_id(&schema_id, &object);
    let label = format!(
        "{}.{}",
        qualified.rsplit_once('.').map(|(s, _)| s).unwrap_or("dbo"),
        object
    );
    emit_module(
        ctx,
        &table_id,
        &label,
        &format!("table:{}", label),
        node_line(node),
    );
}

fn tsql_routine(node: &Node<'_>, scope: &mut TsqlScope<'_>, ctx: &mut Ctx<'_>) {
    let Some(name) = node.child_by_field_name("name") else {
        return;
    };
    let qualified = tsql_name(node_text(&name, ctx.code));
    let line = node_line(node);
    let (schema_id, object) = tsql_schema_module(&qualified, line, scope, ctx);
    let arity = node
        .child_by_field_name("parameters")
        .map(|p| {
            let mut c = p.walk();
            let n = p
                .named_children(&mut c)
                .filter(|x| x.kind() == "parameter")
                .count();
            n
        })
        .unwrap_or(0);
    let func_id = ctx.unique_id(
        scoped_id(&schema_id, &format!("{}_{}", object, arity)),
        line,
    );
    emit_function(
        ctx, &func_id, &schema_id, &object, arity, "public", line, node,
    );
    let kind = node.kind().trim_start_matches("create_").to_string();
    ctx.facts.push(Fact::new(
        "annotation",
        vec![
            FactValue::String(func_id.clone()),
            FactValue::String(kind),
            FactValue::String(String::new()),
        ],
    ));
    if node.kind() == "create_trigger" {
        if let Some(t) = node.child_by_field_name("table") {
            let table = tsql_name(node_text(&t, ctx.code));
            ctx.facts.push(Fact::new(
                "annotation",
                vec![
                    FactValue::String(func_id.clone()),
                    FactValue::String("on".to_string()),
                    FactValue::String(table),
                ],
            ));
        }
    }
    for field in ["body", "returns"] {
        if let Some(body) = node.child_by_field_name(field) {
            tsql_walk(&body, &func_id, ctx);
        }
    }
}

fn tsql_table_fact(ctx: &mut Ctx<'_>, func_id: &str, table: &Node<'_>, predicate: &str) {
    if table.kind() == "variable" {
        return; // @table variables are local
    }
    let name = tsql_name(node_text(table, ctx.code));
    ctx.facts.push(Fact::new(
        predicate,
        vec![
            FactValue::String(func_id.to_string()),
            FactValue::String(name),
            FactValue::Integer(node_line(table) as i64),
        ],
    ));
}

/// Calls and table access in a statement or body, in source order.
fn tsql_walk(node: &Node<'_>, func_id: &str, ctx: &mut Ctx<'_>) {
    let kind = node.kind();
    // The table a data statement writes, which is not also a read or a call.
    let target = node.child_by_field_name("table");
    if matches!(
        kind,
        "insert_statement" | "update_statement" | "delete_statement" | "merge_statement"
    ) {
        if let Some(t) = target {
            tsql_table_fact(ctx, func_id, &t, "writes_table");
        }
    }
    if kind == "exec_statement" {
        if let Some(p) = node.child_by_field_name("procedure") {
            if p.kind() == "object_name" {
                let name = tsql_name(node_text(&p, ctx.code));
                emit_call(ctx, func_id, &name, node_line(&p));
            }
        }
    }
    if kind == "other_statement" {
        // TRUNCATE TABLE t
        let first = node
            .child(0)
            .map(|n| n.kind().to_string())
            .unwrap_or_default();
        if first == "TRUNCATE" {
            let mut c = node.walk();
            if let Some(t) = node
                .named_children(&mut c)
                .find(|n| matches!(n.kind(), "object_name" | "temp_name"))
            {
                tsql_table_fact(ctx, func_id, &t, "writes_table");
            }
            return;
        }
    }

    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
    let skip_ids: Vec<usize> = [
        target,
        node.child_by_field_name("procedure"),
        node.child_by_field_name("name"),
    ]
    .into_iter()
    .flatten()
    .map(|n| n.id())
    .collect();
    // Some("read"/"write") when the next name is a table.
    let mut expect: Option<&'static str> = None;
    let mut in_from = false;
    for (i, c) in children.iter().enumerate() {
        if skip_ids.contains(&c.id()) {
            continue;
        }
        let next_is_paren = children
            .get(i + 1)
            .is_some_and(|n| n.kind() == "parenthesized");
        match c.kind() {
            "FROM" => {
                expect = Some("reads_table");
                in_from = true;
            }
            "INTO" if kind == "select_statement" => expect = Some("writes_table"),
            "," if in_from => expect = Some("reads_table"),
            "object_name" => {
                let text = node_text(c, ctx.code);
                let upper = text.to_ascii_uppercase();
                if matches!(upper.as_str(), "JOIN" | "APPLY") {
                    expect = Some("reads_table");
                    in_from = true;
                    continue;
                }
                if TSQL_CLAUSE_WORDS.contains(&upper.as_str()) {
                    in_from = false;
                    expect = None;
                    continue;
                }
                match expect.take() {
                    Some(_) if next_is_paren => {
                        // FROM dbo.fn(@x): a table-valued function
                        emit_call(ctx, func_id, &tsql_name(text), node_line(c));
                    }
                    Some(pred) => tsql_table_fact(ctx, func_id, c, pred),
                    None if next_is_paren && !TSQL_NOT_CALLS.contains(&upper.as_str()) => {
                        emit_call(ctx, func_id, &tsql_name(text), node_line(c));
                    }
                    None => {}
                }
            }
            "temp_name" | "variable" => {
                if let Some(pred) = expect.take() {
                    tsql_table_fact(ctx, func_id, c, pred);
                }
            }
            "parenthesized" => {
                expect = None;
                tsql_walk(c, func_id, ctx);
            }
            _ if c.named_child_count() > 0 => tsql_walk(c, func_id, ctx),
            _ => {}
        }
    }
}

// ========================================================================
// PowerShell Analysis
// ========================================================================
//
// `function` and `filter` definitions are functions of the file's module,
// with their `param()` parameters; classes are modules with methods and
// constructors. Calls are command names (cmdlets and functions alike),
// `.Method()` and `[Type]::Method()`; calls in script-block arguments
// (`Where-Object { … }`) belong to the enclosing function. `Import-Module`,
// `using module`, `using namespace`, `#Requires -Modules` and dot-sourced
// paths are dependencies. Top-level statements are the file's `script`.

fn analyze_powershell(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let ext = if ctx.filepath.ends_with(".psm1") {
        ".psm1"
    } else {
        ".ps1"
    };
    let (file_id, file_name) = module_id_from_path(ctx.filepath, ext, ".");
    emit_module(ctx, &file_id, &file_name, ctx.filepath, 1);
    let mut script: Option<String> = None;

    let mut cursor = root.walk();
    let children: Vec<Node<'_>> = root.named_children(&mut cursor).collect();
    for child in children {
        if child.kind() == "comment" {
            ps_requires(&child, &file_id, ctx);
            continue;
        }
        ps_top(&child, &file_id, &mut script, ctx);
    }
    Ok(())
}

/// A top-level node: definitions stand alone, anything else is the script.
fn ps_top(node: &Node<'_>, file_id: &str, script: &mut Option<String>, ctx: &mut Ctx<'_>) {
    match node.kind() {
        "statement_list" => {
            let mut cursor = node.walk();
            let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
            for c in children {
                ps_top(&c, file_id, script, ctx);
            }
        }
        "function_statement" => ps_function(node, file_id, file_id, ctx),
        "class_statement" => ps_class(node, ctx),
        "enum_statement" => {
            if let Some(name) = find_child_by_kind(node, "simple_name") {
                let name = node_text(&name, ctx.code).to_string();
                let id = scoped_id(&normalize_id(ctx.filepath), &name);
                emit_module(ctx, &id, &name, &format!("enum:{}", name), node_line(node));
            }
        }
        "comment" => ps_requires(node, file_id, ctx),
        _ => {
            let id = match script {
                Some(id) => id.clone(),
                None => {
                    let id = scoped_id(file_id, "script_0");
                    emit_function(
                        ctx,
                        &id,
                        file_id,
                        "script",
                        0,
                        "public",
                        node_line(node),
                        node,
                    );
                    *script = Some(id.clone());
                    id
                }
            };
            ps_walk(node, &id, file_id, ctx);
        }
    }
}

/// `#Requires -Modules A, B` names modules the script needs.
fn ps_requires(comment: &Node<'_>, file_id: &str, ctx: &mut Ctx<'_>) {
    let text = node_text(comment, ctx.code);
    let Some(rest) = text
        .strip_prefix("#Requires")
        .or_else(|| text.strip_prefix("#requires"))
    else {
        return;
    };
    let Some(i) = rest.to_ascii_lowercase().find("-modules") else {
        return;
    };
    let line = node_line(comment);
    let list = &rest[i + "-modules".len()..];
    // names and @{ ModuleName = …; ModuleVersion = … } tables, comma-separated
    let mut depth = 0;
    let mut parts = vec![String::new()];
    for ch in list.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(String::new());
                continue;
            }
            _ => {}
        }
        if let Some(p) = parts.last_mut() {
            p.push(ch);
        }
    }
    for part in parts {
        let part = part.trim();
        let name = if part.starts_with("@{") {
            ps_requires_module_name(part)
        } else {
            let n = part
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c| c == '"' || c == '\'');
            (!n.is_empty() && !n.starts_with('-')).then(|| n.to_string())
        };
        if let Some(n) = name {
            emit_dependency(ctx, file_id, &n, "requires", line);
        }
    }
}

/// `ModuleName = "Pester"` in a `#Requires -Modules @{ … }` table.
fn ps_requires_module_name(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let i = lower.find("modulename")?;
    let after = &text[i + "modulename".len()..];
    let after = after.trim_start().strip_prefix('=')?.trim_start();
    let q = after.chars().next()?;
    if q == '"' || q == '\'' {
        let rest = &after[1..];
        rest.find(q).map(|j| rest[..j].to_string())
    } else {
        Some(
            after
                .split(|c: char| c == ';' || c == '}' || c.is_whitespace())
                .next()?
                .to_string(),
        )
    }
}

/// The parameters of a function or script block: its `param()` block, or
/// `function F($a, $b)`.
fn ps_parameters<'a>(node: &Node<'a>) -> Vec<Node<'a>> {
    let mut out = Vec::new();
    let mut stack = vec![*node];
    while let Some(n) = stack.pop() {
        let mut c = n.walk();
        for child in n.named_children(&mut c) {
            match child.kind() {
                "script_parameter" | "class_method_parameter" => out.push(child),
                // the function's own parameter list and param block, not nested blocks
                "function_parameter_declaration"
                | "parameter_list"
                | "param_block"
                | "class_method_parameter_list" => stack.push(child),
                "script_block" if n.kind() == "function_statement" => stack.push(child),
                _ => {}
            }
        }
    }
    out.sort_by_key(|p| p.start_byte());
    out
}

fn ps_emit_params(params: &[Node<'_>], func_id: &str, ctx: &mut Ctx<'_>) {
    for (pos, p) in params.iter().enumerate() {
        let Some(var) = find_child_by_kind(p, "variable") else {
            continue;
        };
        let ty = {
            let mut found = None;
            let mut stack = vec![*p];
            while let Some(n) = stack.pop() {
                if n.kind() == "type_literal" {
                    found = Some(
                        node_text(&n, ctx.code)
                            .trim_matches(|c| c == '[' || c == ']')
                            .to_string(),
                    );
                    break;
                }
                if n.kind() == "script_parameter_default" {
                    continue;
                }
                let mut c = n.walk();
                stack.extend(n.named_children(&mut c));
            }
            found
        };
        ctx.facts.push(Fact::new(
            "function_param",
            vec![
                FactValue::String(func_id.to_string()),
                FactValue::Integer(pos as i64),
                FactValue::String(node_text(&var, ctx.code).to_string()),
                match ty {
                    Some(t) => FactValue::String(clean_param(&t)),
                    None => FactValue::Atom("unknown".to_string()),
                },
            ],
        ));
    }
}

/// `[CmdletBinding()]`, `[OutputType(...)]` on a function's param block.
fn ps_annotations(node: &Node<'_>, id: &str, ctx: &mut Ctx<'_>) {
    let Some(block) = find_child_by_kind(node, "script_block") else {
        return;
    };
    let Some(param_block) = find_child_by_kind(&block, "param_block") else {
        return;
    };
    let Some(list) = find_child_by_kind(&param_block, "attribute_list") else {
        return;
    };
    let mut c = list.walk();
    let attrs: Vec<Node<'_>> = list.named_children(&mut c).collect();
    for a in attrs {
        let Some(name) = find_child_by_kind(&a, "attribute_name") else {
            continue;
        };
        let args = find_child_by_kind(&a, "attribute_arguments")
            .map(|x| clean_param(node_text(&x, ctx.code)))
            .unwrap_or_default();
        ctx.facts.push(Fact::new(
            "annotation",
            vec![
                FactValue::String(id.to_string()),
                FactValue::String(node_text(&name, ctx.code).to_string()),
                FactValue::String(args),
            ],
        ));
    }
}

fn ps_function(node: &Node<'_>, scope: &str, module_id: &str, ctx: &mut Ctx<'_>) {
    let name = find_child_by_kind(node, "function_name")
        .map(|n| node_text(&n, ctx.code).to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let params = ps_parameters(node);
    let line = node_line(node);
    let func_id = ctx.unique_id(
        scoped_id(scope, &format!("{}_{}", name, params.len())),
        line,
    );
    emit_function(
        ctx,
        &func_id,
        module_id,
        &name,
        params.len(),
        "public",
        line,
        node,
    );
    ps_emit_params(&params, &func_id, ctx);
    ps_annotations(node, &func_id, ctx);
    if let Some(block) = find_child_by_kind(node, "script_block") {
        ps_walk(&block, &func_id, module_id, ctx);
    }
}

fn ps_class(node: &Node<'_>, ctx: &mut Ctx<'_>) {
    let mut c = node.walk();
    let names: Vec<Node<'_>> = node
        .named_children(&mut c)
        .filter(|n| n.kind() == "simple_name")
        .collect();
    let Some(first) = names.first() else { return };
    let class_name = node_text(first, ctx.code).to_string();
    let class_id = scoped_id(&normalize_id(ctx.filepath), &class_name);
    emit_module(
        ctx,
        &class_id,
        &class_name,
        &format!("class:{}", class_name),
        node_line(node),
    );
    // `class A : B, IC` — read from the header text: the grammar does not
    // parse a qualified base (`System.Exception`) into names.
    let text = node_text(node, ctx.code);
    let header = text.split('{').next().unwrap_or("");
    if let Some((_, bases)) = header.split_once(':') {
        for b in bases.split(',').map(str::trim).filter(|b| !b.is_empty()) {
            emit_dependency(ctx, &class_id, b, "inherits", node_line(node));
        }
    }
    let mut c = node.walk();
    let methods: Vec<Node<'_>> = node
        .named_children(&mut c)
        .filter(|n| n.kind() == "class_method_definition")
        .collect();
    for m in methods {
        let Some(mname) = find_child_by_kind(&m, "simple_name") else {
            continue;
        };
        let mname = node_text(&mname, ctx.code).to_string();
        let params = ps_parameters(&m);
        let line = node_line(&m);
        let text = node_text(&m, ctx.code).trim_start().to_ascii_lowercase();
        let visibility = if text.starts_with("hidden") {
            "private"
        } else {
            "public"
        };
        let func_id = ctx.unique_id(
            scoped_id(&class_id, &format!("{}_{}", mname, params.len())),
            line,
        );
        emit_function(
            ctx,
            &func_id,
            &class_id,
            &mname,
            params.len(),
            visibility,
            line,
            &m,
        );
        ps_emit_params(&params, &func_id, ctx);
        if let Some(block) = find_child_by_kind(&m, "script_block") {
            ps_walk(&block, &func_id, &class_id, ctx);
        }
    }
}

/// `Import-Module X`, `using module X`, `using namespace X`, `. ./x.ps1`.
fn ps_command_dependency(cmd: &Node<'_>, name: &str, module_id: &str, ctx: &mut Ctx<'_>) {
    let lower = name.to_ascii_lowercase();
    let args: Vec<Node<'_>> = match cmd.child_by_field_name("command_elements") {
        Some(e) => {
            let mut c = e.walk();
            e.named_children(&mut c)
                .filter(|n| n.kind() != "command_argument_sep")
                .collect()
        }
        None => Vec::new(),
    };
    let text_of = |n: &Node<'_>| {
        node_text(n, ctx.code)
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string()
    };
    let line = node_line(cmd);
    match lower.as_str() {
        "import-module" | "ipmo" => {
            // the first argument that is not a -Parameter or its value
            let mut skip_next = false;
            for a in &args {
                if a.kind() == "command_parameter" {
                    let p = node_text(a, ctx.code).to_ascii_lowercase();
                    skip_next = !matches!(p.as_str(), "-name" | "-force" | "-global" | "-passthru");
                    continue;
                }
                if skip_next {
                    skip_next = false;
                    continue;
                }
                let t = text_of(a);
                emit_dependency(ctx, module_id, &t, "import", line);
                break;
            }
        }
        "using" if args.len() >= 2 => {
            let kind = text_of(&args[0]).to_ascii_lowercase();
            if kind == "module" || kind == "namespace" || kind == "assembly" {
                let t = text_of(&args[1]);
                emit_dependency(ctx, module_id, &t, "using", line);
            }
        }
        _ => {}
    }
}

/// Calls and dependencies in a body; nested functions are their own.
fn ps_walk(node: &Node<'_>, func_id: &str, module_id: &str, ctx: &mut Ctx<'_>) {
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    for child in children {
        match child.kind() {
            "function_statement" => {
                ps_function(&child, func_id, module_id, ctx);
                continue;
            }
            "class_statement" => {
                ps_class(&child, ctx);
                continue;
            }
            "command" => {
                let dot = find_child_by_kind(&child, "command_invokation_operator")
                    .map(|o| node_text(&o, ctx.code).trim() == ".")
                    .unwrap_or(false);
                if let Some(name) = child.child_by_field_name("command_name") {
                    if name.kind() == "command_name" {
                        let n = node_text(&name, ctx.code).to_string();
                        // `using module X` is a statement the grammar reads as a command
                        if !n.eq_ignore_ascii_case("using") {
                            emit_call(ctx, func_id, &n, node_line(&child));
                        }
                        ps_command_dependency(&child, &n, module_id, ctx);
                    } else if dot {
                        // . "$PSScriptRoot\private\Helpers.ps1": dot-sourcing a file
                        let path = node_text(&name, ctx.code)
                            .trim()
                            .trim_matches(|c| c == '"' || c == '\'')
                            .replace('\\', "/");
                        emit_dependency(ctx, module_id, &path, "dot_source", node_line(&child));
                    }
                }
            }
            "invokation_expression" => {
                let mut c = child.walk();
                let parts: Vec<Node<'_>> = child.named_children(&mut c).collect();
                let method = parts
                    .iter()
                    .rfind(|p| p.kind() == "member_name")
                    .map(|m| node_text(m, ctx.code).to_string());
                if let Some(method) = method {
                    let callee = match parts.first() {
                        Some(t) if t.kind() == "type_literal" => {
                            let ty = node_text(t, ctx.code)
                                .trim_matches(|c| c == '[' || c == ']')
                                .to_string();
                            if method.eq_ignore_ascii_case("new") {
                                ty
                            } else {
                                format!("{}::{}", ty, method)
                            }
                        }
                        _ => method,
                    };
                    emit_call(ctx, func_id, &callee, node_line(&child));
                }
            }
            _ => {}
        }
        ps_walk(&child, func_id, module_id, ctx);
    }
}

// ========================================================================
// Java Analysis
// ========================================================================
//
// As C#: a type's module id is its package-qualified name with its type
// parameters; methods and constructors (named for the type) are functions;
// annotations are `annotation(Id, Name, Args)` (Spring's @GetMapping routes,
// @Transactional); imports and extends/implements are dependencies. Package-
// private members have visibility `package`. Methods of anonymous and local
// classes are functions of their own, scoped by the enclosing method.

const JAVA_TYPES: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "record_declaration",
    "annotation_type_declaration",
];

fn analyze_java(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let (file_id, file_name) = module_id_from_path(ctx.filepath, ".java", ".");
    emit_module(ctx, &file_id, &file_name, ctx.filepath, 1);
    let mut package = String::new();
    let mut cursor = root.walk();
    let children: Vec<Node<'_>> = root.named_children(&mut cursor).collect();
    for child in children {
        match child.kind() {
            "package_declaration" => {
                let mut c = child.walk();
                let found = child
                    .named_children(&mut c)
                    .find(|n| matches!(n.kind(), "scoped_identifier" | "identifier"));
                if let Some(n) = found {
                    package = node_text(&n, ctx.code).to_string();
                }
            }
            "import_declaration" => {
                let text = node_text(&child, ctx.code);
                let target = text
                    .trim_start_matches("import")
                    .trim()
                    .trim_start_matches("static")
                    .trim()
                    .trim_end_matches(';')
                    .trim()
                    .to_string();
                emit_dependency(ctx, &file_id, &target, "import", node_line(&child));
            }
            k if JAVA_TYPES.contains(&k) => java_type(&child, &package, ctx),
            _ => {}
        }
    }
    Ok(())
}

/// Annotations among a declaration's modifiers, as annotation facts.
fn jvm_annotations(node: &Node<'_>, id: &str, ctx: &mut Ctx<'_>) {
    let Some(mods) = find_child_by_kind(node, "modifiers") else {
        return;
    };
    let mut c = mods.walk();
    let anns: Vec<Node<'_>> = mods
        .named_children(&mut c)
        .filter(|a| matches!(a.kind(), "marker_annotation" | "annotation"))
        .collect();
    for a in anns {
        // Java names the annotation in a field; Kotlin's is `@Name(args)` text.
        let name = a
            .child_by_field_name("name")
            .map(|n| node_text(&n, ctx.code).to_string())
            .unwrap_or_else(|| {
                let t = node_text(&a, ctx.code).trim_start_matches('@');
                t.split(['(', ' ', '\n']).next().unwrap_or("").to_string()
            });
        let kotlin_args = (a.child_by_field_name("name").is_none())
            .then(|| {
                let t = node_text(&a, ctx.code);
                t.find('(')
                    .map(|i| clean_param(t[i + 1..].trim_end().trim_end_matches(')')))
            })
            .flatten();
        let args = a
            .child_by_field_name("arguments")
            .map(|x| {
                let t = node_text(&x, ctx.code);
                clean_param(t.trim_start_matches('(').trim_end_matches(')'))
            })
            .or(kotlin_args)
            .unwrap_or_default();
        if !name.is_empty() {
            ctx.facts.push(Fact::new(
                "annotation",
                vec![
                    FactValue::String(id.to_string()),
                    FactValue::String(name),
                    FactValue::String(args),
                ],
            ));
        }
    }
}

fn java_visibility(node: &Node<'_>, code: &[u8], in_interface: bool) -> &'static str {
    let mods = find_child_by_kind(node, "modifiers")
        .map(|m| node_text(&m, code).to_string())
        .unwrap_or_default();
    let words: Vec<&str> = mods.split_whitespace().collect();
    if words.contains(&"public") || in_interface {
        "public"
    } else if words.contains(&"protected") {
        "protected"
    } else if words.contains(&"private") {
        "private"
    } else {
        "package"
    }
}

fn java_type_name(node: &Node<'_>, code: &[u8]) -> String {
    let text = node_text(node, code);
    text.split('<').next().unwrap_or(text).trim().to_string()
}

fn java_type(node: &Node<'_>, outer: &str, ctx: &mut Ctx<'_>) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(&n, ctx.code).to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    let generic = node
        .child_by_field_name("type_parameters")
        .map(|tp| {
            let mut c = tp.walk();
            let names: Vec<String> = tp
                .named_children(&mut c)
                .filter(|p| p.kind() == "type_parameter")
                .filter_map(|p| {
                    first_identifier_text(&p, ctx.code).or_else(|| {
                        let mut cc = p.walk();
                        let found = p
                            .named_children(&mut cc)
                            .find(|x| x.kind() == "type_identifier")
                            .map(|x| node_text(&x, ctx.code).to_string());
                        found
                    })
                })
                .collect();
            format!("<{}>", names.join(","))
        })
        .unwrap_or_default();
    let qualified = dotted(outer, &format!("{}{}", name, generic));
    let type_id = normalize_id(&qualified);
    let label = node.kind().trim_end_matches("_declaration");
    emit_module(
        ctx,
        &type_id,
        &qualified,
        &format!("{}:{}", label, qualified),
        node_line(node),
    );
    jvm_annotations(node, &type_id, ctx);

    let mut first_base: Option<String> = None;
    for field in ["superclass", "interfaces"] {
        if let Some(b) = node.child_by_field_name(field) {
            let mut stack = vec![b];
            while let Some(n) = stack.pop() {
                if matches!(
                    n.kind(),
                    "type_identifier" | "scoped_type_identifier" | "generic_type"
                ) {
                    let base = java_type_name(&n, ctx.code);
                    if field == "superclass" && first_base.is_none() {
                        first_base = Some(base.clone());
                    }
                    emit_dependency(ctx, &type_id, &base, "inherits", node_line(&n));
                    continue;
                }
                let mut c = n.walk();
                let mut kids: Vec<Node<'_>> = n.named_children(&mut c).collect();
                kids.reverse();
                stack.extend(kids);
            }
        }
    }
    // interface I extends A, B
    if let Some(ext) = find_child_by_kind(node, "extends_interfaces") {
        let mut stack = vec![ext];
        while let Some(n) = stack.pop() {
            if matches!(
                n.kind(),
                "type_identifier" | "scoped_type_identifier" | "generic_type"
            ) {
                let base = java_type_name(&n, ctx.code);
                emit_dependency(ctx, &type_id, &base, "inherits", node_line(&n));
                continue;
            }
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c));
        }
    }

    let in_interface = matches!(
        node.kind(),
        "interface_declaration" | "annotation_type_declaration"
    );
    let Some(body) = node.child_by_field_name("body") else {
        return;
    };
    java_members(
        &body,
        &type_id,
        &qualified,
        &name,
        first_base.as_deref(),
        in_interface,
        ctx,
    );
}

/// The members of a class, interface, enum or record body.
#[allow(clippy::too_many_arguments)]
fn java_members(
    body: &Node<'_>,
    type_id: &str,
    qualified: &str,
    type_name: &str,
    first_base: Option<&str>,
    in_interface: bool,
    ctx: &mut Ctx<'_>,
) {
    let mut cursor = body.walk();
    let members: Vec<Node<'_>> = body.named_children(&mut cursor).collect();
    for member in members {
        let kind = member.kind();
        if JAVA_TYPES.contains(&kind) {
            java_type(&member, qualified, ctx);
            continue;
        }
        if kind == "enum_body_declarations" {
            java_members(
                &member,
                type_id,
                qualified,
                type_name,
                first_base,
                in_interface,
                ctx,
            );
            continue;
        }
        let (fname, arity) = match kind {
            "method_declaration" | "annotation_type_element_declaration" => (
                member
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, ctx.code).to_string())
                    .unwrap_or_else(|| "unknown".into()),
                param_count(&member, "parameters"),
            ),
            "constructor_declaration" => {
                (type_name.to_string(), param_count(&member, "parameters"))
            }
            // record R(int a) { R { … } }: the canonical constructor's arity is the record's
            "compact_constructor_declaration" => (type_name.to_string(), 0),
            _ => continue,
        };
        let line = node_line(&member);
        let func_id = ctx.unique_id(scoped_id(type_id, &format!("{}_{}", fname, arity)), line);
        let visibility = java_visibility(&member, ctx.code, in_interface);
        emit_function(
            ctx, &func_id, type_id, &fname, arity, visibility, line, &member,
        );
        jvm_annotations(&member, &func_id, ctx);
        if let Some(b) = member.child_by_field_name("body") {
            java_calls(&b, &func_id, type_id, type_name, first_base, ctx);
        }
    }
}

fn java_calls(
    node: &Node<'_>,
    func_id: &str,
    type_id: &str,
    type_name: &str,
    first_base: Option<&str>,
    ctx: &mut Ctx<'_>,
) {
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    for child in children {
        match child.kind() {
            "method_invocation" => {
                if let Some(name) = child.child_by_field_name("name") {
                    let name = node_text(&name, ctx.code).to_string();
                    let callee = match child.child_by_field_name("object") {
                        Some(o)
                            if o.kind() == "identifier"
                                && node_text(&o, ctx.code)
                                    .chars()
                                    .next()
                                    .is_some_and(|c| c.is_ascii_uppercase()) =>
                        {
                            format!("{}.{}", node_text(&o, ctx.code), name)
                        }
                        _ => name,
                    };
                    emit_call(ctx, func_id, &callee, node_line(&child));
                }
            }
            "object_creation_expression" => {
                if let Some(t) = child.child_by_field_name("type") {
                    let callee = java_type_name(&t, ctx.code);
                    emit_call(ctx, func_id, &callee, node_line(&child));
                }
                // new Runnable() { public void run() { … } }: its methods are functions
                if let Some(body) = find_child_by_kind(&child, "class_body") {
                    let anon = scoped_id(func_id, &format!("anon_l{}", node_line(&child)));
                    java_members(&body, &anon, &anon, "anonymous", None, false, ctx);
                }
                // the constructor's arguments
                if let Some(args) = child.child_by_field_name("arguments") {
                    java_calls(&args, func_id, type_id, type_name, first_base, ctx);
                }
                continue;
            }
            "explicit_constructor_invocation" => {
                let text = node_text(&child, ctx.code).trim_start();
                let target = if text.starts_with("super") {
                    first_base
                        .map(|b| b.rsplit('.').next().unwrap_or(b).to_string())
                        .unwrap_or_else(|| "super".to_string())
                } else {
                    type_name.to_string()
                };
                emit_call(ctx, func_id, &target, node_line(&child));
            }
            "local_class_declaration" | "class_declaration" => {
                java_type(&child, type_id, ctx);
                continue;
            }
            _ => {}
        }
        java_calls(&child, func_id, type_id, type_name, first_base, ctx);
    }
}

// ========================================================================
// Kotlin Analysis
// ========================================================================
//
// Classes, interfaces, objects and companion objects are modules named by
// package (a companion is `Outer.Companion`); top-level functions belong to
// the file's module. Member, top-level and local functions, primary and
// secondary constructors (named for the class) and properties with custom
// accessors are functions; an extension function keeps its name and its
// receiver is an annotation. Delegation specifiers are `inherits`, imports
// `import`. Statements in scripts (`.kts`, Gradle) are the file's `script`.

fn analyze_kotlin(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let ext = if ctx.filepath.ends_with(".kts") {
        ".kts"
    } else {
        ".kt"
    };
    let (file_id, file_name) = module_id_from_path(ctx.filepath, ext, ".");
    emit_module(ctx, &file_id, &file_name, ctx.filepath, 1);
    let mut package = String::new();
    let mut script: Option<String> = None;
    let mut cursor = root.walk();
    let children: Vec<Node<'_>> = root.named_children(&mut cursor).collect();
    for child in children {
        match child.kind() {
            "package_header" => {
                if let Some(id) = find_child_by_kind(&child, "identifier") {
                    package = node_text(&id, ctx.code).to_string();
                }
            }
            "import_list" | "import_header" => {
                let headers: Vec<Node<'_>> = if child.kind() == "import_list" {
                    let mut c = child.walk();
                    child.named_children(&mut c).collect()
                } else {
                    vec![child]
                };
                for h in headers {
                    if let Some(id) = find_child_by_kind(&h, "identifier") {
                        let mut t = node_text(&id, ctx.code).to_string();
                        if find_child_by_kind(&h, "wildcard_import").is_some() {
                            t.push_str(".*");
                        }
                        emit_dependency(ctx, &file_id, &t, "import", node_line(&h));
                    }
                }
            }
            "class_declaration" | "object_declaration" => kotlin_type(&child, &package, ctx),
            "function_declaration" | "property_declaration" => {
                kotlin_member(&child, &file_id, "", ctx);
            }
            "comment" | "multiline_comment" | "line_comment" | "shebang_line"
            | "file_annotation" => {}
            _ => {
                let id = match &script {
                    Some(id) => id.clone(),
                    None => {
                        let id = scoped_id(&file_id, "script_0");
                        emit_function(
                            ctx,
                            &id,
                            &file_id,
                            "script",
                            0,
                            "public",
                            node_line(&child),
                            &child,
                        );
                        script = Some(id.clone());
                        id
                    }
                };
                kotlin_calls(&child, &id, &file_id, ctx);
            }
        }
    }
    Ok(())
}

fn kotlin_visibility(node: &Node<'_>, code: &[u8]) -> &'static str {
    let Some(mods) = find_child_by_kind(node, "modifiers") else {
        return "public";
    };
    match find_child_by_kind(&mods, "visibility_modifier").map(|v| node_text(&v, code).to_string())
    {
        Some(v) if v == "private" => "private",
        Some(v) if v == "protected" => "protected",
        Some(v) if v == "internal" => "internal",
        _ => "public",
    }
}

fn kotlin_type(node: &Node<'_>, outer: &str, ctx: &mut Ctx<'_>) {
    let name = match node.kind() {
        "companion_object" => find_child_by_kind(node, "type_identifier")
            .map(|n| node_text(&n, ctx.code).to_string())
            .unwrap_or_else(|| "Companion".to_string()),
        _ => find_child_by_kind(node, "type_identifier")
            .map(|n| node_text(&n, ctx.code).to_string())
            .unwrap_or_else(|| "Unknown".to_string()),
    };
    let qualified = dotted(outer, &name);
    let type_id = normalize_id(&qualified);
    let text = node_text(node, ctx.code);
    let label = if node.kind() == "class_declaration" {
        let head = text.split(['{', '(']).next().unwrap_or("");
        if head.split_whitespace().any(|w| w == "interface") {
            "interface"
        } else {
            "class"
        }
    } else {
        "object"
    };
    emit_module(
        ctx,
        &type_id,
        &qualified,
        &format!("{}:{}", label, qualified),
        node_line(node),
    );
    jvm_annotations(node, &type_id, ctx);

    let mut c = node.walk();
    let parts: Vec<Node<'_>> = node.named_children(&mut c).collect();
    for p in &parts {
        match p.kind() {
            "delegation_specifier" | "delegation_specifiers" => {
                let mut stack = vec![*p];
                while let Some(n) = stack.pop() {
                    if n.kind() == "user_type" {
                        let base = node_text(&n, ctx.code);
                        let base = base.split('<').next().unwrap_or(base).trim().to_string();
                        emit_dependency(ctx, &type_id, &base, "inherits", node_line(&n));
                        continue;
                    }
                    let mut cc = n.walk();
                    stack.extend(n.named_children(&mut cc));
                }
            }
            "primary_constructor" => {
                let arity = {
                    let mut cc = p.walk();
                    let n = p
                        .named_children(&mut cc)
                        .filter(|x| x.kind() == "class_parameter")
                        .count();
                    n
                };
                let line = node_line(p);
                let id = ctx.unique_id(scoped_id(&type_id, &format!("{}_{}", name, arity)), line);
                emit_function(
                    ctx,
                    &id,
                    &type_id,
                    &name,
                    arity,
                    kotlin_visibility(p, ctx.code),
                    line,
                    p,
                );
            }
            "class_body" | "enum_class_body" => kotlin_body(p, &type_id, &qualified, &name, ctx),
            _ => {}
        }
    }
}

fn kotlin_body(
    body: &Node<'_>,
    type_id: &str,
    qualified: &str,
    type_name: &str,
    ctx: &mut Ctx<'_>,
) {
    let mut c = body.walk();
    let members: Vec<Node<'_>> = body.named_children(&mut c).collect();
    // The property a following stray getter/setter belongs to, and its id once emitted.
    let mut last_property: Option<(Node<'_>, Option<String>)> = None;
    for m in members {
        match m.kind() {
            // An annotated accessor on its own line (`@Synchronized get() = …`)
            // parses as the property's sibling, not its child.
            "getter" | "setter" => {
                if let Some((prop, emitted)) = last_property.as_mut() {
                    let id = match emitted {
                        Some(id) => id.clone(),
                        None => {
                            let name = find_child_by_kind(prop, "variable_declaration")
                                .and_then(|v| find_child_by_kind(&v, "simple_identifier"))
                                .map(|n| node_text(&n, ctx.code).to_string())
                                .unwrap_or_else(|| "unknown".to_string());
                            let line = node_line(prop);
                            let id =
                                ctx.unique_id(scoped_id(type_id, &format!("{}_0", name)), line);
                            let vis = kotlin_visibility(prop, ctx.code);
                            emit_function(ctx, &id, type_id, &name, 0, vis, line, prop);
                            *emitted = Some(id.clone());
                            id
                        }
                    };
                    kotlin_calls(&m, &id, type_id, ctx);
                }
                continue;
            }
            "property_declaration" => last_property = Some((m, None)),
            _ => last_property = None,
        }
        match m.kind() {
            "class_declaration" | "object_declaration" | "companion_object" => {
                kotlin_type(&m, qualified, ctx)
            }
            "function_declaration" | "property_declaration" => {
                kotlin_member(&m, type_id, type_name, ctx)
            }
            "secondary_constructor" => {
                let arity = find_child_by_kind(&m, "function_value_parameters")
                    .map(|p| {
                        let mut cc = p.walk();
                        let n = p
                            .named_children(&mut cc)
                            .filter(|x| x.kind() == "parameter")
                            .count();
                        n
                    })
                    .unwrap_or(0);
                let line = node_line(&m);
                let id = ctx.unique_id(
                    scoped_id(type_id, &format!("{}_{}", type_name, arity)),
                    line,
                );
                emit_function(
                    ctx,
                    &id,
                    type_id,
                    type_name,
                    arity,
                    kotlin_visibility(&m, ctx.code),
                    line,
                    &m,
                );
                kotlin_calls(&m, &id, type_id, ctx);
            }
            "anonymous_initializer" => {
                // init { … }: part of construction
                let id = scoped_id(type_id, &format!("init_l{}", node_line(&m)));
                emit_function(ctx, &id, type_id, "init", 0, "private", node_line(&m), &m);
                kotlin_calls(&m, &id, type_id, ctx);
            }
            _ => {}
        }
    }
}

/// A function, or a property with a getter, setter or delegate.
fn kotlin_member(node: &Node<'_>, module_id: &str, _type_name: &str, ctx: &mut Ctx<'_>) {
    let line = node_line(node);
    if node.kind() == "property_declaration" {
        let has_code = find_child_by_kind(node, "getter").is_some()
            || find_child_by_kind(node, "setter").is_some()
            || find_child_by_kind(node, "property_delegate").is_some();
        if !has_code {
            return;
        }
        let Some(var) = find_child_by_kind(node, "variable_declaration") else {
            return;
        };
        let Some(pname) = find_child_by_kind(&var, "simple_identifier") else {
            return;
        };
        let pname = node_text(&pname, ctx.code).to_string();
        let id = ctx.unique_id(scoped_id(module_id, &format!("{}_0", pname)), line);
        emit_function(
            ctx,
            &id,
            module_id,
            &pname,
            0,
            kotlin_visibility(node, ctx.code),
            line,
            node,
        );
        kotlin_calls(node, &id, module_id, ctx);
        return;
    }
    let name = {
        let mut c = node.walk();
        let found = node
            .named_children(&mut c)
            .find(|n| n.kind() == "simple_identifier")
            .map(|n| node_text(&n, ctx.code).to_string());
        found.unwrap_or_else(|| "unknown".to_string())
    };
    let params = find_child_by_kind(node, "function_value_parameters");
    let arity = params
        .map(|p| {
            let mut c = p.walk();
            let n = p
                .named_children(&mut c)
                .filter(|x| x.kind() == "parameter")
                .count();
            n
        })
        .unwrap_or(0);
    let id = ctx.unique_id(scoped_id(module_id, &format!("{}_{}", name, arity)), line);
    emit_function(
        ctx,
        &id,
        module_id,
        &name,
        arity,
        kotlin_visibility(node, ctx.code),
        line,
        node,
    );
    if let Some(p) = params {
        let mut c = p.walk();
        let ps: Vec<Node<'_>> = p
            .named_children(&mut c)
            .filter(|x| x.kind() == "parameter")
            .collect();
        for (i, param) in ps.iter().enumerate() {
            let pn = find_child_by_kind(param, "simple_identifier")
                .map(|n| node_text(&n, ctx.code).to_string())
                .unwrap_or_default();
            let mut cc = param.walk();
            let ty = param
                .named_children(&mut cc)
                .find(|n| n.kind() != "simple_identifier")
                .map(|n| clean_param(node_text(&n, ctx.code)));
            ctx.facts.push(Fact::new(
                "function_param",
                vec![
                    FactValue::String(id.clone()),
                    FactValue::Integer(i as i64),
                    FactValue::String(pn),
                    match ty {
                        Some(t) => FactValue::String(t),
                        None => FactValue::Atom("unknown".to_string()),
                    },
                ],
            ));
        }
    }
    jvm_annotations(node, &id, ctx);
    if let Some(r) = node.child_by_field_name("receiver") {
        ctx.facts.push(Fact::new(
            "annotation",
            vec![
                FactValue::String(id.clone()),
                FactValue::String("receiver".to_string()),
                FactValue::String(clean_param(node_text(&r, ctx.code))),
            ],
        ));
    }
    if let Some(body) = find_child_by_kind(node, "function_body") {
        kotlin_calls(&body, &id, module_id, ctx);
    }
}

fn kotlin_calls(node: &Node<'_>, func_id: &str, module_id: &str, ctx: &mut Ctx<'_>) {
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    for child in children {
        match child.kind() {
            "call_expression" => {
                if let Some(callee) = child
                    .named_child(0)
                    .and_then(|f| kotlin_callee(&f, ctx.code))
                {
                    emit_call(ctx, func_id, &callee, node_line(&child));
                }
            }
            "function_declaration" => {
                // a local function
                kotlin_member(&child, module_id, "", ctx);
                continue;
            }
            "class_declaration" | "object_declaration" => {
                kotlin_type(&child, module_id, ctx);
                continue;
            }
            _ => {}
        }
        kotlin_calls(&child, func_id, module_id, ctx);
    }
}

/// `f(…)`, `x.f(…)`, `Type.f(…)`: a type receiver is kept.
fn kotlin_callee(function: &Node<'_>, code: &[u8]) -> Option<String> {
    match function.kind() {
        "simple_identifier" => Some(node_text(function, code).to_string()),
        "navigation_expression" => {
            let suffix = {
                let mut c = function.walk();
                let found = function
                    .named_children(&mut c)
                    .filter(|n| n.kind() == "navigation_suffix")
                    .last();
                found
            }?;
            let name = find_child_by_kind(&suffix, "simple_identifier")
                .map(|n| node_text(&n, code).to_string())?;
            match function.named_child(0) {
                Some(r)
                    if r.kind() == "simple_identifier"
                        && node_text(&r, code)
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_uppercase()) =>
                {
                    Some(format!("{}.{}", node_text(&r, code), name))
                }
                _ => Some(name),
            }
        }
        _ => None,
    }
}

// ========================================================================
// .NET project files
// ========================================================================
//
// A project (`.csproj`, `.fsproj`, `.vbproj`) is a module; its
// PackageReference, ProjectReference, Reference (assemblies) and
// TargetFramework(s) are dependencies,
// as are the versions `Directory.Packages.props` pins. A solution (`.sln`)
// depends on its projects. Answers "what depends on this package" without
// reading any code.

fn analyze_msbuild(code: &str, ctx: &mut Ctx<'_>) {
    let path = ctx.filepath;
    let stem = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
    let module_id = normalize_id(path);
    emit_module(ctx, &module_id, &stem, path, 1);

    if path.ends_with(".sln") {
        // Project("{type}") = "Name", "rel\\path.csproj", "{guid}"
        for (i, line) in code.lines().enumerate() {
            let t = line.trim_start();
            if !t.starts_with("Project(") {
                continue;
            }
            let quoted: Vec<&str> = t.split('"').skip(1).step_by(2).collect();
            if let Some(proj) = quoted.get(2) {
                if proj.ends_with("proj") {
                    emit_dependency(ctx, &module_id, &proj.replace('\\', "/"), "project", i + 1);
                }
            }
        }
        return;
    }

    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(code);
    let mut open: Option<(String, Option<String>, usize)> = None; // element, Include, line
    let mut deps: Vec<(String, &'static str, usize)> = Vec::new();
    let line_at = |pos: usize| code[..pos.min(code.len())].matches('\n').count() + 1;

    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                let mut include = None;
                let mut version = None;
                for a in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(a.key.local_name().as_ref()).to_string();
                    let val = a
                        .unescape_value()
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    match key.as_str() {
                        "Include" | "Update" => include = Some(val),
                        "Version" => version = Some(val),
                        _ => {}
                    }
                }
                let line = line_at(pos);
                match (tag.as_str(), include) {
                    ("PackageReference" | "PackageVersion", Some(name)) => {
                        // `$(NunitPackageVersion)` is set elsewhere; an
                        // unexpanded property is no version.
                        let target = match version {
                            Some(v) if !v.is_empty() && !v.contains("$(") => {
                                format!("{}@{}", name, v)
                            }
                            _ => name,
                        };
                        deps.push((target, "package", line));
                    }
                    ("ProjectReference", Some(p)) => {
                        deps.push((p.replace('\\', "/"), "project", line))
                    }
                    // .NET Framework assembly references (`System.Web`) and
                    // shared frameworks (`Microsoft.AspNetCore.App`).
                    ("Reference" | "FrameworkReference", Some(name)) => {
                        let name = name.split(',').next().unwrap_or(&name).trim().to_string();
                        deps.push((name, "assembly", line))
                    }
                    ("TargetFramework" | "TargetFrameworks", _) => {
                        open = Some((tag.clone(), None, line))
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if let Some((_, _, line)) = open.take() {
                    let text = t.unescape().map(|v| v.to_string()).unwrap_or_default();
                    for fw in text
                        .split(';')
                        .map(str::trim)
                        .filter(|f| !f.is_empty() && !f.contains("$("))
                    {
                        deps.push((fw.to_string(), "target_framework", line));
                    }
                }
            }
            Ok(Event::End(_)) => open = None,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    // A package is often referenced once per target framework's ItemGroup.
    let mut seen = std::collections::HashSet::new();
    for (target, kind, line) in deps {
        if seen.insert((target.clone(), kind)) {
            emit_dependency(ctx, &module_id, &target, kind, line);
        }
    }
}

// ========================================================================
// Prolog Analysis
// ========================================================================
//
// Prolog's structure maps onto the same fact vocabulary as the imperative
// languages, with a couple of language-specific twists:
//   - a *predicate* (name/arity) is the unit, and it is defined by one OR MANY
//     clauses — so we emit one `function` fact per predicate (first clause) and
//     an extra `function_line` per additional clause;
//   - `function_param` comes from the representative clause's head arguments;
//   - `calls_external` comes from the goals in a clause body (we recurse only
//     through the control operators `, ; -> *-> |`, and treat each goal's
//     functor — including infix goals like `=`/`is` — as the callee);
//   - `:- module(Name, Exports)` drives visibility (exported ⇒ public), and
//     `:- use_module(...)` becomes a `depends_on` dependency.

/// Body/control operators we recurse THROUGH to reach individual goals.
const PROLOG_CONTROL_OPS: &[&str] = &[",", ";", "|", "->", "*->"];

fn analyze_prolog(root: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let (module_id, module_name) = module_id_from_path(ctx.filepath, ".pl", ":");
    emit_module(ctx, &module_id, &module_name, ctx.filepath, 1);

    // Pass 1: directives — collect exported predicate ids + module deps.
    let mut exported: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut has_module = false;
    let mut cursor = root.walk();
    let clauses: Vec<Node<'_>> = root
        .children(&mut cursor)
        .filter(|c| c.kind() == "clause")
        .collect();

    for clause in &clauses {
        let Some(term) = clause.child_by_field_name("term") else {
            continue;
        };
        if term.kind() == "unary_operation" {
            let op = field_op_text(&term, ctx.code);
            if op == ":-" || op == "?-" {
                if let Some(operand) = term.child_by_field_name("operand") {
                    prolog_directive(&operand, &module_id, &mut exported, &mut has_module, ctx);
                }
            }
        }
    }

    // Pass 2: clauses → predicates (deduped by name/arity), params, calls.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for clause in &clauses {
        let Some(term) = clause.child_by_field_name("term") else {
            continue;
        };

        // Skip directives — they are not predicate definitions.
        if term.kind() == "unary_operation" {
            let op = field_op_text(&term, ctx.code);
            if op == ":-" || op == "?-" {
                continue;
            }
        }

        let (head, body) = prolog_head_body(&term);
        let Some(head) = head else { continue };
        let Some((name, arity, head_node)) = prolog_predicate(&head, ctx.code) else {
            continue;
        };

        let local_id = normalize_id(&format!("{}_{}", name, arity));
        let func_id = scoped_id(&module_id, &local_id);
        let line = node_line(clause);

        if seen.insert(func_id.clone()) {
            let visibility = if !has_module || exported.contains(&local_id) {
                "public"
            } else {
                "private"
            };
            emit_prolog_predicate(
                ctx, &func_id, &module_id, &name, arity, visibility, line, &head_node,
            );
        } else {
            // Another clause of an already-seen predicate — record its line.
            ctx.facts.push(Fact::new(
                "function_line",
                vec![
                    FactValue::String(func_id.clone()),
                    FactValue::Integer(line as i64),
                ],
            ));
        }

        if let Some(body) = body {
            prolog_collect_goals(&body, ctx.code, &func_id, ctx);
        }
    }

    Ok(())
}

/// The text of a node's `operator` field (e.g. `":-"`, `","`, `"is"`), or "".
fn field_op_text(node: &Node<'_>, code: &[u8]) -> String {
    node.child_by_field_name("operator")
        .map(|n| node_text(&n, code).to_string())
        .unwrap_or_default()
}

/// The name of an `atom` node, with surrounding single-quotes stripped for
/// quoted atoms (`'$foo'` → `$foo`).
fn prolog_atom_name(node: &Node<'_>, code: &[u8]) -> String {
    let t = node_text(node, code);
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

/// Split a clause term into its (head, optional body).
fn prolog_head_body<'a>(term: &Node<'a>) -> (Option<Node<'a>>, Option<Node<'a>>) {
    if term.kind() == "binary_operation" {
        let op = term.child_by_field_name("operator");
        let op_kind = op.map(|n| n.kind());
        match op_kind {
            // Rule / DCG: Head :- Body  /  Head --> Body
            Some(":-") | Some("-->") => {
                return (
                    term.child_by_field_name("left"),
                    term.child_by_field_name("right"),
                );
            }
            // ProbLog annotated fact: Prob :: Head
            Some("::") => {
                return (term.child_by_field_name("right"), None);
            }
            _ => {}
        }
    }
    (Some(*term), None)
}

/// Resolve a head term to `(name, arity, head_node)`. Unwraps a leading ProbLog
/// `::` annotation (`0.5::heads :- ...` → head predicate is `heads`).
fn prolog_predicate<'a>(head: &Node<'a>, code: &[u8]) -> Option<(String, usize, Node<'a>)> {
    let mut h = *head;
    if h.kind() == "binary_operation"
        && h.child_by_field_name("operator").map(|n| n.kind()) == Some("::")
    {
        if let Some(r) = h.child_by_field_name("right") {
            h = r;
        }
    }

    match h.kind() {
        "compound_term" => {
            let functor = h.child_by_field_name("functor")?;
            let name = prolog_atom_name(&functor, code);
            let mut cursor = h.walk();
            let arity = h.children_by_field_name("argument", &mut cursor).count();
            Some((name, arity, h))
        }
        "atom" => Some((prolog_atom_name(&h, code), 0, h)),
        "operator_atom" => Some((node_text(&h, code).to_string(), 0, h)),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_prolog_predicate(
    ctx: &mut Ctx<'_>,
    func_id: &str,
    module_id: &str,
    name: &str,
    arity: usize,
    visibility: &str,
    line: usize,
    head_node: &Node<'_>,
) {
    ctx.facts.push(Fact::new(
        "function",
        vec![
            FactValue::String(func_id.to_string()),
            FactValue::String(module_id.to_string()),
            FactValue::String(name.to_string()),
            FactValue::Integer(arity as i64),
            FactValue::Atom(visibility.to_string()),
            FactValue::Integer(line as i64),
            FactValue::String(ctx.commit_sha.to_string()),
        ],
    ));
    ctx.facts.push(Fact::new(
        "function_line",
        vec![
            FactValue::String(func_id.to_string()),
            FactValue::Integer(line as i64),
        ],
    ));

    // The file directly: a method's module is its class, whose module/5 path
    // is `class:Name`, so the module join cannot locate it.
    ctx.facts.push(Fact::new(
        "function_file",
        vec![
            FactValue::String(func_id.to_string()),
            FactValue::String(ctx.filepath.to_string()),
        ],
    ));
    ctx.facts.push(Fact::new(
        "function_visibility",
        vec![
            FactValue::String(func_id.to_string()),
            FactValue::Atom(visibility.to_string()),
        ],
    ));

    // Params from head arguments — positional, untyped (Prolog is untyped).
    let mut cursor = head_node.walk();
    let mut pos: i64 = 0;
    for arg in head_node.children_by_field_name("argument", &mut cursor) {
        let pname = clean_param(node_text(&arg, ctx.code));
        if pname.is_empty() {
            continue;
        }
        ctx.facts.push(Fact::new(
            "function_param",
            vec![
                FactValue::String(func_id.to_string()),
                FactValue::Integer(pos),
                FactValue::String(pname),
                FactValue::Atom("unknown".to_string()),
            ],
        ));
        pos += 1;
    }

    ctx.summary.functions += 1;
}

/// Walk a clause body, recursing only through control operators, and emit a
/// call for each goal's functor (including infix goals like `=`/`is`).
fn prolog_collect_goals(node: &Node<'_>, code: &[u8], caller_id: &str, ctx: &mut Ctx<'_>) {
    match node.kind() {
        "binary_operation" => {
            let op = field_op_text(node, code);
            if PROLOG_CONTROL_OPS.contains(&op.as_str()) {
                if let Some(l) = node.child_by_field_name("left") {
                    prolog_collect_goals(&l, code, caller_id, ctx);
                }
                if let Some(r) = node.child_by_field_name("right") {
                    prolog_collect_goals(&r, code, caller_id, ctx);
                }
            } else if op == ":" {
                // Module-qualified goal `Mod:Goal` — the real callee is the goal
                // on the right; the module qualifier is not itself a call.
                if let Some(r) = node.child_by_field_name("right") {
                    prolog_collect_goals(&r, code, caller_id, ctx);
                }
            } else if !op.is_empty() {
                emit_call(ctx, caller_id, &op, node_line(node));
            }
        }
        "unary_operation" => {
            let op = field_op_text(node, code);
            if op == "\\+" {
                if let Some(o) = node.child_by_field_name("operand") {
                    prolog_collect_goals(&o, code, caller_id, ctx);
                }
            } else if !op.is_empty() {
                emit_call(ctx, caller_id, &op, node_line(node));
            }
        }
        "parenthesized" | "curly_block" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                prolog_collect_goals(&child, code, caller_id, ctx);
            }
        }
        "compound_term" => {
            if let Some(functor) = node.child_by_field_name("functor") {
                let name = prolog_atom_name(&functor, code);
                if !name.is_empty() {
                    emit_call(ctx, caller_id, &name, node_line(node));
                }
            }
        }
        "atom" => {
            let name = prolog_atom_name(node, code);
            if !name.is_empty() {
                emit_call(ctx, caller_id, &name, node_line(node));
            }
        }
        "cut" => emit_call(ctx, caller_id, "!", node_line(node)),
        _ => {}
    }
}

/// Handle a directive's operand (the goal after `:-`).
fn prolog_directive(
    operand: &Node<'_>,
    module_id: &str,
    exported: &mut std::collections::HashSet<String>,
    has_module: &mut bool,
    ctx: &mut Ctx<'_>,
) {
    if operand.kind() != "compound_term" {
        return;
    }
    let Some(functor) = operand.child_by_field_name("functor") else {
        return;
    };
    let dname = prolog_atom_name(&functor, ctx.code);

    let mut cursor = operand.walk();
    let args: Vec<Node<'_>> = operand
        .children_by_field_name("argument", &mut cursor)
        .collect();

    match dname.as_str() {
        "module" => {
            *has_module = true;
            if let Some(list) = args.get(1) {
                if list.kind() == "list" {
                    let mut lc = list.walk();
                    for el in list.children_by_field_name("element", &mut lc) {
                        // Each export is `name/arity`.
                        if el.kind() == "binary_operation" && field_op_text(&el, ctx.code) == "/" {
                            let (Some(l), Some(r)) = (
                                el.child_by_field_name("left"),
                                el.child_by_field_name("right"),
                            ) else {
                                continue;
                            };
                            let lname = prolog_atom_name(&l, ctx.code);
                            let ar = node_text(&r, ctx.code);
                            exported.insert(normalize_id(&format!("{}_{}", lname, ar)));
                        }
                    }
                }
            }
        }
        "use_module" | "ensure_loaded" | "consult" => {
            if let Some(arg) = args.first() {
                let dep = prolog_dep_name(arg, ctx.code);
                if !dep.is_empty() {
                    emit_dependency(ctx, module_id, &dep, &dname, node_line(operand));
                }
            }
        }
        _ => {}
    }
}

/// A dependency target: unwrap `library(lists)` → `lists`, otherwise the
/// stripped atom/string text.
fn prolog_dep_name(arg: &Node<'_>, code: &[u8]) -> String {
    if arg.kind() == "compound_term" {
        if let Some(functor) = arg.child_by_field_name("functor") {
            if prolog_atom_name(&functor, code) == "library" {
                let mut cursor = arg.walk();
                let inner = arg.children_by_field_name("argument", &mut cursor).next();
                if let Some(inner) = inner {
                    return prolog_atom_name(&inner, code);
                }
            }
        }
    }
    let t = node_text(arg, code);
    t.trim_matches(|c| c == '\'' || c == '"').to_string()
}

// ========================================================================
// Shared call analysis (Python uses "call", Rust/JS/Go use "call_expression")
// ========================================================================

fn analyze_calls(
    node: &Node<'_>,
    code: &[u8],
    func_id: &str,
    ctx: &mut Ctx<'_>,
    call_kind: &str,
) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == call_kind {
            if let Some(func_node) = child.child_by_field_name("function") {
                let called_func = callee_name(&func_node, code);
                if !called_func.is_empty() {
                    emit_call(ctx, func_id, &called_func, node_line(&child));
                }
            }
        }
        analyze_calls(&child, code, func_id, ctx, call_kind)?;
    }

    Ok(())
}

/// Extract a clean callee name from a call's `function` node.
///
/// The naive `node_text(function_node)` is wrong for chained / method calls:
/// for `a.b().c()` the outer call's `function` field is the *entire*
/// `a.b().c` expression — including the inner call's arguments and any
/// newlines — which produced multi-line garbage like
/// `"Self::git_output(&[...]\n.map"` in the fact base.
///
/// The inner call is captured independently by the recursion in
/// [`analyze_calls`], so at each node we only want *this* call's callee:
/// - member/field/attribute/selector access → the method/field name, prefixed
///   with the receiver only when it's a simple single-token identifier (so
///   `order.advance` survives but `<nested call>.advance` collapses to
///   `advance`);
/// - everything else (plain identifier, Rust `A::b` scoped path) → its text,
///   passed through [`sanitize_callee`] as a final guard against any
///   language/node-kind we didn't special-case.
fn callee_name(func_node: &Node<'_>, code: &[u8]) -> String {
    match func_node.kind() {
        // Rust `x.method`, JS `obj.prop`, Python `obj.attr`, Go `pkg.Field`.
        "field_expression" | "member_expression" | "attribute" | "selector_expression" => {
            let method = func_node
                .child_by_field_name("field")
                .or_else(|| func_node.child_by_field_name("property"))
                .or_else(|| func_node.child_by_field_name("attribute"))
                .or_else(|| func_node.child_by_field_name("name"))
                .map(|n| node_text(&n, code))
                .unwrap_or("");

            let receiver = func_node
                .child_by_field_name("value")
                .or_else(|| func_node.child_by_field_name("object"))
                .or_else(|| func_node.child_by_field_name("operand"));

            match receiver {
                // Keep a simple qualifier for readability (and nav suffix-match
                // still resolves the bare method); never a nested call.
                Some(r) if is_simple_receiver(&r) => {
                    format!("{}.{}", node_text(&r, code), method)
                }
                _ => method.to_string(),
            }
        }
        _ => sanitize_callee(node_text(func_node, code)),
    }
}

/// A receiver is "simple" when it's a single, single-line token — safe to keep
/// as a qualifier.  Anything containing a call, parentheses, or a newline is
/// not, so we'd drop it and keep only the method name.
fn is_simple_receiver(node: &Node<'_>) -> bool {
    matches!(
        node.kind(),
        "identifier" | "self" | "scoped_identifier" | "field_identifier" | "type_identifier"
    )
}

/// Final guard: collapse any callee text that still looks like an expression
/// (contains a newline or `(`) down to a clean trailing symbol.  Defends every
/// node-kind we didn't special-case across languages.
fn sanitize_callee(text: &str) -> String {
    // `await f<T>()` and `new X<T>()` name f and X: drop the keyword and any
    // type arguments before anything else.
    let text = text.trim();
    let text = text
        .strip_prefix("await ")
        .or_else(|| text.strip_prefix("new "))
        .unwrap_or(text)
        .trim();
    let text = match text.find('<') {
        Some(i) => &text[..i],
        None => text,
    };

    if !text.contains('\n') && !text.contains('(') {
        return text.trim().to_string();
    }
    // Take the part before the first `(`, then the final `.`/`::`-segment.
    let head = text.split('(').next().unwrap_or(text);
    let head = head.replace(['\n', '\r', '\t', ' '], "");
    head.rsplit(['.']).next().unwrap_or(&head).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_callee_produces_clean_single_token() {
        // The exact shape of the observed bug — the contract is "no newline,
        // no paren, no whitespace", not a specific token (the field_expression
        // path handles real chains; this is only the catch-all fallback).
        let garbage = "Self::git_output(&[\"tag\", \"-l\"], &self.path)\n            .map";
        let out = sanitize_callee(garbage);
        assert!(
            !out.contains('\n') && !out.contains('(') && !out.contains(' '),
            "not clean: {out:?}"
        );
        assert!(!out.is_empty());

        // Clean names pass through untouched.
        assert_eq!(sanitize_callee("foo"), "foo");
        assert_eq!(sanitize_callee("Module::bar"), "Module::bar");
    }

    #[test]
    fn rust_method_chain_emits_clean_callees_no_newlines() {
        let mut analyzer = Analyzer::new().unwrap();
        let code = r#"
pub fn run(&self) -> bool {
    let x = self.git_output(&["tag", "-l", "manifold-base"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    helper(x)
}
"#;
        let result = analyzer
            .lens_code(code, Language::Rust, "lib.rs", "abc")
            .unwrap();

        let calls: Vec<&String> = result
            .facts
            .iter()
            .filter(|f| f.starts_with("calls_external("))
            .collect();

        // No callee fact may contain a newline or an opening paren (the bug).
        for f in &calls {
            assert!(!f.contains('\n'), "callee fact has newline: {f}");
            // The fact itself has parens (it's a predicate); check the callee arg.
            // calls_external('caller','unknown','CALLEE',0,line).
            let callee = f.split('\'').nth(5).unwrap_or("");
            assert!(
                !callee.contains('(') && !callee.contains('\n') && !callee.contains(' '),
                "callee arg is not clean: {callee:?} in {f}"
            );
        }

        // The chain's method names are captured individually.
        let joined = calls.iter().fold(String::new(), |a, f| a + f);
        assert!(
            joined.contains("git_output"),
            "missing git_output: {joined}"
        );
        assert!(joined.contains("map"), "missing map: {joined}");
        assert!(joined.contains("unwrap_or"), "missing unwrap_or: {joined}");
        assert!(joined.contains("helper"), "missing helper: {joined}");
    }

    #[test]
    fn rust_function_params_capture_name_and_type() {
        let mut analyzer = Analyzer::new().unwrap();
        let code = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n";
        let result = analyzer
            .lens_code(code, Language::Rust, "lib.rs", "abc")
            .unwrap();

        let params: Vec<&String> = result
            .facts
            .iter()
            .filter(|f| f.starts_with("function_param("))
            .collect();

        assert_eq!(params.len(), 2, "expected 2 params, got: {params:?}");
        let joined = params.iter().fold(String::new(), |a, f| a + f);
        // Position, name and type all present.
        assert!(joined.contains("'a'"), "missing param a: {joined}");
        assert!(joined.contains("'b'"), "missing param b: {joined}");
        assert!(joined.contains("'i32'"), "missing type i32: {joined}");
        // Positions are 0 and 1.
        assert!(params.iter().any(|f| f.contains(", 0,")));
        assert!(params.iter().any(|f| f.contains(", 1,")));
    }

    #[test]
    fn rust_self_receiver_is_captured_then_typed_params() {
        let mut analyzer = Analyzer::new().unwrap();
        let code = "impl S { pub fn m(&self, n: u8) {} }\n";
        let result = analyzer
            .lens_code(code, Language::Rust, "lib.rs", "abc")
            .unwrap();
        let joined = result
            .facts
            .iter()
            .filter(|f| f.starts_with("function_param("))
            .fold(String::new(), |a, f| a + f);
        assert!(joined.contains("'self'"), "self receiver missing: {joined}");
        assert!(
            joined.contains("'n'") && joined.contains("'u8'"),
            "typed param missing: {joined}"
        );
    }

    #[test]
    fn cross_language_function_params() {
        let mut analyzer = Analyzer::new().unwrap();

        let params = |a: &mut Analyzer, code: &str, lang: Language, file: &str| -> String {
            a.lens_code(code, lang, file, "x")
                .unwrap()
                .facts
                .into_iter()
                .filter(|f| f.starts_with("function_param("))
                .collect::<Vec<_>>()
                .join("\n")
        };

        // JavaScript — names, untyped → unknown.
        let js = params(
            &mut analyzer,
            "function f(a, b) { return a; }",
            Language::JavaScript,
            "x.js",
        );
        assert!(js.contains("'a'") && js.contains("'b'"), "js: {js}");
        assert!(js.contains("unknown"), "js untyped should be unknown: {js}");

        // TypeScript — type captured WITHOUT the leading colon.
        let ts = params(
            &mut analyzer,
            "function g(a: number, b: string): number { return a; }",
            Language::TypeScript,
            "x.ts",
        );
        assert!(
            ts.contains("'number'"),
            "ts type should be bare 'number': {ts}"
        );
        assert!(
            ts.contains("'string'"),
            "ts type should be bare 'string': {ts}"
        );
        assert!(
            !ts.contains(": number"),
            "ts type must not keep the colon: {ts}"
        );

        // Go — typed params.
        let go = params(
            &mut analyzer,
            "func Add(a int, b int) int { return a }",
            Language::Go,
            "x.go",
        );
        assert!(go.contains("'a'") && go.contains("'int'"), "go: {go}");

        // Ruby — names, dynamically typed → unknown.
        let rb = params(
            &mut analyzer,
            "def greet(name, count)\n  name\nend\n",
            Language::Ruby,
            "x.rb",
        );
        assert!(rb.contains("'name'") && rb.contains("'count'"), "rb: {rb}");
        assert!(rb.contains("unknown"), "rb untyped should be unknown: {rb}");
    }

    #[test]
    fn python_function_params_capture_names_untyped_are_unknown() {
        let mut analyzer = Analyzer::new().unwrap();
        let code = "def greet(name, count):\n    return name\n";
        let result = analyzer
            .lens_code(code, Language::Python, "m.py", "abc")
            .unwrap();
        let params: Vec<&String> = result
            .facts
            .iter()
            .filter(|f| f.starts_with("function_param("))
            .collect();
        let joined = params.iter().fold(String::new(), |a, f| a + f);
        assert!(joined.contains("'name'"), "missing name: {joined}");
        assert!(joined.contains("'count'"), "missing count: {joined}");
        // No type annotations → type atom is `unknown` (unquoted atom).
        assert!(
            joined.contains("unknown"),
            "untyped params should be unknown: {joined}"
        );
    }

    #[test]
    fn prolog_predicates_params_calls_and_visibility() {
        let mut analyzer = Analyzer::new().unwrap();
        let code = r#":- module(dungeon, [room_accessible/2]).
:- use_module(library(lists)).

relation(entrance, connects_to, hall).
relation(hall, connects_to, vault).

room_accessible(P, R) :-
    attribute(R, requires_key, K),
    relation(P, has_item, K).

room_accessible(P, R) :- relation(P, in_room, R).
"#;
        let result = analyzer
            .lens_code(code, Language::Prolog, "dungeon.pl", "abc123")
            .unwrap();
        let facts = result.facts.join("\n");

        // Predicate emitted once per name/arity (room_accessible/2 has 2 clauses
        // but a single `function` fact).
        let funcs: Vec<&String> = result
            .facts
            .iter()
            .filter(|f| f.starts_with("function('dungeon__room_accessible_2'"))
            .collect();
        assert_eq!(funcs.len(), 1, "predicate should be deduped: {funcs:?}");

        // Exported predicate is public; non-exported is private.
        assert!(
            facts.contains(
                "function('dungeon__room_accessible_2', 'dungeon', 'room_accessible', 2, public"
            ),
            "exported pred should be public: {facts}"
        );
        assert!(
            facts.contains("function('dungeon__relation_3', 'dungeon', 'relation', 3, private"),
            "non-exported pred should be private: {facts}"
        );

        // Head variables become positional params.
        assert!(
            facts.contains("function_param('dungeon__room_accessible_2', 0, 'P', unknown)"),
            "param P missing: {facts}"
        );
        assert!(
            facts.contains("function_param('dungeon__room_accessible_2', 1, 'R', unknown)"),
            "param R missing: {facts}"
        );

        // Body goals become calls (functor of each goal).
        assert!(
            facts.contains("calls_external('dungeon__room_accessible_2', 'unknown', 'attribute'"),
            "attribute call missing: {facts}"
        );
        assert!(
            facts.contains("calls_external('dungeon__room_accessible_2', 'unknown', 'relation'"),
            "relation call missing: {facts}"
        );

        // The second clause adds an extra function_line for the same predicate.
        let lines = result
            .facts
            .iter()
            .filter(|f| f.starts_with("function_line('dungeon__room_accessible_2'"))
            .count();
        assert_eq!(lines, 2, "expected a line per clause: {facts}");

        // use_module → dependency.
        assert!(
            facts.contains("depends_on('dungeon', 'lists', use_module"),
            "use_module dep missing: {facts}"
        );
    }

    #[test]
    fn prolog_problog_annotation_resolves_head_predicate() {
        let mut analyzer = Analyzer::new().unwrap();
        let code = "0.5::heads :- toss.\n0.3::rains.\n";
        let result = analyzer
            .lens_code(code, Language::Prolog, "coin.pl", "z")
            .unwrap();
        let facts = result.facts.join("\n");
        // No module directive → predicates default public.
        assert!(
            facts.contains("function('coin__heads_0', 'coin', 'heads', 0, public"),
            "problog head 'heads' missing: {facts}"
        );
        assert!(
            facts.contains("function('coin__rains_0', 'coin', 'rains', 0, public"),
            "problog fact 'rains' missing: {facts}"
        );
        // The annotated rule's body goal is a call.
        assert!(
            facts.contains("calls_external('coin__heads_0', 'unknown', 'toss'"),
            "toss call missing: {facts}"
        );
    }

    /// Helper: lens a Prolog snippet and join the facts for substring asserts.
    fn prolog_facts(code: &str) -> String {
        Analyzer::new()
            .unwrap()
            .lens_code(code, Language::Prolog, "m.pl", "sha")
            .unwrap()
            .facts
            .join("\n")
    }

    #[test]
    fn prolog_fact_predicate_and_params_from_first_clause() {
        // A fact predicate gets arity + positional params from the head args.
        let f = prolog_facts("relation(entrance, connects_to, hall).\n");
        assert!(
            f.contains("function('m__relation_3', 'm', 'relation', 3, public"),
            "relation/3 fact pred missing: {f}"
        );
        assert!(
            f.contains("function_param('m__relation_3', 0,"),
            "positional param 0 missing: {f}"
        );
        assert!(
            f.contains("function_param('m__relation_3', 2,"),
            "positional param 2 missing: {f}"
        );
    }

    #[test]
    fn prolog_negation_records_inner_goal_call() {
        let f = prolog_facts("safe(X) :- \\+ forbidden(X).\n");
        assert!(
            f.contains("calls_external('m__safe_1', 'unknown', 'forbidden'"),
            "negated inner goal should be a call: {f}"
        );
    }

    #[test]
    fn prolog_if_then_else_records_all_branch_calls() {
        let f = prolog_facts("pick(X) :- ( cond(X) -> yes(X) ; no(X) ).\n");
        for goal in ["cond", "yes", "no"] {
            assert!(
                f.contains(&format!("calls_external('m__pick_1', 'unknown', '{goal}'")),
                "branch goal {goal} missing: {f}"
            );
        }
    }

    #[test]
    fn prolog_module_qualified_goal_calls_inner_predicate() {
        // `lists:member(...)` — the callee is `member`, not the `:` qualifier.
        let f = prolog_facts("has(X, L) :- lists:member(X, L).\n");
        assert!(
            f.contains("calls_external('m__has_2', 'unknown', 'member'"),
            "module-qualified callee should be 'member': {f}"
        );
        assert!(
            !f.contains("'unknown', ':'"),
            "the ':' qualifier must not be recorded as a call: {f}"
        );
    }

    #[test]
    fn prolog_directives_are_not_predicates() {
        // dynamic / discontiguous directives must not produce `function` facts.
        let f = prolog_facts(":- dynamic counter/1.\n:- discontiguous foo/2.\ncounter(0).\n");
        assert!(
            !f.contains("function('dynamic_"),
            "dynamic directive leaked as predicate: {f}"
        );
        assert!(
            !f.contains("function('discontiguous_"),
            "discontiguous directive leaked as predicate: {f}"
        );
        // The actual predicate is still picked up.
        assert!(
            f.contains("function('m__counter_1'"),
            "counter/1 predicate missing: {f}"
        );
    }

    #[test]
    fn prolog_quoted_atom_predicate_name_is_unquoted() {
        let f = prolog_facts("'$secret'(X) :- p(X).\n");
        assert!(
            f.contains("function('m___secret_1', 'm', '$secret', 1"),
            "quoted atom name should be unquoted in name slot: {f}"
        );
    }

    #[test]
    fn prolog_cut_and_infix_goals_are_calls() {
        let f = prolog_facts("g(X, Y) :- p(X), !, Y is X + 1.\n");
        assert!(
            f.contains("calls_external('m__g_2', 'unknown', 'p'"),
            "p call missing: {f}"
        );
        assert!(
            f.contains("calls_external('m__g_2', 'unknown', '!'"),
            "cut should be recorded as a goal: {f}"
        );
        assert!(
            f.contains("calls_external('m__g_2', 'unknown', 'is'"),
            "infix `is` goal should be a call: {f}"
        );
    }

    #[test]
    fn prolog_dcg_head_becomes_predicate() {
        let f = prolog_facts("greeting --> [hello], [world].\n");
        assert!(
            f.contains("function('m__greeting_0', 'm', 'greeting', 0"),
            "DCG head predicate missing: {f}"
        );
    }

    #[test]
    fn prolog_bare_path_dependency() {
        let f = prolog_facts(":- use_module('utils/helpers.pl').\n");
        assert!(
            f.contains("depends_on('m', 'utils/helpers.pl', use_module"),
            "bare-path use_module dep missing: {f}"
        );
    }

    #[test]
    fn test_analyze_python_simple() {
        let mut analyzer = Analyzer::new().unwrap();

        let code = r#"
def hello():
    print('hello')

def world(x, y):
    return x + y
"#;

        let result = analyzer
            .lens_code(code, Language::Python, "test.py", "abc123")
            .unwrap();

        assert!(result.summary.functions >= 2);
        assert!(!result.facts.is_empty());

        let has_hello = result.facts.iter().any(|f| f.contains("'hello'"));
        let has_world = result.facts.iter().any(|f| f.contains("'world'"));

        assert!(has_hello);
        assert!(has_world);
    }

    #[test]
    fn test_analyze_elixir_simple() {
        let mut analyzer = Analyzer::new().unwrap();

        let code = r#"
defmodule MyApp.Calculator do
  import Enum

  def add(a, b) do
    a + b
  end

  defp validate(x) do
    x > 0
  end
end
"#;

        let result = analyzer
            .lens_code(code, Language::Elixir, "calculator.ex", "abc123")
            .unwrap();

        assert!(
            result.summary.functions >= 2,
            "Expected at least 2 functions, got {}",
            result.summary.functions
        );

        let has_add = result.facts.iter().any(|f| f.contains("'add'"));
        let has_validate = result.facts.iter().any(|f| f.contains("'validate'"));
        let has_public = result
            .facts
            .iter()
            .any(|f| f.contains("public") && f.contains("'add'"));
        let has_private = result
            .facts
            .iter()
            .any(|f| f.contains("private") && f.contains("'validate'"));
        let has_import = result
            .facts
            .iter()
            .any(|f| f.contains("depends_on") && f.contains("Enum"));

        assert!(has_add, "Missing add function fact");
        assert!(has_validate, "Missing validate function fact");
        assert!(has_public, "add should be public");
        assert!(has_private, "validate should be private");
        assert!(has_import, "Missing import dependency");
    }

    #[test]
    fn test_analyze_ruby_simple() {
        let mut analyzer = Analyzer::new().unwrap();

        let code = r#"
require "json"

class Calculator
  include Comparable

  def add(a, b)
    a + b
  end

  def self.version
    "1.0"
  end

  private

  def validate(x)
    x > 0
  end
end
"#;

        let result = analyzer
            .lens_code(code, Language::Ruby, "calculator.rb", "abc123")
            .unwrap();

        assert!(
            result.summary.functions >= 2,
            "Expected at least 2 functions, got {}",
            result.summary.functions
        );

        let has_add = result.facts.iter().any(|f| f.contains("'add'"));
        let has_class = result.facts.iter().any(|f| f.contains("Calculator"));
        let has_require = result
            .facts
            .iter()
            .any(|f| f.contains("depends_on") && f.contains("json"));

        assert!(has_add, "Missing add method fact");
        assert!(has_class, "Missing Calculator class fact");
        assert!(has_require, "Missing require dependency");
    }

    #[test]
    fn test_analyze_ruby_visibility() {
        let mut analyzer = Analyzer::new().unwrap();

        let code = r#"
class Account
  def public_method
    nil
  end

  private

  def secret_method
    nil
  end

  def another_secret
    nil
  end

  protected

  def family_method
    nil
  end

  public

  def back_to_public
    nil
  end

  private :targeted_method

  def targeted_method
    nil
  end
end
"#;

        let result = analyzer
            .lens_code(code, Language::Ruby, "account.rb", "abc123")
            .unwrap();

        let vis_for = |name: &str| -> Option<String> {
            result
                .facts
                .iter()
                .find(|f| f.starts_with("function_visibility(") && f.contains(name))
                .and_then(|f| {
                    if f.contains("public") {
                        Some("public".to_string())
                    } else if f.contains("private") {
                        Some("private".to_string())
                    } else if f.contains("protected") {
                        Some("protected".to_string())
                    } else {
                        None
                    }
                })
        };

        assert_eq!(
            vis_for("public_method").as_deref(),
            Some("public"),
            "public_method should be public"
        );
        assert_eq!(
            vis_for("secret_method").as_deref(),
            Some("private"),
            "secret_method should be private"
        );
        assert_eq!(
            vis_for("another_secret").as_deref(),
            Some("private"),
            "another_secret should be private (inherits from bare private)"
        );
        assert_eq!(
            vis_for("family_method").as_deref(),
            Some("protected"),
            "family_method should be protected"
        );
        assert_eq!(
            vis_for("back_to_public").as_deref(),
            Some("public"),
            "back_to_public should be public (visibility reset)"
        );
        assert_eq!(
            vis_for("targeted_method").as_deref(),
            Some("private"),
            "targeted_method should be private (via private :method_name)"
        );
    }

    #[test]
    fn test_analyze_php_simple() {
        let mut analyzer = Analyzer::new().unwrap();

        let code = r#"<?php
namespace App\Billing;

use App\Support\Logger;

require_once 'vendor/autoload.php';

class InvoiceService
{
    public function total(array $items, float $tax_rate)
    {
        $sum = array_sum($items);
        $this->log($sum);
        Logger::write("total computed");
        $formatter = new NumberFormatter();
        return $sum * (1 + $tax_rate);
    }

    private function log($value)
    {
        error_log($value);
    }

    protected function audit()
    {
        return null;
    }

    public function untouched()
    {
        return true;
    }
}

function apply_discount($amount, $percent)
{
    return $amount - ($amount * $percent / 100);
}
"#;

        let result = analyzer
            .lens_code(
                code,
                Language::Php,
                "app/billing/invoice_service.php",
                "abc123",
            )
            .unwrap();

        assert!(
            result.summary.functions >= 5,
            "Expected at least 5 functions, got {}",
            result.summary.functions
        );

        let has = |needle: &str| result.facts.iter().any(|f| f.contains(needle));

        // Class emitted as a module; methods scoped to it.
        assert!(
            has("InvoiceService") || has("invoiceservice"),
            "Missing class module fact"
        );
        assert!(has("'total'"), "Missing total method fact");
        assert!(has("'apply_discount'"), "Missing standalone function fact");

        // Visibility: explicit modifiers + default public.
        let vis_for = |name: &str| -> Option<&str> {
            result
                .facts
                .iter()
                .find(|f| f.starts_with("function_visibility(") && f.contains(name))
                .map(|f| {
                    if f.contains("private") {
                        "private"
                    } else if f.contains("protected") {
                        "protected"
                    } else {
                        "public"
                    }
                })
        };
        assert_eq!(vis_for("total"), Some("public"), "total should be public");
        assert_eq!(vis_for("log_1"), Some("private"), "log should be private");
        assert_eq!(
            vis_for("audit"),
            Some("protected"),
            "audit should be protected"
        );
        assert_eq!(
            vis_for("untouched"),
            Some("public"),
            "unmodified method should default to public"
        );

        // Dependencies: use-import, require, extends-free.
        assert!(
            result
                .facts
                .iter()
                .any(|f| f.contains("depends_on") && f.contains("Logger")),
            "Missing use-import dependency"
        );
        assert!(
            result
                .facts
                .iter()
                .any(|f| f.contains("depends_on") && f.contains("vendor/autoload.php")),
            "Missing require_once dependency"
        );

        // Calls: plain function, method-on-$this, scoped (static), constructor.
        assert!(has("array_sum"), "Missing plain function call");
        assert!(
            result
                .facts
                .iter()
                .any(|f| f.contains("calls_external") && f.contains("'log'")),
            "Missing $this->log method call"
        );
        assert!(has("Logger::write"), "Missing scoped static call");
        assert!(has("NumberFormatter"), "Missing constructor call");

        // Params captured with PHP $names.
        assert!(
            result
                .facts
                .iter()
                .any(|f| f.contains("function_param") && f.contains("$tax_rate")),
            "Missing $tax_rate param fact"
        );
    }

    #[test]
    fn test_analyze_php_html_template() {
        // WordPress-style template: HTML with PHP islands must still lens.
        let mut analyzer = Analyzer::new().unwrap();

        let code = r#"<html>
<body>
<?php
function render_header($title)
{
    echo esc_html($title);
    get_template_part('nav');
}
?>
<div><?php render_header(wp_title()); ?></div>
</body>
</html>
"#;

        let result = analyzer
            .lens_code(
                code,
                Language::Php,
                "wp-content/themes/x/header.php",
                "abc123",
            )
            .unwrap();

        assert!(
            result.summary.functions >= 1,
            "Expected the template function, got {}",
            result.summary.functions
        );
        let has = |needle: &str| result.facts.iter().any(|f| f.contains(needle));
        assert!(has("'render_header'"), "Missing render_header fact");
        assert!(
            has("esc_html"),
            "Missing esc_html call inside function body"
        );
    }

    #[test]
    fn test_analyze_php_namespace_and_traits() {
        // File-level `namespace Foo;` (no body) re-scopes the rest of the
        // file; interfaces/traits emit modules; grouped use-imports emit one
        // dependency per name.
        let mut analyzer = Analyzer::new().unwrap();

        let code = r#"<?php
namespace App\Payments;

use App\Support\{Clock, Money};

interface Refundable
{
    public function refund($amount);
}

trait LogsActivity
{
    public function log_activity($event)
    {
        activity_recorder($event);
    }
}

function currency_round($amount)
{
    return round($amount, 2);
}
"#;

        let result = analyzer
            .lens_code(code, Language::Php, "app/payments/support.php", "abc123")
            .unwrap();

        let has = |needle: &str| result.facts.iter().any(|f| f.contains(needle));

        // Namespace emitted as a module.
        assert!(
            has("namespace:App\\Payments") || has("app_payments"),
            "Missing namespace module fact"
        );

        // Interface + trait emitted as modules, their members as functions.
        assert!(has("interface:Refundable"), "Missing interface module fact");
        assert!(has("trait:LogsActivity"), "Missing trait module fact");
        assert!(has("'refund'"), "Missing interface method fact");
        assert!(has("'log_activity'"), "Missing trait method fact");

        // A function AFTER the bodiless namespace declaration scopes to it,
        // not to the path-derived file module.
        let ns_scoped = result.facts.iter().any(|f| {
            f.starts_with("function(")
                && f.contains("'currency_round'")
                && f.contains("app_payments")
        });
        assert!(
            ns_scoped,
            "currency_round should be scoped to the app_payments namespace module"
        );

        // Grouped use-import: one dependency per name.
        assert!(
            result
                .facts
                .iter()
                .any(|f| f.contains("depends_on") && f.contains("Clock")),
            "Missing Clock dependency from grouped use"
        );
        assert!(
            result
                .facts
                .iter()
                .any(|f| f.contains("depends_on") && f.contains("Money")),
            "Missing Money dependency from grouped use"
        );
    }

    #[test]
    fn test_rust_impl_methods_are_struct_qualified() {
        // Regression: before this fix, both `impl Order { fn new }` and
        // `impl OrderItem { fn new }` produced func_id `new_0`, silently
        // colliding when downstream tools query by id.  The Self type is now
        // prefixed onto the id: `order_new_0` vs `orderitem_new_0`.  Free
        // (non-impl) functions are unchanged: `fn run` → `run_0`.
        let mut analyzer = Analyzer::new().unwrap();

        let code = r#"
pub struct Order;
pub struct OrderItem;

impl Order {
    pub fn new() -> Self { Order }
    pub fn cancel(&self) {}
}

impl OrderItem {
    pub fn new() -> Self { OrderItem }
    pub fn cancel(&self) {}
}

pub fn run() {}
"#;

        let result = analyzer
            .lens_code(code, Language::Rust, "src/order.rs", "abc123")
            .unwrap();

        // Collect every function/4 id from the emitted Prolog facts.
        let ids: Vec<&str> = result
            .facts
            .iter()
            .filter(|f| f.starts_with("function("))
            .filter_map(|f| {
                let inner = f.strip_prefix("function(")?;
                let id_quoted = inner.split(',').next()?.trim();
                Some(id_quoted.trim_matches('\''))
            })
            .collect();

        // Both Order::new and OrderItem::new must coexist as distinct ids.
        assert!(
            ids.contains(&"src__order__order_new_0"),
            "expected src__order__order_new_0 in {ids:?}"
        );
        assert!(
            ids.contains(&"src__order__orderitem_new_0"),
            "expected src__order__orderitem_new_0 in {ids:?}"
        );

        // Same for the cancel methods — they have the same arity but different
        // Self types and must not collide.
        assert!(
            ids.contains(&"src__order__order_cancel_1"),
            "expected src__order__order_cancel_1 in {ids:?}"
        );
        assert!(
            ids.contains(&"src__order__orderitem_cancel_1"),
            "expected src__order__orderitem_cancel_1 in {ids:?}"
        );

        // The free function `run` is scoped by its module alone.
        assert!(
            ids.contains(&"src__order__run_0"),
            "free function run should be module-scoped, got {ids:?}"
        );

        // And there should be NO bare `new_0` id (would indicate collision).
        assert!(
            !ids.contains(&"new_0"),
            "bare new_0 should not exist — impl methods must be qualified: {ids:?}"
        );
    }
}
