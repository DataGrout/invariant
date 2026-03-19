//! Invariant CLI - Semantic code analysis for the AI era

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use ignore::WalkBuilder;
use indicatif::{ProgressBar, ProgressStyle};
use invariant_core::{bridge::Bridge, Analyzer, Config, Language};
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
    Diff {
        /// Path to the original file
        #[arg(long)]
        before: PathBuf,

        /// Path to the modified file
        #[arg(long)]
        after: PathBuf,

        /// Intent / goal the change should accomplish
        #[arg(long)]
        goal: String,

        /// Programming language (auto-detected if omitted)
        #[arg(long)]
        language: Option<String>,

        /// Output format
        #[arg(long, default_value = "text")]
        output: OutputFormat,
    },

    /// Show connection and configuration status
    Status,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

/// Detect repo_id and commit SHA from the current git context.
fn detect_repo_context() -> (String, String) {
    match git2::Repository::discover(".") {
        Ok(repo) => {
            let repo_id = repo
                .workdir()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            let commit_sha = repo
                .head()
                .ok()
                .and_then(|h| h.peel_to_commit().ok())
                .map(|c| c.id().to_string())
                .unwrap_or_else(|| "HEAD".to_string());

            (repo_id, commit_sha)
        }
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

    let bridge = Bridge::connect(&url).await?;
    eprintln!("  {} Connected to DataGrout", "✓".green());
    Ok(bridge)
}

/// Try to connect without failing (for optional upload in lens).
async fn try_bridge(config: &Config) -> Option<Bridge> {
    let url = config.resolve_url(None)?;
    match Bridge::connect(&url).await {
        Ok(bridge) => {
            eprintln!("  {} Connected to DataGrout", "✓".green());
            Some(bridge)
        }
        Err(_) => None,
    }
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
            before,
            after,
            goal,
            language,
            output,
        } => cmd_diff(&config, before, after, goal, language, output).await?,
        Commands::Status => cmd_status(&config),
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
                    println!(
                        "  {} You can retry with: invariant init --url {} --token <your-token>",
                        "hint:".yellow(),
                        url
                    );
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
        println!(
            "\n  {} No DataGrout URL configured. Provide one to enable remote features:",
            "⚠".yellow()
        );
        println!("    invariant init --url https://gateway.datagrout.ai/servers/{{uuid}}/mcp");
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

async fn cmd_diff(
    config: &Config,
    before: PathBuf,
    after: PathBuf,
    goal: String,
    language: Option<String>,
    output: OutputFormat,
) -> Result<()> {
    let bridge = require_bridge(config).await?;

    let before_code = std::fs::read_to_string(&before)?;
    let after_code = std::fs::read_to_string(&after)?;

    println!("{} Analyzing diff...", "→".cyan().bold());

    let analysis = bridge
        .diff_analyze(&before_code, &after_code, &goal, language.as_deref(), None)
        .await?;

    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&analysis)?),
        OutputFormat::Text => {
            println!(
                "\n{} Alignment score: {:.0}%",
                "Result:".green().bold(),
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
