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
        let tree = self.parser.parse(code, language)?;
        let root = tree.root_node();

        let mut facts = Vec::new();
        let mut summary = AnalysisSummary::default();

        let ctx = &mut Ctx {
            code: code.as_bytes(),
            filepath,
            commit_sha,
            facts: &mut facts,
            summary: &mut summary,
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

    ctx.facts.push(Fact::new(
        "function_visibility",
        vec![
            FactValue::String(func_id.to_string()),
            FactValue::Atom(visibility.to_string()),
        ],
    ));

    emit_params(ctx, func_id, fn_node);

    ctx.summary.functions += 1;
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
                analyze_python_function(&child, module_id, ctx)?;
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

fn analyze_python_function(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let name_node = node
        .child_by_field_name("name")
        .ok_or_else(|| Error::Parse("Function has no name".to_string()))?;
    let func_name = node_text(&name_node, ctx.code);
    let arity = param_count(node, "parameters");
    let func_id = normalize_id(&format!("{}_{}", func_name, arity));
    let line = node_line(node);

    let visibility = if func_name.starts_with("__") && func_name.ends_with("__") {
        "public"
    } else if func_name.starts_with("__") {
        "private"
    } else if func_name.starts_with('_') {
        "protected"
    } else {
        "public"
    };

    emit_function(ctx, &func_id, module_id, func_name, arity, visibility, line, node);

    if let Some(body) = node.child_by_field_name("body") {
        analyze_calls(&body, ctx.code, &func_id, ctx, "call")?;
    }

    Ok(())
}

fn analyze_python_class(node: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    let name_node = node
        .child_by_field_name("name")
        .ok_or_else(|| Error::Parse("Class has no name".to_string()))?;
    let class_name = node_text(&name_node, ctx.code);
    let class_id = normalize_id(class_name);

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

    let mut cursor = root.walk();
    for node in root.children(&mut cursor) {
        match node.kind() {
            "function_item" => {
                analyze_rust_function(&node, &module_id, None, ctx)?;
            }
            "struct_item" | "enum_item" | "trait_item" => {
                analyze_rust_type(&node, &module_id, ctx)?;
            }
            "impl_item" => {
                analyze_rust_impl(&node, &module_id, ctx)?;
            }
            "use_declaration" => {
                analyze_rust_use(&node, &module_id, ctx)?;
            }
            "mod_item" => {
                analyze_rust_mod(&node, &module_id, ctx)?;
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
            normalize_id(&format!("{}_{}_{}", prefix, func_name, arity))
        }
        _ => normalize_id(&format!("{}_{}", func_name, arity)),
    };
    let line = node_line(node);

    let visibility = if has_pub_modifier(node, ctx.code) {
        "public"
    } else {
        "private"
    };

    emit_function(ctx, &func_id, module_id, func_name, arity, visibility, line, node);

    if let Some(body) = node.child_by_field_name("body") {
        analyze_calls(&body, ctx.code, &func_id, ctx, "call_expression")?;
    }

    Ok(())
}

fn analyze_rust_type(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let type_name = node_text(&name_node, ctx.code);
        let type_id = normalize_id(type_name);
        emit_module(
            ctx,
            &type_id,
            type_name,
            &format!("type:{}", module_id),
            node_line(node),
        );
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
    let text = node_text(node, ctx.code);

    if let Some(used_module) = text
        .strip_prefix("use ")
        .and_then(|s| s.split_whitespace().next())
    {
        let used_module = used_module.trim_end_matches(';').replace("::", ".");
        emit_dependency(ctx, module_id, &used_module, "use", node_line(node));
    }

    Ok(())
}

fn analyze_rust_mod(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let mod_name = node_text(&name_node, ctx.code);
        emit_dependency(ctx, module_id, mod_name, "mod", node_line(node));
    }

    Ok(())
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
    let module_name = ctx
        .filepath
        .trim_end_matches(".js")
        .trim_end_matches(".ts")
        .trim_end_matches(".jsx")
        .trim_end_matches(".tsx")
        .replace(['/', '\\'], ".");
    let module_id = normalize_id(&module_name);
    emit_module(ctx, &module_id, &module_name, ctx.filepath, 1);

    analyze_js_node(root, &module_id, ctx)
}

fn analyze_js_node(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "method_definition" | "arrow_function" => {
                analyze_js_function(&child, module_id, ctx)?;
            }
            "class_declaration" => {
                analyze_js_class(&child, ctx)?;
            }
            "import_statement" => {
                analyze_js_import(&child, module_id, ctx)?;
            }
            _ => {
                analyze_js_node(&child, module_id, ctx)?;
            }
        }
    }

    Ok(())
}

fn analyze_js_function(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(&n, ctx.code).to_string())
        .unwrap_or_else(|| "anonymous".to_string());

    let arity = param_count(node, "parameters");
    let func_id = normalize_id(&format!("{}_{}", name, arity));
    let line = node_line(node);

    emit_function(ctx, &func_id, module_id, &name, arity, "public", line, node);

    if let Some(body) = node.child_by_field_name("body") {
        analyze_calls(&body, ctx.code, &func_id, ctx, "call_expression")?;
    }

    Ok(())
}

fn analyze_js_class(node: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let class_name = node_text(&name_node, ctx.code);
        let class_id = normalize_id(class_name);

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
    let text = node_text(node, ctx.code);

    if let Some(from_idx) = text.find("from") {
        if let Some(quote_start) = text[from_idx..].find(['"', '\'']) {
            let remaining = &text[from_idx + quote_start + 1..];
            if let Some(quote_end) = remaining.find(['"', '\'']) {
                let imported_module = &remaining[..quote_end];
                emit_dependency(ctx, module_id, imported_module, "import", node_line(node));
            }
        }
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
    let func_id = normalize_id(&format!("{}_{}", name, arity));
    let line = node_line(node);

    let visibility = if name.starts_with(|c: char| c.is_uppercase()) {
        "public"
    } else {
        "private"
    };

    emit_function(ctx, &func_id, module_id, &name, arity, visibility, line, node);

    if let Some(body) = node.child_by_field_name("body") {
        analyze_calls(&body, ctx.code, &func_id, ctx, "call_expression")?;
    }

    Ok(())
}

fn analyze_go_type(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let text = node_text(node, ctx.code);

    if let Some(type_name) = text.split_whitespace().nth(1) {
        emit_dependency(ctx, module_id, type_name, "type", node_line(node));
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
    analyze_elixir_node(root, &module_id, ctx)
}

fn analyze_elixir_node(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "call" => {
                if let Some(target) = child.child_by_field_name("target") {
                    let target_text = node_text(&target, ctx.code);
                    match target_text {
                        "defmodule" => analyze_elixir_defmodule(&child, ctx)?,
                        "def" | "defp" => {
                            analyze_elixir_function(&child, module_id, target_text, ctx)?
                        }
                        "import" | "use" | "alias" => {
                            analyze_elixir_dep(&child, module_id, target_text, ctx)?
                        }
                        _ => analyze_elixir_node(&child, module_id, ctx)?,
                    }
                } else {
                    analyze_elixir_node(&child, module_id, ctx)?;
                }
            }
            _ => {
                analyze_elixir_node(&child, module_id, ctx)?;
            }
        }
    }

    Ok(())
}

fn analyze_elixir_defmodule(node: &Node<'_>, ctx: &mut Ctx<'_>) -> Result<()> {
    // In tree-sitter-elixir, `defmodule Foo do...end` is a `call` node with children:
    //   target: identifier("defmodule")       [field]
    //   arguments: (alias("Foo"))             [no field, find by kind]
    //   do_block: do...end                    [no field, find by kind]

    let module_name = find_child_by_kind(node, "arguments")
        .and_then(|a| a.named_child(0))
        .map(|n| node_text(&n, ctx.code).to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let module_id = normalize_id(&module_name);
    emit_module(
        ctx,
        &module_id,
        &module_name,
        &format!("defmodule:{}", module_name),
        node_line(node),
    );

    if let Some(do_block) = find_child_by_kind(node, "do_block") {
        analyze_elixir_node(&do_block, &module_id, ctx)?;
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

    let func_id = normalize_id(&format!("{}_{}", func_name, arity));
    let line = node_line(node);
    let visibility = if kind == "defp" { "private" } else { "public" };

    emit_function(
        ctx, &func_id, module_id, &func_name, arity, visibility, line, node,
    );

    if let Some(do_block) = find_child_by_kind(node, "do_block") {
        analyze_elixir_calls(&do_block, ctx.code, &func_id, ctx)?;
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
                if !matches!(
                    called,
                    "def" | "defp" | "defmodule" | "import" | "use" | "alias" | "do" | "end"
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

    let class_id = normalize_id(&class_name);
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

    let module_id = normalize_id(&module_name);
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

    let func_id = normalize_id(&format!("{}_{}", name, arity));
    let line = node_line(node);

    emit_function(ctx, &func_id, module_id, &name, arity, visibility, line, node);

    if let Some(body) = node.child_by_field_name("body") {
        analyze_ruby_calls(&body, ctx.code, &func_id, ctx)?;
    }

    Ok(())
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

fn analyze_ruby_include(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let text = node_text(node, ctx.code);
    let parts: Vec<&str> = text.split_whitespace().collect();

    if parts.len() >= 2 {
        let kind = parts[0];
        let included = parts[1];
        emit_dependency(ctx, module_id, included, kind, node_line(node));
    }

    Ok(())
}

fn analyze_ruby_calls(
    node: &Node<'_>,
    code: &[u8],
    func_id: &str,
    ctx: &mut Ctx<'_>,
) -> Result<()> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(method) = child.child_by_field_name("method") {
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
        }
        analyze_ruby_calls(&child, code, func_id, ctx)?;
    }

    Ok(())
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
    if !text.contains('\n') && !text.contains('(') {
        return text.trim().to_string();
    }
    // Take the part before the first `(`, then the final `.`/`::`-segment.
    let head = text.split('(').next().unwrap_or(text);
    let head = head.replace(['\n', '\r', '\t', ' '], "");
    head.rsplit(['.'])
        .next()
        .unwrap_or(&head)
        .to_string()
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
        assert!(!out.contains('\n') && !out.contains('(') && !out.contains(' '), "not clean: {out:?}");
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
        assert!(joined.contains("git_output"), "missing git_output: {joined}");
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
        assert!(joined.contains("'n'") && joined.contains("'u8'"), "typed param missing: {joined}");
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
        let js = params(&mut analyzer, "function f(a, b) { return a; }", Language::JavaScript, "x.js");
        assert!(js.contains("'a'") && js.contains("'b'"), "js: {js}");
        assert!(js.contains("unknown"), "js untyped should be unknown: {js}");

        // TypeScript — type captured WITHOUT the leading colon.
        let ts = params(
            &mut analyzer,
            "function g(a: number, b: string): number { return a; }",
            Language::TypeScript,
            "x.ts",
        );
        assert!(ts.contains("'number'"), "ts type should be bare 'number': {ts}");
        assert!(ts.contains("'string'"), "ts type should be bare 'string': {ts}");
        assert!(!ts.contains(": number"), "ts type must not keep the colon: {ts}");

        // Go — typed params.
        let go = params(&mut analyzer, "func Add(a int, b int) int { return a }", Language::Go, "x.go");
        assert!(go.contains("'a'") && go.contains("'int'"), "go: {go}");

        // Ruby — names, dynamically typed → unknown.
        let rb = params(&mut analyzer, "def greet(name, count)\n  name\nend\n", Language::Ruby, "x.rb");
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
        assert!(joined.contains("unknown"), "untyped params should be unknown: {joined}");
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
            ids.contains(&"order_new_0"),
            "expected order_new_0 in {ids:?}"
        );
        assert!(
            ids.contains(&"orderitem_new_0"),
            "expected orderitem_new_0 in {ids:?}"
        );

        // Same for the cancel methods — they have the same arity but different
        // Self types and must not collide.
        assert!(
            ids.contains(&"order_cancel_1"),
            "expected order_cancel_1 in {ids:?}"
        );
        assert!(
            ids.contains(&"orderitem_cancel_1"),
            "expected orderitem_cancel_1 in {ids:?}"
        );

        // The free function `run` keeps the legacy unqualified form.
        assert!(
            ids.contains(&"run_0"),
            "free function run should remain unqualified, got {ids:?}"
        );

        // And there should be NO bare `new_0` id (would indicate collision).
        assert!(
            !ids.contains(&"new_0"),
            "bare new_0 should not exist — impl methods must be qualified: {ids:?}"
        );
    }
}
