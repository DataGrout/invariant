//! What a namespace already holds, so `lens` uploads only what changed.
//!
//! `.invariant/lens_state.json` maps each lensed file (project-relative) to
//! the content hash its facts were built from. A re-lens skips files whose
//! hash still matches, replaces the facts of files that changed, and retracts
//! the facts of files that are gone, each by its per-file tag.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::facts::normalize_id;

const STATE_FILE: &str = "lens_state.json";

/// The persisted record for one namespace.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LensState {
    /// The namespace these hashes describe; state for another namespace is
    /// ignored, since its facts are not there.
    pub namespace: String,
    /// Project-relative path → content hash of the lensed source.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

impl LensState {
    /// Load the state for `namespace`, or an empty one when none is recorded
    /// or it describes a different namespace.
    pub fn load(invariant_dir: &Path, namespace: &str) -> Self {
        let fresh = Self {
            namespace: namespace.to_string(),
            files: BTreeMap::new(),
        };

        std::fs::read_to_string(invariant_dir.join(STATE_FILE))
            .ok()
            .and_then(|s| serde_json::from_str::<Self>(&s).ok())
            .filter(|state| state.namespace == namespace)
            .unwrap_or(fresh)
    }

    /// The namespace this repository was last lensed into, if it has been.
    /// Queries read the facts from there rather than from the older
    /// per-commit store, which `lens` no longer writes.
    pub fn lensed_namespace(invariant_dir: &Path) -> Option<String> {
        std::fs::read_to_string(invariant_dir.join(STATE_FILE))
            .ok()
            .and_then(|s| serde_json::from_str::<Self>(&s).ok())
            .filter(|state| !state.files.is_empty())
            .map(|state| state.namespace)
    }

    /// Write the state next to the config.
    pub fn save(&self, invariant_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(invariant_dir)?;
        std::fs::write(
            invariant_dir.join(STATE_FILE),
            serde_json::to_string_pretty(self)?,
        )?;
        Ok(())
    }

    /// Whether `path` was lensed from exactly this content.
    pub fn unchanged(&self, path: &str, hash: &str) -> bool {
        self.files.get(path).map(String::as_str) == Some(hash)
    }

    /// Recorded files not in `present`: their facts should be retracted.
    pub fn vanished<'a>(&'a self, present: &'a [String]) -> impl Iterator<Item = &'a String> {
        self.files.keys().filter(move |p| !present.contains(p))
    }
}

/// The default logic-cell namespace for a repository's code facts:
/// `code_<repo>` (`acme/billing` → `code_acme_billing`).
pub fn code_namespace(repo_id: &str) -> String {
    format!("code_{}", normalize_id(repo_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lensed_namespace_is_the_one_last_lensed() {
        let dir = std::env::temp_dir().join(format!("invariant_lensed_ns_{}", std::process::id()));
        assert_eq!(LensState::lensed_namespace(&dir), None);

        // A state with no files recorded has lensed nothing yet.
        LensState::load(&dir, "code_a").save(&dir).unwrap();
        assert_eq!(LensState::lensed_namespace(&dir), None);

        let mut state = LensState::load(&dir, "code_custom");
        state.files.insert("src/a.rs".into(), "h1".into());
        state.save(&dir).unwrap();
        assert_eq!(
            LensState::lensed_namespace(&dir).as_deref(),
            Some("code_custom")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn state_round_trips_and_ignores_other_namespaces() {
        let dir = std::env::temp_dir().join(format!("invariant_lens_state_{}", std::process::id()));
        let mut state = LensState::load(&dir, "code_a");
        assert!(state.files.is_empty());

        state.files.insert("src/a.rs".into(), "h1".into());
        state.save(&dir).unwrap();

        let back = LensState::load(&dir, "code_a");
        assert!(back.unchanged("src/a.rs", "h1"));
        assert!(!back.unchanged("src/a.rs", "h2"));
        assert!(LensState::load(&dir, "code_b").files.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vanished_lists_recorded_files_no_longer_present() {
        let mut state = LensState::default();
        state.files.insert("a.py".into(), "1".into());
        state.files.insert("b.py".into(), "2".into());
        let present = vec!["b.py".to_string()];
        assert_eq!(state.vanished(&present).collect::<Vec<_>>(), vec!["a.py"]);
    }

    #[test]
    fn namespace_from_repo_id() {
        assert_eq!(code_namespace("acme/billing"), "code_acme_billing");
    }
}
