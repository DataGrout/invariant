//! Unified diff / patch parser.
//!
//! Parses `git diff` output or `.patch` files into [`FileDiff`] structs
//! compatible with the git module, enabling `--patch` and `--stdin` modes.

use crate::git::{DiffStatus, FileDiff};
use crate::parser::Language;
use anyhow::{Context, Result};
use std::path::Path;

/// Parse unified diff text (as produced by `git diff`) into file diffs.
///
/// For each file in the diff, reconstructs the before and after content
/// from the hunk headers and changed lines. Only supports the subset of
/// unified diff that `git diff` produces.
pub fn parse_unified_diff(input: &str) -> Result<Vec<FileDiff>> {
    let mut results = Vec::new();
    let mut lines = input.lines().peekable();

    while lines.peek().is_some() {
        // Scan forward to next "diff --git" header
        let header = loop {
            match lines.next() {
                Some(line) if line.starts_with("diff --git ") => break line,
                Some(_) => continue,
                None => return Ok(results),
            }
        };

        let (old_path, new_path) = parse_diff_header(header)?;

        // Consume metadata lines (index, old mode, new mode, similarity, etc.)
        let mut status = DiffStatus::Modified;
        while let Some(&line) = lines.peek() {
            if line.starts_with("new file mode") {
                status = DiffStatus::Added;
                lines.next();
            } else if line.starts_with("deleted file mode") {
                status = DiffStatus::Deleted;
                lines.next();
            } else if line.starts_with("rename from") || line.starts_with("rename to") {
                status = DiffStatus::Renamed;
                lines.next();
            } else if line.starts_with("index ")
                || line.starts_with("old mode")
                || line.starts_with("new mode")
                || line.starts_with("similarity")
                || line.starts_with("dissimilarity")
                || line.starts_with("copy from")
                || line.starts_with("copy to")
            {
                lines.next();
            } else {
                break;
            }
        }

        // Consume --- and +++ lines
        if let Some(&line) = lines.peek() {
            if line.starts_with("---") {
                lines.next();
            }
        }
        if let Some(&line) = lines.peek() {
            if line.starts_with("+++") {
                lines.next();
            }
        }

        // Collect hunks to build before/after content
        let mut before_lines: Vec<String> = Vec::new();
        let mut after_lines: Vec<String> = Vec::new();
        let mut has_hunks = false;

        while let Some(&line) = lines.peek() {
            if line.starts_with("diff --git ") {
                break;
            }

            if line.starts_with("@@") {
                has_hunks = true;
                lines.next();
                continue;
            }

            if !has_hunks {
                // Binary diff or empty — skip
                lines.next();
                continue;
            }

            if let Some(content) = line.strip_prefix('-') {
                before_lines.push(content.to_string());
                lines.next();
            } else if let Some(content) = line.strip_prefix('+') {
                after_lines.push(content.to_string());
                lines.next();
            } else if line.starts_with('\\') {
                // "\ No newline at end of file"
                lines.next();
            } else {
                // Context line (starts with space or is plain)
                let content = line.strip_prefix(' ').unwrap_or(line);
                before_lines.push(content.to_string());
                after_lines.push(content.to_string());
                lines.next();
            }
        }

        let path = if new_path != "/dev/null" {
            new_path.clone()
        } else {
            old_path.clone()
        };

        let language = Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Language::from_extension);

        let before = if status == DiffStatus::Added || before_lines.is_empty() {
            None
        } else {
            Some(before_lines.join("\n"))
        };

        let after = if status == DiffStatus::Deleted || after_lines.is_empty() {
            None
        } else {
            Some(after_lines.join("\n"))
        };

        results.push(FileDiff {
            path,
            language,
            status,
            before,
            after,
        });
    }

    Ok(results)
}

/// Parse `diff --git a/path b/path` header into (old_path, new_path).
fn parse_diff_header(header: &str) -> Result<(String, String)> {
    let rest = header
        .strip_prefix("diff --git ")
        .context("Invalid diff header")?;

    // Try quoted paths first, then unquoted
    // Format: a/path b/path (or "a/path" "b/path" for paths with spaces)
    if let Some(stripped) = rest.strip_prefix('"') {
        // Quoted paths — find matching closing quotes
        let end_first = stripped
            .find('"')
            .context("Malformed quoted diff header")?;
        let first = &stripped[..end_first];
        let second_part = stripped[end_first + 1..].trim_start();
        let second = if second_part.starts_with('"') {
            second_part.trim_matches('"')
        } else {
            second_part
        };
        Ok((
            strip_ab_prefix(first),
            strip_ab_prefix(second),
        ))
    } else {
        // Unquoted: split on " b/" boundary
        // Find the last occurrence of " b/" to handle paths with spaces
        if let Some(pos) = rest.rfind(" b/") {
            let old = &rest[..pos];
            let new = &rest[pos + 1..];
            Ok((strip_ab_prefix(old), strip_ab_prefix(new)))
        } else {
            // Fallback: split in half
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() == 2 {
                Ok((
                    strip_ab_prefix(parts[0]),
                    strip_ab_prefix(parts[1]),
                ))
            } else {
                anyhow::bail!("Cannot parse diff header: {header}")
            }
        }
    }
}

/// Remove the `a/` or `b/` prefix that git adds to diff paths.
fn strip_ab_prefix(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("a/") {
        rest.to_string()
    } else if let Some(rest) = path.strip_prefix("b/") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_DIFF: &str = r#"diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,6 @@
 fn main() {
-    println!("hello");
+    println!("hello world");
+    println!("goodbye");
     let x = 1;
     let y = 2;
 }
"#;

    #[test]
    fn parse_simple_modification() {
        let diffs = parse_unified_diff(SAMPLE_DIFF).unwrap();
        assert_eq!(diffs.len(), 1);
        let d = &diffs[0];
        assert_eq!(d.path, "src/main.rs");
        assert_eq!(d.status, DiffStatus::Modified);
        assert!(d.before.is_some());
        assert!(d.after.is_some());

        let before = d.before.as_ref().unwrap();
        assert!(before.contains("println!(\"hello\");"));
        assert!(!before.contains("hello world"));

        let after = d.after.as_ref().unwrap();
        assert!(after.contains("hello world"));
        assert!(after.contains("goodbye"));
    }

    const ADDED_FILE_DIFF: &str = r#"diff --git a/new_file.py b/new_file.py
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/new_file.py
@@ -0,0 +1,3 @@
+def hello():
+    print("hello")
+    return True
"#;

    #[test]
    fn parse_added_file() {
        let diffs = parse_unified_diff(ADDED_FILE_DIFF).unwrap();
        assert_eq!(diffs.len(), 1);
        let d = &diffs[0];
        assert_eq!(d.path, "new_file.py");
        assert_eq!(d.status, DiffStatus::Added);
        assert!(d.before.is_none());
        assert!(d.after.is_some());
        assert!(d.after.as_ref().unwrap().contains("def hello():"));
    }

    const DELETED_FILE_DIFF: &str = r#"diff --git a/old.rs b/old.rs
deleted file mode 100644
index abc1234..0000000
--- a/old.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-fn deprecated() {}
-fn also_gone() {}
"#;

    #[test]
    fn parse_deleted_file() {
        let diffs = parse_unified_diff(DELETED_FILE_DIFF).unwrap();
        assert_eq!(diffs.len(), 1);
        let d = &diffs[0];
        assert_eq!(d.path, "old.rs");
        assert_eq!(d.status, DiffStatus::Deleted);
        assert!(d.before.is_some());
        assert!(d.after.is_none());
    }

    const MULTI_FILE_DIFF: &str = r#"diff --git a/a.py b/a.py
index abc..def 100644
--- a/a.py
+++ b/a.py
@@ -1,3 +1,3 @@
 x = 1
-y = 2
+y = 3
 z = 4
diff --git a/b.py b/b.py
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/b.py
@@ -0,0 +1,2 @@
+a = 10
+b = 20
"#;

    #[test]
    fn parse_multi_file_diff() {
        let diffs = parse_unified_diff(MULTI_FILE_DIFF).unwrap();
        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[0].path, "a.py");
        assert_eq!(diffs[0].status, DiffStatus::Modified);
        assert_eq!(diffs[1].path, "b.py");
        assert_eq!(diffs[1].status, DiffStatus::Added);
    }

    #[test]
    fn parse_diff_header_unquoted() {
        let (old, new) = parse_diff_header("diff --git a/src/lib.rs b/src/lib.rs").unwrap();
        assert_eq!(old, "src/lib.rs");
        assert_eq!(new, "src/lib.rs");
    }

    #[test]
    fn strip_ab_prefix_works() {
        assert_eq!(strip_ab_prefix("a/foo.rs"), "foo.rs");
        assert_eq!(strip_ab_prefix("b/bar/baz.py"), "bar/baz.py");
        assert_eq!(strip_ab_prefix("plain.txt"), "plain.txt");
    }

    const RENAMED_FILE_DIFF: &str = r#"diff --git a/old_name.rs b/new_name.rs
similarity index 95%
rename from old_name.rs
rename to new_name.rs
index abc1234..def5678 100644
--- a/old_name.rs
+++ b/new_name.rs
@@ -1,3 +1,3 @@
 fn greet() {
-    println!("old");
+    println!("new");
 }
"#;

    #[test]
    fn parse_renamed_file() {
        let diffs = parse_unified_diff(RENAMED_FILE_DIFF).unwrap();
        assert_eq!(diffs.len(), 1);
        let d = &diffs[0];
        assert_eq!(d.path, "new_name.rs");
        assert_eq!(d.status, DiffStatus::Renamed);
        assert!(d.before.is_some());
        assert!(d.after.is_some());
        assert!(d.before.as_ref().unwrap().contains("\"old\""));
        assert!(d.after.as_ref().unwrap().contains("\"new\""));
    }

    const BINARY_DIFF: &str = r#"diff --git a/image.png b/image.png
index abc1234..def5678 100644
Binary files a/image.png and b/image.png differ
diff --git a/src/lib.rs b/src/lib.rs
index 111..222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
-fn old() {}
+fn new() {}
"#;

    #[test]
    fn parse_binary_diff_skipped_gracefully() {
        let diffs = parse_unified_diff(BINARY_DIFF).unwrap();
        // Binary file should still appear but with no before/after content
        let binary = diffs.iter().find(|d| d.path == "image.png");
        assert!(binary.is_some());
        let b = binary.unwrap();
        assert!(b.before.is_none() || b.after.is_none());

        // The text file after the binary should parse fine
        let text = diffs.iter().find(|d| d.path == "src/lib.rs");
        assert!(text.is_some());
        assert!(text.unwrap().after.as_ref().unwrap().contains("fn new()"));
    }

    #[test]
    fn parse_empty_diff() {
        let diffs = parse_unified_diff("").unwrap();
        assert!(diffs.is_empty());
    }

    #[test]
    fn parse_no_newline_at_eof() {
        let input = "diff --git a/f.txt b/f.txt\n\
                      index abc..def 100644\n\
                      --- a/f.txt\n\
                      +++ b/f.txt\n\
                      @@ -1,2 +1,2 @@\n\
                       line1\n\
                      -line2\n\
                      +line2_changed\n\
                      \\ No newline at end of file\n";

        let diffs = parse_unified_diff(input).unwrap();
        assert_eq!(diffs.len(), 1);
        let d = &diffs[0];
        assert!(d.after.as_ref().unwrap().contains("line2_changed"));
        // The "\ No newline" marker should not appear in content
        assert!(!d.after.as_ref().unwrap().contains("No newline"));
    }

    #[test]
    fn parse_diff_header_quoted_paths() {
        let header = r#"diff --git "a/path with spaces/file.rs" "b/path with spaces/file.rs""#;
        let (old, new) = parse_diff_header(header).unwrap();
        assert_eq!(old, "path with spaces/file.rs");
        assert_eq!(new, "path with spaces/file.rs");
    }

    #[test]
    fn parse_diff_header_rename_different_paths() {
        let (old, new) =
            parse_diff_header("diff --git a/src/old.rs b/src/new.rs").unwrap();
        assert_eq!(old, "src/old.rs");
        assert_eq!(new, "src/new.rs");
    }

    #[test]
    fn language_detection_from_patch() {
        let diffs = parse_unified_diff(ADDED_FILE_DIFF).unwrap();
        assert_eq!(diffs[0].language, Some(Language::Python));

        let diffs = parse_unified_diff(SAMPLE_DIFF).unwrap();
        assert_eq!(diffs[0].language, Some(Language::Rust));
    }
}
