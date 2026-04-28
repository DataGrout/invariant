//! Git integration — repo identity, diff extraction, and revision resolution.
//!
//! Wraps `git2` to provide ergonomic access to git state for invariant's
//! diff and lens workflows. All functions operate on the repo discovered
//! from the current working directory.

use crate::parser::Language;
use anyhow::{Context, Result};
use git2::{Delta, DiffOptions, Repository};
use std::path::Path;

/// Status of a file in a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffStatus {
    /// New file added
    Added,
    /// Existing file modified
    Modified,
    /// File deleted
    Deleted,
    /// File renamed (with possible content change)
    Renamed,
}

/// A single file's before/after content extracted from a git diff.
#[derive(Debug, Clone)]
pub struct FileDiff {
    /// Relative path within the repo (uses the "after" path for renames)
    pub path: String,
    /// Detected language based on file extension
    pub language: Option<Language>,
    /// What happened to this file
    pub status: DiffStatus,
    /// Content before the change (None for added files)
    pub before: Option<String>,
    /// Content after the change (None for deleted files)
    pub after: Option<String>,
}

/// Result of `detect_repo_context` — repo identity + current commit.
#[derive(Debug, Clone)]
pub struct RepoContext {
    /// Stable repo identifier (from remote URL or folder name)
    pub repo_id: String,
    /// Current HEAD commit SHA
    pub commit_sha: String,
}

/// Discover the git repository from the current directory.
pub fn discover_repo() -> Result<Repository> {
    Repository::discover(".").context("Not a git repository (or any parent)")
}

/// Derive a stable `repo_id` from the first remote URL, falling back to
/// the workdir folder name.
///
/// Normalizes SSH and HTTPS URLs:
///   `git@github.com:owner/repo.git` → `owner/repo`
///   `https://github.com/owner/repo.git` → `owner/repo`
pub fn resolve_repo_id(repo: &Repository) -> String {
    if let Ok(remote) = repo.find_remote("origin") {
        if let Some(url) = remote.url() {
            if let Some(id) = normalize_remote_url(url) {
                return id;
            }
        }
    }

    // Fallback: iterate all remotes
    if let Ok(remotes) = repo.remotes() {
        for name in remotes.iter().flatten() {
            if let Ok(remote) = repo.find_remote(name) {
                if let Some(url) = remote.url() {
                    if let Some(id) = normalize_remote_url(url) {
                        return id;
                    }
                }
            }
        }
    }

    // Final fallback: workdir folder name
    repo.workdir()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Extract `owner/repo` from a git remote URL.
fn normalize_remote_url(url: &str) -> Option<String> {
    // SSH: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@") {
        let after_host = rest.find(':').map(|i| &rest[i + 1..])?;
        let cleaned = clean_repo_path(after_host);
        if !cleaned.is_empty() {
            return Some(cleaned);
        }
    }

    // HTTPS/HTTP: https://github.com/owner/repo.git
    if url.starts_with("https://") || url.starts_with("http://") {
        let after_scheme = url.split("://").nth(1)?;
        let after_host = after_scheme.find('/').map(|i| &after_scheme[i + 1..])?;
        let cleaned = clean_repo_path(after_host);
        if !cleaned.is_empty() {
            return Some(cleaned);
        }
    }

    // ssh://git@github.com/owner/repo.git
    if url.starts_with("ssh://") {
        let after_scheme = url.strip_prefix("ssh://")?;
        let after_host = after_scheme.find('/').map(|i| &after_scheme[i + 1..])?;
        let cleaned = clean_repo_path(after_host);
        if !cleaned.is_empty() {
            return Some(cleaned);
        }
    }

    None
}

/// Strip trailing slashes and `.git` suffix from a repo path segment.
fn clean_repo_path(path: &str) -> String {
    path.trim_matches('/')
        .trim_end_matches(".git")
        .trim_matches('/')
        .to_string()
}

/// Get repo context: stable `repo_id` + current HEAD commit SHA.
pub fn detect_repo_context() -> Result<RepoContext> {
    let repo = discover_repo()?;
    let repo_id = resolve_repo_id(&repo);

    let commit_sha = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| c.id().to_string())
        .unwrap_or_else(|| "HEAD".to_string());

    Ok(RepoContext {
        repo_id,
        commit_sha,
    })
}

/// Diff staged changes (index) against HEAD.
/// This is the default when the user runs `invariant diff --goal "..."` with no rev.
pub fn diff_staged() -> Result<Vec<FileDiff>> {
    let repo = discover_repo()?;
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());

    let mut opts = DiffOptions::new();
    opts.include_untracked(false);

    let diff = repo
        .diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts))
        .context("Failed to diff index against HEAD")?;

    extract_file_diffs(&repo, &diff)
}

/// Diff a single commit against its parent(s).
pub fn diff_commit(rev: &str) -> Result<Vec<FileDiff>> {
    let repo = discover_repo()?;
    let obj = repo
        .revparse_single(rev)
        .with_context(|| format!("Cannot resolve revision '{rev}'"))?;
    let commit = obj
        .peel_to_commit()
        .with_context(|| format!("'{rev}' is not a commit"))?;

    let tree = commit.tree()?;

    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };

    let diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
        .context("Failed to compute commit diff")?;

    extract_file_diffs(&repo, &diff)
}

/// Diff between two revisions (e.g. `main..HEAD` or `main...feature`).
pub fn diff_range(base: &str, head: &str) -> Result<Vec<FileDiff>> {
    let repo = discover_repo()?;

    let base_obj = repo
        .revparse_single(base)
        .with_context(|| format!("Cannot resolve base revision '{base}'"))?;
    let head_obj = repo
        .revparse_single(head)
        .with_context(|| format!("Cannot resolve head revision '{head}'"))?;

    let base_tree = base_obj
        .peel_to_tree()
        .with_context(|| format!("'{base}' does not point to a tree"))?;
    let head_tree = head_obj
        .peel_to_tree()
        .with_context(|| format!("'{head}' does not point to a tree"))?;

    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None)
        .context("Failed to compute range diff")?;

    extract_file_diffs(&repo, &diff)
}

/// Classify a rev spec string into a diff mode.
#[derive(Debug, PartialEq, Eq)]
pub enum DiffMode<'a> {
    /// No spec — diff staged changes
    Staged,
    /// `base..head` or `base...head` — range diff
    Range(&'a str, &'a str),
    /// Single revision — commit diff
    Commit(&'a str),
}

/// Parse a rev spec string into a `DiffMode`.
pub fn parse_diff_spec(spec: Option<&str>) -> DiffMode<'_> {
    match spec {
        None => DiffMode::Staged,
        Some(s) => {
            if let Some((base, head)) = s.split_once("...") {
                DiffMode::Range(base, head)
            } else if let Some((base, head)) = s.split_once("..") {
                DiffMode::Range(base, head)
            } else {
                DiffMode::Commit(s)
            }
        }
    }
}

/// Resolve a user-provided rev spec into file diffs.
///
/// Handles:
///   - `main..HEAD` or `main...HEAD` → range diff
///   - `HEAD~1` / `abc123` → single commit diff
///   - None → staged diff
pub fn diff_from_spec(spec: Option<&str>) -> Result<Vec<FileDiff>> {
    match parse_diff_spec(spec) {
        DiffMode::Staged => diff_staged(),
        DiffMode::Range(base, head) => diff_range(base, head),
        DiffMode::Commit(rev) => diff_commit(rev),
    }
}

/// Read blob content from a git diff delta, returning None for binary/missing.
fn read_blob(repo: &Repository, oid: git2::Oid) -> Option<String> {
    if oid.is_zero() {
        return None;
    }
    let blob = repo.find_blob(oid).ok()?;
    std::str::from_utf8(blob.content()).ok().map(String::from)
}

/// Extract `Vec<FileDiff>` from a `git2::Diff`.
fn extract_file_diffs(repo: &Repository, diff: &git2::Diff<'_>) -> Result<Vec<FileDiff>> {
    let mut results = Vec::new();

    for delta_idx in 0..diff.deltas().len() {
        let delta = diff.get_delta(delta_idx).unwrap();

        let status = match delta.status() {
            Delta::Added => DiffStatus::Added,
            Delta::Deleted => DiffStatus::Deleted,
            Delta::Modified => DiffStatus::Modified,
            Delta::Renamed => DiffStatus::Renamed,
            Delta::Copied => DiffStatus::Added,
            _ => continue,
        };

        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let language = Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Language::from_extension);

        let before = read_blob(repo, delta.old_file().id());
        let after = read_blob(repo, delta.new_file().id());

        // For index diffs, the new file's blob might be zero if it's from
        // the workdir. In that case, read from disk.
        let after = after.or_else(|| {
            if status == DiffStatus::Added || status == DiffStatus::Modified {
                repo.workdir()
                    .map(|wd| wd.join(&path))
                    .and_then(|p| std::fs::read_to_string(p).ok())
            } else {
                None
            }
        });

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ssh_url() {
        assert_eq!(
            normalize_remote_url("git@github.com:datagrout/invariant.git"),
            Some("datagrout/invariant".to_string())
        );
    }

    #[test]
    fn normalize_https_url() {
        assert_eq!(
            normalize_remote_url("https://github.com/datagrout/invariant.git"),
            Some("datagrout/invariant".to_string())
        );
    }

    #[test]
    fn normalize_https_no_dotgit() {
        assert_eq!(
            normalize_remote_url("https://github.com/owner/repo"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn normalize_ssh_scheme_url() {
        assert_eq!(
            normalize_remote_url("ssh://git@github.com/owner/repo.git"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn normalize_garbage_returns_none() {
        assert_eq!(normalize_remote_url("not-a-url"), None);
    }

    #[test]
    fn normalize_empty_string_returns_none() {
        assert_eq!(normalize_remote_url(""), None);
    }

    #[test]
    fn normalize_ssh_trailing_slash() {
        assert_eq!(
            normalize_remote_url("git@github.com:owner/repo.git/"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn normalize_https_deep_path() {
        assert_eq!(
            normalize_remote_url("https://gitlab.com/group/subgroup/repo.git"),
            Some("group/subgroup/repo".to_string())
        );
    }

    // ── DiffMode dispatch ──────────────────────────────────────────────

    #[test]
    fn parse_spec_none_is_staged() {
        assert_eq!(parse_diff_spec(None), DiffMode::Staged);
    }

    #[test]
    fn parse_spec_single_rev() {
        assert_eq!(parse_diff_spec(Some("HEAD~1")), DiffMode::Commit("HEAD~1"));
        assert_eq!(parse_diff_spec(Some("abc123")), DiffMode::Commit("abc123"));
    }

    #[test]
    fn parse_spec_two_dot_range() {
        assert_eq!(
            parse_diff_spec(Some("main..HEAD")),
            DiffMode::Range("main", "HEAD")
        );
    }

    #[test]
    fn parse_spec_three_dot_range() {
        assert_eq!(
            parse_diff_spec(Some("main...feature")),
            DiffMode::Range("main", "feature")
        );
    }

    #[test]
    fn parse_spec_three_dot_takes_precedence() {
        // "a...b..c" — the first split_once on "..." wins
        assert_eq!(
            parse_diff_spec(Some("a...b..c")),
            DiffMode::Range("a", "b..c")
        );
    }
}
