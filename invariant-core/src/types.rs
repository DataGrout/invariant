//! Core type definitions for Invariant

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Result type alias
pub type Result<T> = std::result::Result<T, crate::Error>;

/// Invariant error types
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Parse error
    #[error("Parse error: {0}")]
    Parse(String),

    /// Query error (server-side query failed or unavailable)
    #[error("Query error: {0}")]
    Query(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Other error
    #[error("{0}")]
    Other(String),
}

/// Analysis result containing extracted facts and summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    /// List of Prolog fact strings
    pub facts: Vec<String>,

    /// Summary statistics
    pub summary: AnalysisSummary,

    /// Optional receipt (if integrated with DataGrout)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
}

/// Summary statistics from analysis
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnalysisSummary {
    /// Number of modules found
    pub modules: usize,

    /// Number of functions found
    pub functions: usize,

    /// Number of function calls found
    pub calls: usize,

    /// Number of dependencies found
    pub dependencies: usize,

    /// Lines of code
    pub loc: usize,

    /// Language-specific metrics
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metrics: HashMap<String, serde_json::Value>,
}

/// Receipt for cost tracking (DataGrout integration)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    /// Receipt ID
    pub id: String,

    /// Total cost in credits
    pub total_cost: u64,

    /// Tool calls made
    pub tool_calls: Vec<ToolCall>,

    /// Timestamp
    pub timestamp: String,
}

/// Tool call record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Tool name
    pub name: String,

    /// Cost in credits
    pub cost: u64,

    /// Duration in milliseconds
    pub duration_ms: u64,
}

/// Query result returned from server-side Prism queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// Query that was executed
    pub query: String,

    /// List of results
    pub results: Vec<serde_json::Value>,

    /// Number of results
    pub count: usize,

    /// Repository ID
    pub repo_id: String,

    /// Commit SHA
    pub commit_sha: String,

    /// Optional error message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

