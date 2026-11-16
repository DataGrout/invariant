//! `invariant check`: the dev-loop gate.
//!
//! Lenses what changed, works out what the change did to each file's functions
//! (the lines it touched, the functions it removed, the parameter counts it
//! changed) by analysing the file before and after, and asks the server's
//! `invariant.check` for the named problems.

use invariant_core::{Analyzer, Language};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// What a change touched on the new side: the line ranges it added or changed,
/// and its deletions, each the gap before a new-side line (so lines taken off a
/// function's end still count for that function).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Touched {
    pub ranges: Vec<(u32, u32)>,
    pub deletions: Vec<u32>,
    /// Deletions whose removed lines were decorators or attributes (`@login_required`,
    /// `#[cfg(test)]`): the gap before the function they decorated.
    pub decorator_deletions: Vec<u32>,
}

pub fn changed_ranges(before: &str, after: &str) -> Touched {
    let mut lines: Vec<u32> = Vec::new();
    let mut deletions: Vec<u32> = Vec::new();
    let mut decorator_deletions: Vec<u32> = Vec::new();

    if let Ok(patch) =
        git2::Patch::from_buffers(before.as_bytes(), None, after.as_bytes(), None, None)
    {
        for h in 0..patch.num_hunks() {
            let Ok((hunk, count)) = patch.hunk(h) else {
                continue;
            };
            let mut next_new = hunk.new_start().max(1);

            for l in 0..count {
                let Ok(line) = patch.line_in_hunk(h, l) else {
                    continue;
                };
                match line.origin() {
                    '+' => {
                        if let Some(n) = line.new_lineno() {
                            lines.push(n);
                            next_new = n + 1;
                        }
                    }
                    '-' => {
                        deletions.push(next_new);
                        if is_decorator(&String::from_utf8_lossy(line.content())) {
                            decorator_deletions.push(next_new);
                        }
                    }
                    ' ' => {
                        if let Some(n) = line.new_lineno() {
                            next_new = n + 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    deletions.sort_unstable();
    deletions.dedup();
    decorator_deletions.sort_unstable();
    decorator_deletions.dedup();
    Touched {
        ranges: to_ranges(lines),
        deletions,
        decorator_deletions,
    }
}

fn is_decorator(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('@') || t.starts_with("#[")
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The decorator or attribute lines directly above a function that starts at
/// `def_line` (1-based), as an inclusive range, multi-line decorators included.
/// The analyzer's function starts at its `def`, so without this a change that
/// only adds or removes a decorator (an authentication check, say) touches no
/// function and the gate stays silent.
fn decorator_block(lines: &[&str], def_line: u32) -> Option<(u32, u32)> {
    let d = def_line as usize;
    if d < 2 || d > lines.len() {
        return None;
    }
    let def_indent = indent(lines[d - 1]);
    let mut top: Option<usize> = None;
    let mut i = d - 1;
    while i >= 1 {
        let line = lines[i - 1];
        let t = line.trim();
        if t.is_empty() {
            break;
        }
        if is_decorator(line) {
            top = Some(i);
        } else if !(indent(line) > def_indent || t.starts_with(')') || t.starts_with(']')) {
            break;
        }
        i -= 1;
    }
    top.map(|t| (t as u32, def_line - 1))
}

/// Counts a change to a function's decorators as a change to the function: adds
/// its `def` line when a changed line falls in the decorator block above it, or
/// a removed decorator left its gap there.
pub fn with_decorated_functions(mut touched: Touched, after: &str, def_lines: &[u32]) -> Touched {
    let lines: Vec<&str> = after.lines().collect();
    let mut extra: Vec<u32> = Vec::new();

    for &d in def_lines {
        let block = decorator_block(&lines, d);
        let edited =
            block.is_some_and(|(b, e)| touched.ranges.iter().any(|&(s, t)| s <= e && t >= b));
        let removed = touched
            .decorator_deletions
            .iter()
            .any(|&p| p == d || block.is_some_and(|(b, _)| p >= b && p <= d));
        if edited || removed {
            extra.push(d);
        }
    }

    if !extra.is_empty() {
        let mut all: Vec<u32> = touched.ranges.iter().flat_map(|&(s, e)| s..=e).collect();
        all.extend(extra);
        touched.ranges = to_ranges(all);
    }
    touched
}

/// The start lines of the functions in a file's facts (`function/7`'s sixth
/// argument).
pub fn function_lines(facts: &[String]) -> Vec<u32> {
    facts
        .iter()
        .filter_map(|f| {
            let body = f.strip_prefix("function(")?;
            let args = split_args(body.trim_end_matches('.').trim_end_matches(')'));
            args.get(5)?.trim().parse().ok()
        })
        .collect()
}

fn to_ranges(mut lines: Vec<u32>) -> Vec<(u32, u32)> {
    lines.sort_unstable();
    lines.dedup();
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for n in lines {
        match ranges.last_mut() {
            Some((_, end)) if n <= *end + 1 => *end = n,
            _ => ranges.push((n, n)),
        }
    }
    ranges
}

/// A function as the analyzer names it: its module and name, and its arity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionShape {
    pub module: String,
    pub name: String,
    pub arity: u32,
}

/// The functions in a file's facts (`function/7`, or `function/4`).
pub fn function_shapes(facts: &[String]) -> Vec<FunctionShape> {
    facts
        .iter()
        .filter_map(|f| {
            let body = f.strip_prefix("function(")?;
            let args = split_args(body.trim_end_matches('.').trim_end_matches(')'));
            if args.len() < 4 {
                return None;
            }
            Some(FunctionShape {
                module: unquote(&args[1]),
                name: unquote(&args[2]),
                arity: args[3].trim().parse().ok()?,
            })
        })
        .collect()
}

fn split_args(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut depth = 0i32;

    for c in s.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
            continue;
        }
        match (quote, c) {
            (Some(_), '\\') => {
                cur.push(c);
                escaped = true;
            }
            (Some(q), c) if c == q => {
                quote = None;
                cur.push(c);
            }
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                cur.push(c);
            }
            (None, '(' | '[') => {
                depth += 1;
                cur.push(c);
            }
            (None, ')' | ']') => {
                depth -= 1;
                cur.push(c);
            }
            (None, ',') if depth == 0 => {
                args.push(cur.trim().to_string());
                cur.clear();
            }
            (None, c) => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        args.push(cur.trim().to_string());
    }
    args
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = s
        .strip_prefix('\'')
        .and_then(|x| x.strip_suffix('\''))
        .unwrap_or(s);
    inner.replace("\\'", "'").replace("\\\\", "\\")
}

/// What the change did to one file's functions: names that are gone, and
/// functions whose parameter count changed, as `(name, before, after)`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ShapeChanges {
    pub removed: Vec<String>,
    pub signatures: Vec<(String, u32, u32)>,
}

pub fn shape_changes(before: &[FunctionShape], after: &[FunctionShape]) -> ShapeChanges {
    let key = |f: &FunctionShape| (f.module.clone(), f.name.clone());
    let after_by_key: BTreeMap<_, _> = after.iter().map(|f| (key(f), f.arity)).collect();
    let after_names: std::collections::BTreeSet<_> = after.iter().map(|f| &f.name).collect();

    let mut changes = ShapeChanges::default();
    for f in before {
        match after_by_key.get(&key(f)) {
            Some(&arity) if arity != f.arity => {
                changes.signatures.push((f.name.clone(), f.arity, arity));
            }
            Some(_) => {}
            None if !after_names.contains(&f.name) => changes.removed.push(f.name.clone()),
            None => {}
        }
    }
    changes.removed.sort();
    changes.removed.dedup();
    changes.signatures.sort();
    changes.signatures.dedup();
    changes
}

/// One changed file's part of the request.
pub struct FileChange {
    pub path: String,
    pub touched: Touched,
    pub shapes: ShapeChanges,
}

/// Work out a changed file's ranges and function changes. `before` is None
/// for a new file, `after` None for a deleted one.
pub fn file_change(
    analyzer: &mut Analyzer,
    path: &str,
    language: Language,
    before: Option<&str>,
    after: Option<&str>,
) -> FileChange {
    let facts = |analyzer: &mut Analyzer, code: Option<&str>| -> Vec<String> {
        code.and_then(|c| analyzer.lens_code(c, language, path, "check").ok())
            .map(|r| r.facts)
            .unwrap_or_default()
    };

    let before_facts = facts(analyzer, before);
    let after_facts = facts(analyzer, after);

    FileChange {
        path: path.to_string(),
        touched: match after {
            Some(a) => with_decorated_functions(
                changed_ranges(before.unwrap_or(""), a),
                a,
                &function_lines(&after_facts),
            ),
            None => Touched::default(),
        },
        shapes: shape_changes(
            &function_shapes(&before_facts),
            &function_shapes(&after_facts),
        ),
    }
}

/// The `invariant.check` arguments for a set of file changes.
pub fn check_args(
    namespace: &str,
    files: &[FileChange],
    goal: Option<&str>,
    goal_ref: Option<&str>,
    max_blast_radius: Option<u32>,
) -> Value {
    let changes: Vec<Value> = files
        .iter()
        .filter(|f| !f.touched.ranges.is_empty() || !f.touched.deletions.is_empty())
        .map(|f| {
            json!({
                "file": f.path,
                "ranges": f.touched.ranges.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
                "deletions": f.touched.deletions,
            })
        })
        .collect();

    let removed: Vec<Value> = files
        .iter()
        .flat_map(|f| {
            f.shapes
                .removed
                .iter()
                .map(move |name| json!({"file": f.path, "name": name}))
        })
        .collect();

    let signatures: Vec<Value> = files
        .iter()
        .flat_map(|f| {
            f.shapes.signatures.iter().map(move |(name, before, after)| {
                json!({"file": f.path, "name": name, "before_params": before, "after_params": after})
            })
        })
        .collect();

    let mut args = json!({
        "namespace": namespace,
        "changes": changes,
        "removed": removed,
        "signature_changes": signatures,
    });
    if let Some(g) = goal {
        args["goal"] = json!(g);
    }
    if let Some(r) = goal_ref {
        args["goal_ref"] = json!(r);
    }
    if let Some(n) = max_blast_radius {
        args["max_blast_radius"] = json!(n);
    }
    args
}

/// The problems as plain lines, one per problem, for a terminal or for the
/// message an editor hook hands back to its agent.
pub fn problem_lines(result: &Value) -> Vec<String> {
    result["problems"]
        .as_array()
        .map(|ps| {
            ps.iter()
                .map(|p| {
                    let kind = p["kind"].as_str().unwrap_or("problem");
                    let msg = p["message"].as_str().unwrap_or("");
                    format!("[{kind}] {msg}")
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What Cursor's `stop` hook prints: a follow-up for the agent when the change
/// has problems, nothing to say when it is clean.
pub fn cursor_hook_output(result: &Value) -> Value {
    let lines = problem_lines(result);
    if lines.is_empty() {
        return json!({});
    }
    let mut msg = String::from(
        "invariant check found problems in the change you just made. Fix them, or say why \
         they don't apply:\n",
    );
    for l in &lines {
        msg.push_str("- ");
        msg.push_str(l);
        msg.push('\n');
    }
    json!({ "followup_message": msg })
}

/// The fields of Cursor's hook input this command reads.
pub struct HookInput {
    pub status: Option<String>,
    pub workspace_root: Option<String>,
}

pub fn parse_hook_input(stdin: &str) -> HookInput {
    let v: Value = serde_json::from_str(stdin).unwrap_or(Value::Null);
    HookInput {
        status: v["status"].as_str().map(String::from),
        workspace_root: v["workspace_roots"]
            .as_array()
            .and_then(|roots| roots.first())
            .and_then(|r| r.as_str())
            .map(String::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_ranges_cover_added_changed_and_removed_lines() {
        let before = "a\nb\nc\nd\ne\n";
        let after = "a\nB\nc\nnew\nd\n";
        // b -> B at line 2 (b deleted in the gap before it); "new" added at 4;
        // e taken off the end, in the gap before line 6.
        assert_eq!(
            changed_ranges(before, after),
            Touched {
                ranges: vec![(2, 2), (4, 4)],
                deletions: vec![2, 6],
                decorator_deletions: vec![],
            }
        );
    }

    fn decorated(before: &str, after: &str) -> Vec<(u32, u32)> {
        let mut analyzer = Analyzer::new().unwrap();
        let facts = analyzer
            .lens_code(after, Language::Python, "app/views.py", "check")
            .unwrap()
            .facts;
        with_decorated_functions(
            changed_ranges(before, after),
            after,
            &function_lines(&facts),
        )
        .ranges
    }

    #[test]
    fn removing_a_decorator_touches_the_function_it_decorated() {
        let before = "def a():\n    return 1\n\n@login_required\ndef view():\n    return 2\n";
        let after = "def a():\n    return 1\n\ndef view():\n    return 2\n";
        // Only a deletion, credited to the gap before line 4: `view`'s def.
        assert_eq!(decorated(before, after), vec![(4, 4)]);
    }

    #[test]
    fn adding_or_editing_a_decorator_touches_the_function() {
        let before = "def view():\n    return 2\n";
        let after = "@login_required\ndef view():\n    return 2\n";
        assert_eq!(decorated(before, after), vec![(1, 2)]);

        let before = "@app.route(\n    \"/a\",\n)\ndef view():\n    return 2\n";
        let after = "@app.route(\n    \"/b\",\n)\ndef view():\n    return 2\n";
        // The edit is inside a multi-line decorator, three lines above the def.
        assert_eq!(decorated(before, after), vec![(2, 2), (4, 4)]);
    }

    #[test]
    fn a_plain_deletion_before_a_function_does_not_touch_it() {
        let before = "def a():\n    x = 1\n    return x\ndef b():\n    return 2\n";
        let after = "def a():\n    x = 1\ndef b():\n    return 2\n";
        // `return x` came off a's end; b's def is not credited.
        assert_eq!(decorated(before, after), vec![]);
    }

    #[test]
    fn rust_attributes_count_as_decorators() {
        let lines = vec!["#[cfg(test)]", "#[inline]", "fn f() {}"];
        assert_eq!(decorator_block(&lines, 3), Some((1, 2)));
        assert_eq!(decorator_block(&["fn f() {}"], 1), None);
    }

    #[test]
    fn a_new_file_is_changed_throughout() {
        assert_eq!(changed_ranges("", "x\ny\nz\n").ranges, vec![(1, 3)]);
    }

    #[test]
    fn function_shapes_read_function_facts() {
        let facts = vec![
            "function('m__run_2', 'm', 'run', 2, public, 3, 'c1').".to_string(),
            "function('m__it\\'s_0', 'm', 'it\\'s', 0).".to_string(),
            "calls_external('m__run_2', 'unknown', 'x', 0, 4).".to_string(),
        ];
        assert_eq!(
            function_shapes(&facts),
            vec![
                FunctionShape {
                    module: "m".into(),
                    name: "run".into(),
                    arity: 2
                },
                FunctionShape {
                    module: "m".into(),
                    name: "it's".into(),
                    arity: 0
                },
            ]
        );
    }

    #[test]
    fn shape_changes_find_removals_and_parameter_count_changes() {
        let f = |m: &str, n: &str, a: u32| FunctionShape {
            module: m.into(),
            name: n.into(),
            arity: a,
        };
        let before = vec![
            f("m", "load", 1),
            f("m", "gone", 0),
            f("m", "same", 2),
            f("k", "moved", 1),
        ];
        let after = vec![f("m", "load", 2), f("m", "same", 2), f("j", "moved", 1)];

        assert_eq!(
            shape_changes(&before, &after),
            ShapeChanges {
                removed: vec!["gone".into()],
                signatures: vec![("load".into(), 1, 2)],
            }
        );
    }

    #[test]
    fn signature_changes_found_by_analysing_both_versions() {
        let mut analyzer = Analyzer::new().unwrap();
        let before = "def load(order_id):\n    return 1\n\ndef old():\n    return 2\n";
        let after = "def load(order_id, cache):\n    return 1\n";
        let change = file_change(
            &mut analyzer,
            "app/orders.py",
            Language::Python,
            Some(before),
            Some(after),
        );

        assert_eq!(change.shapes.removed, vec!["old".to_string()]);
        assert_eq!(change.shapes.signatures, vec![("load".to_string(), 1, 2)]);
        assert!(!change.touched.ranges.is_empty());
    }

    #[test]
    fn check_args_carry_changes_removals_and_signatures() {
        let files = vec![FileChange {
            path: "app/orders.py".into(),
            touched: Touched {
                ranges: vec![(2, 3)],
                deletions: vec![5],
                decorator_deletions: vec![],
            },
            shapes: ShapeChanges {
                removed: vec!["old".into()],
                signatures: vec![("load".into(), 1, 2)],
            },
        }];
        let args = check_args("code_acme_shop", &files, None, Some("ABC-1"), Some(10));

        assert_eq!(
            args["changes"],
            json!([{"file": "app/orders.py", "ranges": [[2, 3]], "deletions": [5]}])
        );
        assert_eq!(
            args["removed"],
            json!([{"file": "app/orders.py", "name": "old"}])
        );
        assert_eq!(args["signature_changes"][0]["after_params"], json!(2));
        assert_eq!(args["goal_ref"], json!("ABC-1"));
        assert_eq!(args["max_blast_radius"], json!(10));
    }

    #[test]
    fn the_cursor_hook_says_nothing_for_a_clean_change() {
        assert_eq!(cursor_hook_output(&json!({"problems": []})), json!({}));

        let out = cursor_hook_output(&json!({"problems": [
            {"kind": "untested_change", "message": "price changed and no test reaches it."}
        ]}));
        let msg = out["followup_message"].as_str().unwrap();
        assert!(msg.contains("- [untested_change] price changed and no test reaches it."));
    }

    #[test]
    fn hook_input_reads_status_and_workspace() {
        let h = parse_hook_input(
            r#"{"status":"completed","loop_count":0,"workspace_roots":["/w/repo"]}"#,
        );
        assert_eq!(h.status.as_deref(), Some("completed"));
        assert_eq!(h.workspace_root.as_deref(), Some("/w/repo"));
        assert!(parse_hook_input("not json").status.is_none());
    }
}
