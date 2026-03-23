//! Configuration for Invariant

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const CONFIG_DIR: &str = ".invariant";
const CONFIG_FILE: &str = "config.toml";

/// Invariant configuration persisted to `.invariant/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// DataGrout gateway URL
    pub datagrout_url: Option<String>,

    /// Bearer token fallback used when mTLS bootstrap is unavailable.
    #[serde(default)]
    pub access_token: Option<String>,

    /// Repository ID (derived from git remote or directory name)
    pub repo_id: Option<String>,

    /// Patterns to ignore during file walks
    #[serde(default = "default_ignore_patterns")]
    pub ignore_patterns: Vec<String>,
}

fn default_ignore_patterns() -> Vec<String> {
    vec![
        "node_modules".to_string(),
        ".git".to_string(),
        "target".to_string(),
        "dist".to_string(),
        "__pycache__".to_string(),
        ".venv".to_string(),
        "vendor".to_string(),
    ]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            datagrout_url: None,
            access_token: None,
            repo_id: None,
            ignore_patterns: default_ignore_patterns(),
        }
    }
}

impl Config {
    /// Load config from `.invariant/config.toml` in the given directory,
    /// falling back to defaults if the file doesn't exist.
    ///
    /// Prints a warning to stderr when a config file exists but cannot be
    /// read or parsed, so users are aware of misconfiguration.
    pub fn load(project_root: &Path) -> Self {
        let config_path = project_root.join(CONFIG_DIR).join(CONFIG_FILE);
        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(contents) => match toml::from_str(&contents) {
                    Ok(config) => return config,
                    Err(e) => {
                        eprintln!("warning: failed to parse {}: {}", config_path.display(), e);
                    }
                },
                Err(e) => {
                    eprintln!("warning: failed to read {}: {}", config_path.display(), e);
                }
            }
        }
        Self::default()
    }

    /// Save config to `.invariant/config.toml` in the given directory.
    pub fn save(&self, project_root: &Path) -> Result<PathBuf> {
        let config_dir = project_root.join(CONFIG_DIR);
        std::fs::create_dir_all(&config_dir)?;

        let config_path = config_dir.join(CONFIG_FILE);
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(&config_path, contents)?;

        Ok(config_path)
    }

    /// Resolve the DataGrout URL from config, env var, or CLI flag (in priority order).
    pub fn resolve_url(&self, cli_url: Option<&str>) -> Option<String> {
        cli_url
            .map(String::from)
            .or_else(|| std::env::var("DATAGROUT_URL").ok())
            .or_else(|| self.datagrout_url.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn config_round_trips_access_token() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("invariant-config-test-{suffix}"));

        let config = Config {
            datagrout_url: Some("https://gateway.datagrout.ai/servers/test/mcp".to_string()),
            access_token: Some("secret-token".to_string()),
            repo_id: Some("sample-repo".to_string()),
            ignore_patterns: vec!["target".to_string()],
        };

        let path = config.save(&temp_dir).unwrap();
        let loaded = Config::load(&temp_dir);

        assert_eq!(loaded.datagrout_url, config.datagrout_url);
        assert_eq!(loaded.access_token, config.access_token);
        assert_eq!(loaded.repo_id, config.repo_id);
        assert_eq!(loaded.ignore_patterns, config.ignore_patterns);

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(temp_dir.join(".invariant")).unwrap();
        std::fs::remove_dir(temp_dir).unwrap();
    }
}
