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

fn emit_function(
    ctx: &mut Ctx<'_>,
    func_id: &str,
    module_id: &str,
    name: &str,
    arity: usize,
    visibility: &str,
    line: usize,
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

    ctx.summary.functions += 1;
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

    emit_function(ctx, &func_id, module_id, func_name, arity, visibility, line);

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
                analyze_rust_function(&node, &module_id, ctx)?;
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

fn analyze_rust_function(node: &Node<'_>, module_id: &str, ctx: &mut Ctx<'_>) -> Result<()> {
    let name_node = node
        .child_by_field_name("name")
        .ok_or_else(|| Error::Parse("Function has no name".to_string()))?;
    let func_name = node_text(&name_node, ctx.code);
    let arity = param_count(node, "parameters");
    let func_id = normalize_id(&format!("{}_{}", func_name, arity));
    let line = node_line(node);

    let visibility = if has_pub_modifier(node, ctx.code) {
        "public"
    } else {
        "private"
    };

    emit_function(ctx, &func_id, module_id, func_name, arity, visibility, line);

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
    let body = node.child_by_field_name("body").unwrap_or(*node);
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_item" {
            analyze_rust_function(&child, module_id, ctx)?;
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

    emit_function(ctx, &func_id, module_id, &name, arity, "public", line);

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

    emit_function(ctx, &func_id, module_id, &name, arity, visibility, line);

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
        ctx, &func_id, module_id, &func_name, arity, visibility, line,
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
                let called_func = node_text(&func_node, code);
                emit_call(ctx, func_id, called_func, node_line(&child));
            }
        }
        analyze_calls(&child, code, func_id, ctx, call_kind)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
