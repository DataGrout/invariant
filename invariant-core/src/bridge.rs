//! Conduit Bridge — connects Invariant to DataGrout Prism tools.
//!
//! When a DataGrout gateway URL is configured, the bridge delegates to
//! server-side Prism tools for intent-enriched code analysis, advanced
//! Prolog queries, and diff analysis. Falls back gracefully when offline.
//!
//! Authentication is handled automatically by the Conduit SDK:
//! - On first run, call [`Bridge::bootstrap`] with a token to generate and
//!   register an mTLS identity (saved to `~/.conduit/`).
//! - On subsequent runs, [`Bridge::connect`] auto-discovers the persisted
//!   identity — no token or API key needed.

use anyhow::{Context, Result};
use datagrout_conduit::{Client, ClientBuilder, Transport};
use serde::{Deserialize, Serialize};
use serde_json::json;

const TOOL_CODE_LENS: &str = "data-grout@1/invariant.code_lens@1";
const TOOL_CODE_QUERY: &str = "data-grout@1/invariant.code_query@1";
const TOOL_DIFF_ANALYZER: &str = "data-grout@1/invariant.diff_analyzer@1";

/// Connection to DataGrout via the Conduit SDK.
pub struct Bridge {
    client: Client,
}

/// Diff analysis result returned by `invariant.diff_analyzer`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffAnalysis {
    /// Structured breakdown of what changed (functions added/deleted/modified, deps, calls).
    pub changes_detected: serde_json::Value,
    /// Issues found with severity, type, message, and suggestion.
    #[serde(default)]
    pub concerns: Vec<serde_json::Value>,
    /// 0.0–1.0 score of how well changes match the stated goal.
    pub alignment_score: f64,
    /// Explanation of the alignment score.
    #[serde(default)]
    pub alignment_reasoning: Option<String>,
    /// Changes not expected given the stated goal.
    #[serde(default)]
    pub unexpected_changes: Vec<String>,
    /// Optional receipt for cost tracking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<serde_json::Value>,
}

/// Extract the inner JSON value from an MCP content wrapper.
/// The Conduit SDK returns `{"type": "text", "text": "...json..."}` for tool results.
fn unwrap_mcp_content(value: serde_json::Value) -> serde_json::Value {
    if let Some(text) = value.get("text").and_then(|t| t.as_str()) {
        serde_json::from_str(text).unwrap_or(value)
    } else {
        value
    }
}

impl Bridge {
    async fn from_client(client: Client, url: &str) -> Result<Self> {
        client.connect().await.context("Failed to connect to DataGrout")?;
        tracing::info!("Connected to DataGrout at {}", url);
        Ok(Self { client })
    }

    /// Connect to DataGrout using auto-discovered mTLS identity.
    ///
    /// After a successful [`bootstrap`], this is all that's needed — the
    /// Conduit SDK finds the persisted identity in `~/.conduit/` automatically.
    pub async fn connect(url: &str) -> Result<Self> {
        let client = ClientBuilder::new()
            .url(url)
            .transport(Transport::Mcp)
            .with_identity_auto()
            .build()?;

        Self::from_client(client, url).await
    }

    /// Connect with an explicit bearer token (fallback when no mTLS identity exists).
    pub async fn connect_with_token(url: &str, token: &str) -> Result<Self> {
        let client = ClientBuilder::new()
            .url(url)
            .transport(Transport::Mcp)
            .auth_bearer(token)
            .with_identity_auto()
            .build()?;

        Self::from_client(client, url).await
    }

    /// Bootstrap a new mTLS identity and connect.
    ///
    /// Generates an ECDSA keypair, registers it with the DataGrout CA using the
    /// provided token, and persists the signed certificate to `~/.conduit/`.
    /// Subsequent calls to [`connect`] will auto-discover it.
    pub async fn bootstrap(url: &str, token: &str, name: &str) -> Result<Self> {
        let client = ClientBuilder::new()
            .url(url)
            .transport(Transport::Mcp)
            .bootstrap_identity(token, name)
            .await
            .context("Identity bootstrap failed")?
            .build()?;

        Self::from_client(client, url).await
    }

    /// Check whether an mTLS identity already exists on disk.
    pub fn has_identity() -> bool {
        datagrout_conduit::ConduitIdentity::try_default().is_some()
    }

    /// Upload locally-extracted facts to `invariant.code_lens` for server-side
    /// persistence and optional LLM intent enrichment.
    pub async fn upload_facts(
        &self,
        code: &str,
        language: &str,
        filepath: &str,
        commit_sha: &str,
        repo_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut args = json!({
            "code": code,
            "language": language,
            "filepath": filepath,
            "commit_sha": commit_sha,
        });

        if let Some(id) = repo_id {
            args["repo_id"] = json!(id);
        }

        let result = self
            .client
            .call_tool(TOOL_CODE_LENS, args)
            .await
            .context("invariant.code_lens call failed")?;

        Ok(unwrap_mcp_content(result))
    }

    /// Run a server-side Prolog query via `invariant.code_query`.
    /// Gives access to advanced queries (intent_mismatches, test_gaps,
    /// security_concerns, hotspots) that require LLM-enriched facts.
    pub async fn query(
        &self,
        repo_id: &str,
        query: &str,
        commit_sha: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut args = json!({
            "repo_id": repo_id,
            "query": query,
        });

        if let Some(sha) = commit_sha {
            args["commit_sha"] = json!(sha);
        }

        let result = self
            .client
            .call_tool(TOOL_CODE_QUERY, args)
            .await
            .context("invariant.code_query call failed")?;

        Ok(unwrap_mcp_content(result))
    }

    /// Analyze code changes for goal alignment via `invariant.diff_analyzer`.
    /// No local fallback — requires LLM on the server.
    pub async fn diff_analyze(
        &self,
        before: &str,
        after: &str,
        goal: &str,
        language: Option<&str>,
        context: Option<&str>,
    ) -> Result<DiffAnalysis> {
        let mut args = json!({
            "before": before,
            "after": after,
            "goal": goal,
        });

        if let Some(lang) = language {
            args["language"] = json!(lang);
        }
        if let Some(ctx) = context {
            args["context"] = json!(ctx);
        }

        let result = self
            .client
            .call_tool(TOOL_DIFF_ANALYZER, args)
            .await
            .context("invariant.diff_analyzer call failed")?;

        let analysis: DiffAnalysis = serde_json::from_value(unwrap_mcp_content(result))?;
        Ok(analysis)
    }
}
