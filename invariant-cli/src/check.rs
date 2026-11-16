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
}

pub fn changed_ranges(before: &str, after: &str) -> Touched {
    let mut lines: Vec<u32> = Vec::new();
    let mut deletions: Vec<u32> = Vec::new();

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
                    '-' => deletions.push(next_new),
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
    Touched {
        ranges: to_ranges(lines),
        deletions,
    }
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
    let facts = |analyzer: &mut Analyzer, code: Option<&str>| -> Vec<FunctionShape> {
        code.and_then(|c| analyzer.lens_code(c, language, path, "check").ok())
            .map(|r| function_shapes(&r.facts))
            .unwrap_or_default()
    };

    let before_shapes = facts(analyzer, before);
    let after_shapes = facts(analyzer, after);

    FileChange {
        path: path.to_string(),
        touched: match after {
            Some(a) => changed_ranges(before.unwrap_or(""), a),
            None => Touched::default(),
        },
        shapes: shape_changes(&before_shapes, &after_shapes),
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
            }
        );
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
