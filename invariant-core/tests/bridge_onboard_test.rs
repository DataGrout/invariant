//! Integration tests for Bridge::onboard.
//!
//! These tests require a live DataGrout gateway and are skipped unless the
//! DG_GATEWAY_URL environment variable is set. Run with:
//!
//!   DG_GATEWAY_URL=https://app.datagrout.ai cargo test --test bridge_onboard_test -- --include-ignored

use invariant_core::bridge::Bridge;

/// Helper: read env var or skip the test.
macro_rules! require_env {
    ($var:expr) => {
        match std::env::var($var) {
            Ok(val) if !val.is_empty() => val,
            _ => {
                eprintln!("skipping: {} not set", $var);
                return;
            }
        }
    };
}

/// Unique agent name so parallel test runs don't collide.
fn unique_agent_name(suffix: &str) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("invariant-test-{}-{}", suffix, ts)
}

// ---------------------------------------------------------------------------
// onboard — happy path
// ---------------------------------------------------------------------------

/// Full onramp + mTLS bootstrap returns a non-empty MCP URL and a connected bridge.
#[tokio::test]
#[ignore = "requires DG_GATEWAY_URL and live network"]
async fn test_onboard_returns_mcp_url() {
    let gateway = require_env!("DG_GATEWAY_URL");

    let opts = datagrout_conduit::OnrampOptions {
        gateway: gateway.clone(),
        agent_name: unique_agent_name("onboard"),
        agent_type: Some("invariant-test".into()),
        intended_use: Some("Integration test — Bridge::onboard".into()),
        access_code: None,
    };

    let result = Bridge::onboard(opts).await;
    assert!(result.is_ok(), "Bridge::onboard failed: {:?}", result.err());

    let (_bridge, url) = result.unwrap();
    assert!(!url.is_empty(), "onboard returned an empty MCP URL");
    assert!(
        url.contains("datagrout") || url.contains("mcp"),
        "MCP URL looks wrong: {url}"
    );
}

/// The returned URL is a valid MCP endpoint (contains /mcp or /rpc suffix).
#[tokio::test]
#[ignore = "requires DG_GATEWAY_URL and live network"]
async fn test_onboard_url_is_valid_mcp_endpoint() {
    let gateway = require_env!("DG_GATEWAY_URL");

    let (_bridge, url) = Bridge::onboard(datagrout_conduit::OnrampOptions {
        gateway,
        agent_name: unique_agent_name("url-check"),
        agent_type: None,
        intended_use: None,
        access_code: None,
    })
    .await
    .expect("onboard failed");

    // URL must be parseable and use https.
    assert!(url.starts_with("https://"), "expected https URL, got: {url}");

    // Must look like an MCP server URL.
    assert!(
        url.contains("/servers/") || url.contains("/mcp"),
        "URL does not look like an MCP server path: {url}"
    );
}

/// After onboard, has_identity() returns true — the mTLS cert was persisted.
#[tokio::test]
#[ignore = "requires DG_GATEWAY_URL and live network"]
async fn test_onboard_persists_mtls_identity() {
    let gateway = require_env!("DG_GATEWAY_URL");

    Bridge::onboard(datagrout_conduit::OnrampOptions {
        gateway,
        agent_name: unique_agent_name("identity"),
        agent_type: None,
        intended_use: None,
        access_code: None,
    })
    .await
    .expect("onboard failed");

    assert!(
        Bridge::has_identity(),
        "has_identity() returned false after onboard — cert was not persisted to ~/.conduit/"
    );
}

/// Two sequential onboard calls reuse the existing identity (fast path) and
/// return the same server URL without creating a duplicate account.
#[tokio::test]
#[ignore = "requires DG_GATEWAY_URL and live network"]
async fn test_onboard_is_idempotent() {
    let gateway = require_env!("DG_GATEWAY_URL");

    let name = unique_agent_name("idempotent");

    let opts = || datagrout_conduit::OnrampOptions {
        gateway: gateway.clone(),
        agent_name: name.clone(),
        agent_type: None,
        intended_use: None,
        access_code: None,
    };

    let (_, url1) = Bridge::onboard(opts()).await.expect("first onboard failed");
    let (_, url2) = Bridge::onboard(opts()).await.expect("second onboard failed");

    assert_eq!(url1, url2, "repeated onboard returned different URLs");
}

/// onboard with an invalid gateway URL returns a clear error rather than hanging.
#[tokio::test]
#[ignore = "requires live network (checks DNS rejection)"]
async fn test_onboard_invalid_gateway_errors_cleanly() {
    let result = Bridge::onboard(datagrout_conduit::OnrampOptions {
        gateway: "https://does-not-exist.invalid".into(),
        agent_name: "test-agent".into(),
        agent_type: None,
        intended_use: None,
        access_code: None,
    })
    .await;

    assert!(
        result.is_err(),
        "expected error for invalid gateway, got success"
    );

    let msg = format!("{}", result.unwrap_err());
    assert!(
        !msg.is_empty(),
        "error message should explain what went wrong"
    );
}
