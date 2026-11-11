//! Conduit Bridge — connects Invariant to DataGrout Invariant server-side tools.
//!
//! When a DataGrout gateway URL is configured, the bridge delegates to
//! server-side Invariant tools for intent-enriched code analysis, advanced
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
const TOOL_LOGIC_ASSERT: &str = "data-grout@1/logic.assert@1";
const TOOL_LOGIC_FORGET: &str = "data-grout@1/logic.forget@1";
const TOOL_FRAME_SLICE: &str = "data-grout@1/frame.slice@1";
/// Under DataGrout's per-result row cap, so a page is never itself headed.
const SLICE_PAGE: usize = 40;

/// Clauses per `logic.assert` call: batched, and well under its payload cap.
pub const ASSERT_BATCH: usize = 200;

/// The per-file tag lens facts are asserted under, so one file's facts can be
/// replaced or retracted without touching the rest. Same convention as
/// Manifold's lens (`src/store.rs` → `lens_src_store_rs`), so either client
/// can refresh a namespace the other built.
pub fn lens_tag(path: &str) -> String {
    let mut tag = String::with_capacity(path.len() + 5);
    tag.push_str("lens_");
    tag.extend(
        path.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }),
    );
    tag
}

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
        client
            .connect()
            .await
            .context("Failed to connect to DataGrout")?;
        tracing::info!("Connected to DataGrout at {}", url);
        Ok(Self { client })
    }

    /// Connect to DataGrout using auto-discovered mTLS identity.
    ///
    /// After a successful [`Bridge::bootstrap`], this is all that's needed — the
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
    /// Subsequent calls to [`Bridge::connect`] will auto-discover it.
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

    /// Full autonomous onboarding: onramp registration + mTLS identity bootstrap.
    ///
    /// Does not require an existing DataGrout URL or account. Returns the connected
    /// bridge along with the MCP server URL provisioned for this agent. Saves the
    /// mTLS identity to `~/.conduit/` so subsequent `Bridge::connect` calls are
    /// token-free.
    pub async fn onboard(opts: datagrout_conduit::OnrampOptions) -> Result<(Self, String)> {
        let (creds, token) = datagrout_conduit::register_and_exchange(&opts)
            .await
            .context("Onramp registration failed")?;

        let url = creds
            .mcp_url
            .ok_or_else(|| anyhow::anyhow!("DataGrout did not provision an MCP server URL"))?;

        let bridge = Self::bootstrap(&url, &token, &opts.agent_name).await?;
        Ok((bridge, url))
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

    /// Assert lens facts (Prolog clause strings) into a logic-cell namespace
    /// under `tag`. Deterministic structure only: no LLM, no embeddings.
    /// Returns how many clauses the server accepted.
    pub async fn assert_clauses(
        &self,
        namespace: &str,
        tag: &str,
        clauses: &[String],
    ) -> Result<usize> {
        let mut accepted = 0;

        for chunk in clauses.chunks(ASSERT_BATCH) {
            let body: Vec<&str> = chunk
                .iter()
                .map(|c| c.strip_suffix('.').unwrap_or(c))
                .collect();
            let args = json!({
                "prolog": body,
                "tag": tag,
                "namespace": namespace,
                "skip_embedding": true,
            });

            let result = self
                .client
                .call_tool(TOOL_LOGIC_ASSERT, args)
                .await
                .context("logic.assert call failed")?;

            let result = unwrap_mcp_content(result);
            accepted += result
                .get("asserted_count")
                .or_else(|| result.get("count"))
                .and_then(|v| v.as_u64())
                .unwrap_or(chunk.len() as u64) as usize;
        }

        Ok(accepted)
    }

    /// Retract every fact asserted under `tag` in `namespace`.
    pub async fn forget_tag(&self, namespace: &str, tag: &str) -> Result<()> {
        self.client
            .call_tool(
                TOOL_LOGIC_FORGET,
                json!({"tag": tag, "namespace": namespace}),
            )
            .await
            .context("logic.forget call failed")?;
        Ok(())
    }

    /// Run a server-side Prolog query via `invariant.code_query`.
    /// Gives access to advanced queries (intent_mismatches, test_gaps,
    /// security_concerns, hotspots) that require LLM-enriched facts.
    ///
    /// With `namespace`, the query runs over the facts `lens` put in that
    /// logic-cell namespace; without it, over the older per-commit store.
    pub async fn query(
        &self,
        repo_id: &str,
        query: &str,
        commit_sha: Option<&str>,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.query_filtered(repo_id, query, commit_sha, namespace, None)
            .await
    }

    /// As [`Bridge::query`], keeping only the rows that mention `filter`
    /// (case-insensitive): a table for `table_access`, a package for
    /// `dependencies`, an annotation or a path for `routes`.
    pub async fn query_filtered(
        &self,
        repo_id: &str,
        query: &str,
        commit_sha: Option<&str>,
        namespace: Option<&str>,
        filter: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut args = json!({
            "repo_id": repo_id,
            "query": query,
        });

        if let Some(f) = filter {
            args["filter"] = json!(f);
        }

        if let Some(sha) = commit_sha {
            args["commit_sha"] = json!(sha);
        }

        if let Some(ns) = namespace {
            args["namespace"] = json!(ns);
        }

        let result = self
            .client
            .call_tool(TOOL_CODE_QUERY, args)
            .await
            .context("invariant.code_query call failed")?;

        self.unhead(unwrap_mcp_content(result)).await
    }

    /// DataGrout caps a tool result at a few dozen rows and returns the rest
    /// as a preview plus a `cache_ref`: a safety valve for agents, whose
    /// context it protects. The CLI wants every row, so a headed result is
    /// read back in pages with `frame.slice` and put back under `results`.
    async fn unhead(&self, result: serde_json::Value) -> Result<serde_json::Value> {
        let headed = result.get("_headed").and_then(|h| h.as_bool()) == Some(true);
        let cache_ref = result.get("cache_ref").and_then(|c| c.as_str());

        let (Some(cache_ref), true) = (cache_ref, headed) else {
            return Ok(result);
        };

        let total = result
            .get("record_count")
            .or_else(|| result.get("count"))
            .and_then(|c| c.as_u64())
            .unwrap_or(0) as usize;

        let mut rows: Vec<serde_json::Value> = Vec::with_capacity(total);

        while rows.len() < total {
            let page = self
                .client
                .call_tool(
                    TOOL_FRAME_SLICE,
                    json!({"payload": cache_ref, "offset": rows.len(), "limit": SLICE_PAGE}),
                )
                .await
                .context("frame.slice call failed")?;

            let page = unwrap_mcp_content(page);
            let records = page
                .get("records")
                .or_else(|| page.get("result").and_then(|r| r.get("records")))
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();

            if records.is_empty() {
                break;
            }
            rows.extend(records);
        }

        let mut full = result;
        if let Some(obj) = full.as_object_mut() {
            obj.insert("count".into(), json!(rows.len()));
            obj.insert("results".into(), serde_json::Value::Array(rows));
            for key in [
                "_headed",
                "_hint",
                "_answer_from",
                "preview",
                "next_tools",
                "schema",
            ] {
                obj.remove(key);
            }
        }
        Ok(full)
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
