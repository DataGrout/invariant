//! Invariant CLI - Semantic code analysis for the AI era

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use datagrout_conduit::OnrampOptions;
use ignore::WalkBuilder;
use indicatif::{ProgressBar, ProgressStyle};
use invariant_core::{
    bridge::{Bridge, DiffAnalysis},
    git::{self, DiffStatus, FileDiff},
    patch, Analyzer, Config, Language,
};
use std::io::Read as _;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "invariant")]
#[command(about = "Invariant — Semantic code analysis for the AI era", long_about = None)]
struct Cli {
    /// Enable verbose (debug-level) logging
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize Invariant for the current repository
    Init {
        /// DataGrout gateway URL
        #[arg(long)]
        url: Option<String>,

        /// API key or token for first-time identity bootstrap
        #[arg(long)]
        token: Option<String>,
    },

    /// Analyze code files into structured facts
    Lens {
        /// Paths to analyze (files or directories, defaults to current dir)
        paths: Vec<PathBuf>,

        /// Language filter (e.g. "rust", "python", "elixir")
        #[arg(long)]
        language: Option<String>,

        /// Skip uploading to DataGrout (local analysis only)
        #[arg(long)]
        local_only: bool,
    },

    /// Execute queries over analyzed facts via DataGrout Invariant
    Query {
        /// Query name (e.g. "orphans", "test_gaps", "intent_mismatches")
        query: String,

        /// Commit SHA to query (defaults to HEAD)
        #[arg(long)]
        commit: Option<String>,

        /// Output format
        #[arg(long, default_value = "text")]
        output: OutputFormat,
    },

    /// Analyze code changes against a stated goal
    ///
    /// Supports multiple input modes:
    ///   invariant diff --goal "..."                    # staged changes vs HEAD
    ///   invariant diff HEAD~1 --goal "..."             # specific commit
    ///   invariant diff main..HEAD --goal "..."         # branch diff
    ///   invariant diff --patch file.patch --goal "..." # from patch file
    ///   git diff | invariant diff --stdin --goal "..." # from stdin
    ///   invariant diff --before a.py --after b.py --goal "..." # legacy file mode
    Diff {
        /// Git revision spec: commit SHA, range (main..HEAD), or ref
        #[arg(conflicts_with_all = ["before", "after", "patch", "stdin"])]
        rev: Option<String>,

        /// Read unified diff from a patch file
        #[arg(long, conflicts_with_all = ["rev", "before", "after", "stdin"])]
        patch: Option<PathBuf>,

        /// Read unified diff from stdin (pipe git diff output)
        #[arg(long, conflicts_with_all = ["rev", "before", "after", "patch"])]
        stdin: bool,

        /// Path to the original file (legacy file mode)
        #[arg(long, requires = "after", conflicts_with_all = ["rev", "patch", "stdin"])]
        before: Option<PathBuf>,

        /// Path to the modified file (legacy file mode)
        #[arg(long, requires = "before")]
        after: Option<PathBuf>,

        /// Intent / goal the change should accomplish
        #[arg(long)]
        goal: String,

        /// Filter to specific file paths (for git/patch modes)
        #[arg(long, short)]
        files: Vec<String>,

        /// Programming language (auto-detected if omitted)
        #[arg(long)]
        language: Option<String>,

        /// Output format
        #[arg(long, default_value = "text")]
        output: OutputFormat,
    },

    /// Lens changed files, diff-analyze, and run queries in one shot
    ///
    /// Combines lens + diff + query into a single developer workflow:
    ///   invariant review --goal "..."                  # staged changes
    ///   invariant review HEAD~1 --goal "..."           # last commit
    ///   invariant review main..HEAD --goal "..."       # branch review
    Review {
        /// Git revision spec (defaults to staged changes)
        rev: Option<String>,

        /// Intent / goal the change should accomplish
        #[arg(long)]
        goal: String,

        /// Queries to run on lensed files (defaults to orphans + test_gaps)
        #[arg(long, short)]
        queries: Vec<String>,

        /// Output format
        #[arg(long, default_value = "text")]
        output: OutputFormat,
    },

    /// Show connection and configuration status
    Status,

    /// Create a DataGrout account and get your server URL — no prior account needed
    ///
    /// For humans:     invariant onboard
    /// For agents:     invariant onboard --agent --name my-agent
    Onboard {
        /// Agent or machine name used to identify this instance (defaults to hostname)
        #[arg(long)]
        name: Option<String>,

        /// Agent mode — skips interactive prompts, suitable for CI or automated pipelines
        #[arg(long)]
        agent: bool,
    },
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

/// Detect repo_id and commit SHA from the current git context.
///
/// Uses the git remote URL (normalized to `owner/repo`) for a stable
/// identifier. Falls back to workdir folder name if no remote is set.
fn detect_repo_context() -> (String, String) {
    match git::detect_repo_context() {
        Ok(ctx) => (ctx.repo_id, ctx.commit_sha),
        Err(_) => ("unknown".to_string(), "HEAD".to_string()),
    }
}

/// Find the project root (walks up to the nearest .git directory).
fn find_project_root() -> PathBuf {
    git2::Repository::discover(".")
        .ok()
        .and_then(|r| r.workdir().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Load config and connect to DataGrout (auto-discovers mTLS identity).
async fn require_bridge(config: &Config) -> Result<Bridge> {
    let url = config
        .resolve_url(None)
        .ok_or_else(|| anyhow::anyhow!("DataGrout URL not configured. Run `invariant init`."))?;

    match Bridge::connect(&url).await {
        Ok(bridge) => {
            eprintln!("  {} Connected to DataGrout", "✓".green());
            Ok(bridge)
        }
        Err(mtls_err) => {
            if let Some(token) = config.access_token.as_deref() {
                let bridge = Bridge::connect_with_token(&url, token).await.map_err(|token_err| {
                    anyhow::anyhow!(
                        "Failed to connect to DataGrout via mTLS ({mtls_err}) or bearer token ({token_err})"
                    )
                })?;
                eprintln!(
                    "  {} Connected to DataGrout with saved bearer token",
                    "✓".green()
                );
                Ok(bridge)
            } else {
                Err(mtls_err)
            }
        }
    }
}

/// Try to connect without failing (for optional upload in lens).
async fn try_bridge(config: &Config) -> Option<Bridge> {
    config.resolve_url(None)?;
    require_bridge(config).await.ok()
}

/// Walk a directory and collect files with recognized extensions,
/// respecting `.gitignore` and `.ignore` files automatically.
fn collect_files(path: &PathBuf) -> Vec<PathBuf> {
    WalkBuilder::new(path)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .and_then(Language::from_extension)
                .is_some()
        })
        .map(|e| e.into_path())
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = if cli.verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::WARN
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive(log_level.into()),
        )
        .init();

    let project_root = find_project_root();
    let config = Config::load(&project_root);

    match cli.command {
        Commands::Init { url, token } => cmd_init(&project_root, config, url, token).await?,
        Commands::Lens {
            paths,
            language,
            local_only,
        } => cmd_lens(&config, paths, language, local_only).await?,
        Commands::Query {
            query,
            commit,
            output,
        } => cmd_query(&config, query, commit, output).await?,
        Commands::Diff {
            rev,
            patch,
            stdin,
            before,
            after,
            goal,
            files,
            language,
            output,
        } => {
            cmd_diff(
                &config, rev, patch, stdin, before, after, goal, files, language, output,
            )
            .await?
        }
        Commands::Review {
            rev,
            goal,
            queries,
            output,
        } => cmd_review(&config, rev, goal, queries, output).await?,
        Commands::Status => cmd_status(&config),
        Commands::Onboard { name, agent } => {
            cmd_onboard(&project_root, config, name, agent).await?
        }
    }

    Ok(())
}

async fn cmd_init(
    project_root: &std::path::Path,
    mut config: Config,
    url: Option<String>,
    token: Option<String>,
) -> Result<()> {
    println!("{}", "Initializing Invariant...".blue().bold());

    let (repo_id, commit_sha) = detect_repo_context();

    println!("  {} {}", "Repository:".green(), repo_id);
    println!("  {} {}", "Path:".green(), project_root.display());
    println!(
        "  {} {}",
        "Commit:".green(),
        &commit_sha[..std::cmp::min(8, commit_sha.len())]
    );

    config.repo_id = Some(repo_id.clone());

    let gateway_url = url
        .or_else(|| std::env::var("DATAGROUT_URL").ok())
        .or_else(|| config.datagrout_url.clone());

    if let Some(ref url) = gateway_url {
        config.datagrout_url = Some(url.clone());
        println!("  {} {}", "Gateway:".green(), url);

        if Bridge::has_identity() {
            config.access_token = None;
            println!("  {} mTLS identity found", "✓".green());

            match Bridge::connect(url).await {
                Ok(_) => println!("  {} Connection verified", "✓".green()),
                Err(e) => println!("  {} Connection test failed: {}", "⚠".yellow(), e),
            }
        } else if let Some(token) = token {
            println!("  {} Bootstrapping mTLS identity...", "→".cyan());

            let machine_name = format!("invariant-{}", repo_id);
            match Bridge::bootstrap(url, &token, &machine_name).await {
                Ok(_) => {
                    config.access_token = None;
                    println!(
                        "  {} Identity created and saved to ~/.conduit/",
                        "✓".green()
                    );
                    println!(
                        "  {} Future runs will auto-authenticate (no token needed)",
                        "✓".green()
                    );
                }
                Err(e) => {
                    println!("  {} Bootstrap failed: {}", "✗".red(), e);

                    match Bridge::connect_with_token(url, &token).await {
                        Ok(_) => {
                            config.access_token = Some(token);
                            println!("  {} Connected with bearer token fallback", "✓".green());
                            println!(
                                "  {} Future runs will use the saved bearer token if mTLS is unavailable",
                                "✓".green()
                            );
                        }
                        Err(token_err) => {
                            config.access_token = None;
                            println!(
                                "  {} Bearer token validation also failed: {}",
                                "✗".red(),
                                token_err
                            );
                            println!(
                                "  {} You can retry with: invariant init --url {} --token <your-token>",
                                "hint:".yellow(),
                                url
                            );
                        }
                    }
                }
            }
        } else {
            println!(
                "  {} No mTLS identity found. Provide --token to bootstrap:",
                "⚠".yellow()
            );
            println!("    invariant init --url {} --token <your-api-token>", url);
        }
    } else {
        println!("\n  {} No DataGrout URL configured.", "⚠".yellow());

        // Offer to create an account if running interactively.
        use std::io::IsTerminal as _;
        if std::io::stdin().is_terminal() {
            print!(
                "  {} Create a free DataGrout account now? [Y/n] ",
                "→".cyan()
            );
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut answer = String::new();
            let _ = std::io::stdin().read_line(&mut answer);
            let answer = answer.trim().to_lowercase();

            if answer.is_empty() || answer == "y" || answer == "yes" {
                let hostname = std::env::var("HOSTNAME")
                    .or_else(|_| std::env::var("COMPUTERNAME"))
                    .unwrap_or_else(|_| "machine".to_string());
                let agent_name = format!("invariant-{}", hostname);

                println!("\n  {} Registering with DataGrout...", "→".cyan());

                match Bridge::onboard(OnrampOptions {
                    gateway: "https://app.datagrout.ai".to_string(),
                    agent_name: agent_name.clone(),
                    agent_type: Some(format!("invariant/{}", invariant_core::VERSION)),
                    intended_use: Some("Semantic code analysis via Invariant CLI".to_string()),
                    access_code: None,
                })
                .await
                {
                    Ok((_, server_url)) => {
                        config.datagrout_url = Some(server_url.clone());
                        println!("  {} Account created!", "✓".green());
                        println!("  {} {}", "Server URL:".green(), server_url);
                        println!(
                            "  {} mTLS identity saved to ~/.conduit/ — no tokens needed going forward",
                            "✓".green()
                        );
                    }
                    Err(e) => {
                        println!("  {} Registration failed: {}", "✗".red(), e);
                        println!(
                            "  {} You can try again later with: invariant onboard",
                            "hint:".yellow()
                        );
                        println!(
                            "  {} Or provide an existing URL: invariant init --url <your-url>",
                            "hint:".yellow()
                        );
                    }
                }
            } else {
                println!(
                    "  Skipped. Run {} to sign up later.",
                    "invariant onboard".bold()
                );
                println!(
                    "  Or: invariant init --url https://gateway.datagrout.ai/servers/{{uuid}}/mcp"
                );
            }
        } else {
            println!("  Run {} to create an account.", "invariant onboard".bold());
            println!(
                "  Or: invariant init --url https://gateway.datagrout.ai/servers/{{uuid}}/mcp"
            );
        }
    }

    let config_path = config.save(project_root)?;
    println!(
        "\n  {} Saved config to {}",
        "✓".green(),
        config_path.display()
    );

    println!("\n{}", "Ready.".green().bold());
    println!("  {} to analyze your code", "invariant lens".bold());
    println!("  {} to check for issues", "invariant query orphans".bold());

    Ok(())
}

async fn cmd_lens(
    config: &Config,
    paths: Vec<PathBuf>,
    language_filter: Option<String>,
    local_only: bool,
) -> Result<()> {
    let mut analyzer = Analyzer::new()?;
    let bridge = if local_only {
        None
    } else {
        try_bridge(config).await
    };

    let (repo_id, commit_sha) = detect_repo_context();

    let scan_paths = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths
    };

    let mut files_to_lens: Vec<PathBuf> = Vec::new();
    for path in &scan_paths {
        if path.is_file() {
            files_to_lens.push(path.clone());
        } else if path.is_dir() {
            files_to_lens.extend(collect_files(path));
        }
    }

    if let Some(ref filter) = language_filter {
        let filter_lower = filter.to_lowercase();
        files_to_lens.retain(|f| {
            f.extension()
                .and_then(|e| e.to_str())
                .and_then(Language::from_extension)
                .map(|l| l.name() == filter_lower)
                .unwrap_or(false)
        });
    }

    println!(
        "{} {} files...",
        "Analyzing".blue().bold(),
        files_to_lens.len()
    );
    println!("  {} {}", "Repo:".green(), repo_id);
    println!(
        "  {} {}",
        "Commit:".green(),
        &commit_sha[..std::cmp::min(8, commit_sha.len())]
    );

    let pb = ProgressBar::new(files_to_lens.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
            .unwrap()
            .progress_chars("##-"),
    );

    let mut total_functions = 0;
    let mut total_facts = 0;
    let mut uploaded = 0;
    let mut errors = 0;

    for file_path in &files_to_lens {
        pb.set_message(format!("{}", file_path.display()));

        if let Some(ext) = file_path.extension().and_then(|s| s.to_str()) {
            if let Some(language) = Language::from_extension(ext) {
                match std::fs::read_to_string(file_path) {
                    Ok(code) => {
                        match analyzer.lens_code(
                            &code,
                            language,
                            file_path.to_str().unwrap_or("unknown"),
                            &commit_sha,
                        ) {
                            Ok(result) => {
                                total_functions += result.summary.functions;
                                total_facts += result.facts.len();

                                if let Some(ref b) = bridge {
                                    if b.upload_facts(
                                        &code,
                                        language.name(),
                                        file_path.to_str().unwrap_or("unknown"),
                                        &commit_sha,
                                        Some(&repo_id),
                                    )
                                    .await
                                    .is_ok()
                                    {
                                        uploaded += 1;
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::debug!("Failed to analyze {}: {}", file_path.display(), e);
                                errors += 1;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Failed to read {}: {}", file_path.display(), e);
                        errors += 1;
                    }
                }
            }
        }

        pb.inc(1);
    }

    pb.finish_with_message("Done!");

    println!(
        "\n{} {} files",
        "Analyzed".green().bold(),
        files_to_lens.len()
    );
    println!("  Functions: {}", total_functions);
    println!("  Facts:     {}", total_facts);

    if uploaded > 0 {
        println!("  {} {} files uploaded to Invariant", "↑".cyan(), uploaded);
    } else if bridge.is_none() && !local_only {
        println!(
            "  {} Run `invariant init` to enable remote analysis",
            "hint:".yellow()
        );
    }

    if errors > 0 {
        println!(
            "  {} {} files skipped (use -v for details)",
            "⚠".yellow(),
            errors
        );
    }

    Ok(())
}

async fn cmd_query(
    config: &Config,
    query_name: String,
    commit: Option<String>,
    output: OutputFormat,
) -> Result<()> {
    let bridge = require_bridge(config).await?;
    let (repo_id, detected_sha) = detect_repo_context();
    let commit_sha = commit.unwrap_or(detected_sha);

    println!(
        "{} {} (repo: {}, commit: {}) ...",
        "Querying:".blue().bold(),
        query_name.bold(),
        repo_id,
        &commit_sha[..std::cmp::min(8, commit_sha.len())]
    );

    let result = bridge
        .query(&repo_id, &query_name, Some(&commit_sha))
        .await?;

    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
        OutputFormat::Text => render_query_result(&query_name, &result),
    }

    Ok(())
}

fn render_query_result(query_name: &str, result: &serde_json::Value) {
    if let Some(results) = result.get("results").and_then(|r| r.as_array()) {
        let count = results.len();
        println!(
            "\n{} {} result{}",
            format!("[{}]", query_name).bold(),
            count,
            if count == 1 { "" } else { "s" }
        );

        for item in results {
            if let Some(obj) = item.as_object() {
                let parts: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{}: {}",
                            k.dimmed(),
                            v.as_str()
                                .map(String::from)
                                .unwrap_or_else(|| v.to_string())
                        )
                    })
                    .collect();
                println!("  • {}", parts.join(", "));
            } else {
                println!("  • {}", item);
            }
        }
    } else if let Some(error) = result.get("error").and_then(|e| e.as_str()) {
        println!("\n{} {}", "Error:".red().bold(), error);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(result).unwrap_or_default()
        );
    }
}

/// Resolve the diff input mode and return file diffs.
fn resolve_file_diffs(
    rev: Option<String>,
    patch_path: Option<PathBuf>,
    stdin: bool,
    before: Option<PathBuf>,
    after: Option<PathBuf>,
    files_filter: &[String],
) -> Result<Vec<FileDiff>> {
    let mut diffs = if let (Some(before_path), Some(after_path)) = (before, after) {
        // Legacy file mode
        let before_code = std::fs::read_to_string(&before_path)
            .with_context(|| format!("Cannot read {}", before_path.display()))?;
        let after_code = std::fs::read_to_string(&after_path)
            .with_context(|| format!("Cannot read {}", after_path.display()))?;

        let path = after_path.to_string_lossy().to_string();
        let language = std::path::Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Language::from_extension);

        vec![FileDiff {
            path,
            language,
            status: DiffStatus::Modified,
            before: Some(before_code),
            after: Some(after_code),
        }]
    } else if let Some(patch_path) = patch_path {
        let contents = std::fs::read_to_string(&patch_path)
            .with_context(|| format!("Cannot read patch file {}", patch_path.display()))?;
        patch::parse_unified_diff(&contents)?
    } else if stdin {
        let mut contents = String::new();
        std::io::stdin()
            .read_to_string(&mut contents)
            .context("Failed to read from stdin")?;
        patch::parse_unified_diff(&contents)?
    } else {
        // Git-native mode
        git::diff_from_spec(rev.as_deref())?
    };

    // Apply file filter if specified
    if !files_filter.is_empty() {
        diffs.retain(|d| files_filter.iter().any(|f| d.path.contains(f.as_str())));
    }

    Ok(diffs)
}

#[allow(clippy::too_many_arguments)]
async fn cmd_diff(
    config: &Config,
    rev: Option<String>,
    patch_path: Option<PathBuf>,
    stdin: bool,
    before: Option<PathBuf>,
    after: Option<PathBuf>,
    goal: String,
    files_filter: Vec<String>,
    language: Option<String>,
    output: OutputFormat,
) -> Result<()> {
    let diffs = resolve_file_diffs(rev, patch_path, stdin, before, after, &files_filter)?;

    if diffs.is_empty() {
        println!("{} No changes found.", "→".cyan().bold());
        return Ok(());
    }

    let bridge = require_bridge(config).await?;

    println!(
        "{} Analyzing {} file{}...",
        "→".cyan().bold(),
        diffs.len(),
        if diffs.len() == 1 { "" } else { "s" }
    );

    let mut all_analyses: Vec<(String, DiffAnalysis)> = Vec::new();

    for diff in &diffs {
        let before_code = diff.before.as_deref().unwrap_or("");
        let after_code = diff.after.as_deref().unwrap_or("");

        if before_code.is_empty() && after_code.is_empty() {
            continue;
        }

        let auto_lang = diff.language.map(|l| l.name().to_string());
        let lang = language.as_deref().or(auto_lang.as_deref());

        println!("  {} {}", status_icon(diff.status), diff.path.dimmed());

        let analysis = bridge
            .diff_analyze(before_code, after_code, &goal, lang, None)
            .await?;

        all_analyses.push((diff.path.clone(), analysis));
    }

    render_diff_results(&all_analyses, &output)?;
    Ok(())
}

fn status_icon(status: DiffStatus) -> colored::ColoredString {
    match status {
        DiffStatus::Added => "+".green().bold(),
        DiffStatus::Deleted => "-".red().bold(),
        DiffStatus::Modified => "~".yellow().bold(),
        DiffStatus::Renamed => "→".cyan().bold(),
    }
}

fn render_diff_results(analyses: &[(String, DiffAnalysis)], output: &OutputFormat) -> Result<()> {
    match output {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = analyses
                .iter()
                .map(|(path, a)| {
                    let mut v = serde_json::to_value(a).unwrap_or_default();
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("file".to_string(), serde_json::json!(path));
                    }
                    v
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        OutputFormat::Text => {
            if analyses.len() == 1 {
                render_single_diff_analysis(&analyses[0].0, &analyses[0].1);
            } else {
                // Multi-file summary
                let avg_score: f64 = if analyses.is_empty() {
                    0.0
                } else {
                    analyses.iter().map(|(_, a)| a.alignment_score).sum::<f64>()
                        / analyses.len() as f64
                };

                println!(
                    "\n{} {} files analyzed, average alignment: {:.0}%",
                    "Summary:".green().bold(),
                    analyses.len(),
                    avg_score * 100.0
                );

                for (path, analysis) in analyses {
                    let score_pct = analysis.alignment_score * 100.0;
                    let score_color = if score_pct >= 80.0 {
                        format!("{:.0}%", score_pct).green()
                    } else if score_pct >= 50.0 {
                        format!("{:.0}%", score_pct).yellow()
                    } else {
                        format!("{:.0}%", score_pct).red()
                    };
                    println!("  {} {}", score_color, path);

                    if !analysis.concerns.is_empty() {
                        for concern in &analysis.concerns {
                            let severity = concern
                                .get("severity")
                                .and_then(|s| s.as_str())
                                .unwrap_or("?");
                            let msg = concern
                                .get("message")
                                .and_then(|s| s.as_str())
                                .unwrap_or("");
                            println!("       [{}] {}", severity.to_uppercase(), msg);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn render_single_diff_analysis(path: &str, analysis: &DiffAnalysis) {
    println!(
        "\n{} {} — alignment: {:.0}%",
        "Result:".green().bold(),
        path,
        analysis.alignment_score * 100.0
    );

    if let Some(reasoning) = &analysis.alignment_reasoning {
        println!("  {}", reasoning);
    }

    if let Some(changes) = analysis.changes_detected.as_object() {
        println!("\n  {}:", "Changes".bold());
        for (key, val) in changes {
            if let Some(arr) = val.as_array() {
                if !arr.is_empty() {
                    let items: Vec<String> = arr.iter().map(|v| v.to_string()).collect();
                    println!("    {}: {}", key, items.join(", "));
                }
            }
        }
    }

    if !analysis.concerns.is_empty() {
        println!("\n  {}:", "Concerns".bold());
        for concern in &analysis.concerns {
            let severity = concern
                .get("severity")
                .and_then(|s| s.as_str())
                .unwrap_or("?");
            let msg = concern
                .get("message")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            println!("    [{}] {}", severity.to_uppercase(), msg);
        }
    }

    if !analysis.unexpected_changes.is_empty() {
        println!("\n  {}:", "Unexpected Changes".bold());
        for change in &analysis.unexpected_changes {
            println!("    - {}", change);
        }
    }
}

// ============================================================================
// Review command — lens + diff + query in one shot
// ============================================================================

async fn cmd_review(
    config: &Config,
    rev: Option<String>,
    goal: String,
    queries: Vec<String>,
    output: OutputFormat,
) -> Result<()> {
    let diffs = git::diff_from_spec(rev.as_deref())?;

    if diffs.is_empty() {
        println!("{} No changes found.", "→".cyan().bold());
        return Ok(());
    }

    let bridge = require_bridge(config).await?;
    let (repo_id, commit_sha) = detect_repo_context();
    let mut analyzer = Analyzer::new()?;

    // Phase 1: Lens changed files
    let lensable: Vec<&FileDiff> = diffs
        .iter()
        .filter(|d| d.after.is_some() && d.language.is_some())
        .collect();

    println!(
        "{} Reviewing {} changed file{}...",
        "→".cyan().bold(),
        diffs.len(),
        if diffs.len() == 1 { "" } else { "s" }
    );

    if !lensable.is_empty() {
        println!(
            "\n{} Lensing {} file{}...",
            "1.".bold(),
            lensable.len(),
            if lensable.len() == 1 { "" } else { "s" }
        );

        for diff in &lensable {
            let code = diff.after.as_ref().unwrap();
            let lang = diff.language.unwrap();

            print!("   {} {} ", status_icon(diff.status), diff.path.dimmed());

            if let Ok(_result) = analyzer.lens_code(code, lang, &diff.path, &commit_sha) {
                if bridge
                    .upload_facts(code, lang.name(), &diff.path, &commit_sha, Some(&repo_id))
                    .await
                    .is_ok()
                {
                    println!("{}", "✓".green());
                } else {
                    println!("{}", "⚠ upload failed".yellow());
                }
            } else {
                println!("{}", "⚠ parse failed".yellow());
            }
        }
    }

    // Phase 2: Diff analysis
    let diffable: Vec<&FileDiff> = diffs
        .iter()
        .filter(|d| d.before.is_some() || d.after.is_some())
        .collect();

    if !diffable.is_empty() {
        println!(
            "\n{} Analyzing {} diff{}...",
            "2.".bold(),
            diffable.len(),
            if diffable.len() == 1 { "" } else { "s" }
        );

        let mut analyses = Vec::new();

        for diff in &diffable {
            let before_code = diff.before.as_deref().unwrap_or("");
            let after_code = diff.after.as_deref().unwrap_or("");

            if before_code.is_empty() && after_code.is_empty() {
                continue;
            }

            let auto_lang = diff.language.map(|l| l.name().to_string());

            match bridge
                .diff_analyze(before_code, after_code, &goal, auto_lang.as_deref(), None)
                .await
            {
                Ok(analysis) => analyses.push((diff.path.clone(), analysis)),
                Err(e) => {
                    println!("   {} {} — {}", "⚠".yellow(), diff.path.dimmed(), e);
                }
            }
        }

        if !analyses.is_empty() {
            render_diff_results(&analyses, &output)?;
        }
    }

    // Phase 3: Run queries
    let query_names: Vec<String> = if queries.is_empty() {
        vec!["orphans".to_string(), "test_gaps".to_string()]
    } else {
        queries
    };

    println!(
        "\n{} Running {} quer{}...",
        "3.".bold(),
        query_names.len(),
        if query_names.len() == 1 { "y" } else { "ies" }
    );

    for query_name in &query_names {
        match bridge.query(&repo_id, query_name, Some(&commit_sha)).await {
            Ok(result) => match output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Text => {
                    render_query_result(query_name, &result);
                }
            },
            Err(e) => {
                println!("   {} {} — {}", "⚠".yellow(), query_name, e);
            }
        }
    }

    Ok(())
}

async fn cmd_onboard(
    project_root: &std::path::Path,
    mut config: Config,
    name: Option<String>,
    agent: bool,
) -> Result<()> {
    println!("{}", "DataGrout Onboarding".blue().bold());

    // Check if already configured.
    if let Some(ref url) = config.datagrout_url {
        if Bridge::has_identity() {
            println!("  {} Already connected to {}", "✓".green(), url);
            println!(
                "  Run {} to rerun setup or change the URL.",
                "invariant init".bold()
            );
            return Ok(());
        }
    }

    // Determine agent name: flag > hostname env > fallback.
    let agent_name = name.unwrap_or_else(|| {
        let host = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "machine".to_string());
        format!("invariant-{}", host)
    });

    if !agent {
        // Human-facing: show what we're about to do.
        println!("  {} {}", "Name:".green(), agent_name);
        println!("  {} app.datagrout.ai", "Gateway:".green());
        println!();
        println!(
            "  This will create a DataGrout account and provision a private MCP server for you."
        );
        println!(
            "  Your mTLS identity will be saved to {} — no tokens needed afterward.",
            "~/.conduit/".bold()
        );
        println!();

        // Confirm before proceeding (unless stdin is not a terminal).
        use std::io::IsTerminal as _;
        if std::io::stdin().is_terminal() {
            print!("  Continue? [Y/n] ");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut answer = String::new();
            let _ = std::io::stdin().read_line(&mut answer);
            let answer = answer.trim().to_lowercase();
            if !answer.is_empty() && answer != "y" && answer != "yes" {
                println!("  Cancelled.");
                return Ok(());
            }
        }
    }

    println!("\n  {} Registering with DataGrout...", "→".cyan());

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message("Contacting DataGrout...");
    pb.enable_steady_tick(std::time::Duration::from_millis(80));

    match Bridge::onboard(OnrampOptions {
        gateway: "https://app.datagrout.ai".to_string(),
        agent_name: agent_name.clone(),
        agent_type: Some(format!("invariant/{}", invariant_core::VERSION)),
        intended_use: Some("Semantic code analysis via Invariant CLI".to_string()),
        access_code: None,
    })
    .await
    {
        Ok((_, server_url)) => {
            pb.finish_and_clear();
            config.datagrout_url = Some(server_url.clone());
            config.repo_id = {
                let (repo_id, _) = detect_repo_context();
                if repo_id != "unknown" {
                    Some(repo_id)
                } else {
                    None
                }
            };

            let config_path = config.save(project_root)?;

            println!("  {} Account created!", "✓".green().bold());
            println!("  {} {}", "Server URL:".green(), server_url);
            println!("  {} mTLS identity saved to ~/.conduit/", "✓".green());
            println!(
                "  {} Config saved to {}",
                "✓".green(),
                config_path.display()
            );
            println!();
            println!("{}", "Ready to go!".green().bold());
            println!("  {} to analyze your code", "invariant lens".bold());
            println!("  {} to check for issues", "invariant query orphans".bold());
        }
        Err(e) => {
            pb.finish_and_clear();
            println!("  {} Registration failed: {}", "✗".red().bold(), e);
            println!();
            println!("  If the error persists:");
            println!("    • Check your internet connection");
            println!("    • Visit https://app.datagrout.ai to sign up manually");
            println!(
                "    • Then run: {} --url <your-server-url>",
                "invariant init".bold()
            );
            return Err(e);
        }
    }

    Ok(())
}

fn cmd_status(config: &Config) {
    println!("{}", "Invariant Status".blue().bold());
    println!("  {} v{}", "Version:".green(), invariant_core::VERSION);

    match config.resolve_url(None) {
        Some(url) => println!("  {} {}", "DataGrout:".green(), url),
        None => println!(
            "  {} Not configured (run `invariant init`)",
            "DataGrout:".yellow()
        ),
    }

    if Bridge::has_identity() {
        println!("  {} mTLS identity present", "Identity:".green());
    } else if config.access_token.is_some() {
        println!(
            "  {} Bearer token configured (mTLS bootstrap unavailable)",
            "Identity:".yellow()
        );
    } else {
        println!(
            "  {} No identity (run `invariant init --token <token>`)",
            "Identity:".yellow()
        );
    }

    if let Some(ref repo_id) = config.repo_id {
        println!("  {} {}", "Repo:".green(), repo_id);
    }
}
