use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};

use crate::api_client::ApiClient;
use crate::commands::server;
use crate::config::{ClientConfig, ServerConfig};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install litehouse on this server (run as root)
    Install {
        /// Base domain for wildcard routing (e.g., lh.example.com)
        #[arg(long)]
        domain: String,

        /// Skip the final verification step
        #[arg(long)]
        skip_verify: bool,

        /// S3 Access Key ID for backups
        #[arg(long)]
        s3_access_key: Option<String>,

        /// S3 Secret Access Key for backups
        #[arg(long)]
        s3_secret_key: Option<String>,

        /// S3 Bucket name for backups
        #[arg(long)]
        s3_bucket: Option<String>,

        /// S3 Region (default: us-east-1)
        #[arg(long)]
        s3_region: Option<String>,

        /// S3 Endpoint URL (optional, for S3-compatible services)
        #[arg(long)]
        s3_endpoint: Option<String>,

        /// S3 Path Prefix (default: litehouse)
        #[arg(long)]
        s3_path_prefix: Option<String>,

        /// GitHub PAT (read:packages scope) used to pull private ghcr.io
        /// images. Configured on the freshly installed server automatically
        /// if provided.
        #[arg(long)]
        ghcr_token: Option<String>,
    },

    /// Upgrade litehouse binary and container image (run as root)
    Upgrade {
        /// Specific version to upgrade to (default: latest)
        #[arg(long)]
        version: Option<String>,

        /// Path to a local binary to use instead of downloading
        #[arg(long)]
        from_path: Option<String>,
    },

    /// Create a new app: registers it on the server, commits a deploy
    /// workflow to the GitHub repo, and sets the deploy-token secret — a
    /// `git push` to the repo is all that's needed to deploy from then on.
    Create {
        /// Name of the app
        app_name: String,

        /// GitHub repo this app deploys from, "owner/name" form. Inferred
        /// from the `origin` git remote in the current directory if omitted.
        #[arg(long)]
        repo: Option<String>,

        /// Re-link an app that already exists: mints a fresh deploy token
        /// and re-commits the workflow instead of failing with a conflict.
        #[arg(long)]
        rotate_token: bool,

        /// Print a single machine-readable JSON object instead of human
        /// text, and never fall back to interactive GitHub device-flow
        /// login.
        #[arg(long)]
        json: bool,
    },

    /// Delete an app
    Delete {
        /// Name of the app
        app_name: String,
    },

    /// Deploy a container image to an app (pulls, replaces the running
    /// container, and syncs Caddy)
    Deploy {
        /// Name of the app
        app_name: String,

        /// Image reference to deploy, e.g. ghcr.io/org/app:sha-abc123
        #[arg(long)]
        image: String,

        /// Git commit sha this image was built from
        #[arg(long)]
        sha: Option<String>,
    },

    /// List (and optionally wait on) an app's deploy history
    Deploys {
        /// Name of the app
        app_name: String,

        /// Number of deploys to show
        #[arg(long, default_value = "20")]
        limit: u32,

        /// Print raw JSON instead of a table
        #[arg(long)]
        json: bool,

        /// Poll until the newest deploy leaves "in_progress"
        #[arg(long)]
        wait: bool,

        /// Max seconds to wait with --wait before giving up (exit code 2)
        #[arg(long, default_value = "600")]
        timeout: u64,
    },

    /// Deploy a binary to an app
    Env {
        /// Name of the app
        app_name: String,

        /// Environment variable key
        key: String,

        /// Environment variable value
        value: String,

        #[arg(long)]
        delete: bool,
    },

    /// Start an app
    Start {
        /// Name of the app
        app_name: String,
    },

    /// Stop an app
    Stop {
        /// Name of the app
        app_name: String,
    },

    /// Restart an app
    Restart {
        /// Name of the app
        app_name: String,
    },

    /// Show app status
    Status {
        /// Name of the app (optional, shows all apps if not specified)
        app_name: Option<String>,
    },

    /// View app logs
    Logs {
        /// Name of the app
        app_name: String,

        /// Number of lines to show
        #[arg(short, long, default_value = "50")]
        lines: usize,

        /// Follow logs in real time
        #[arg(short, long)]
        follow: bool,
    },

    /// Start the BinaryDrop server
    Serve,

    /// Configuration management
    Config {
        #[command(subcommand)]
        command: Option<ConfigCmd>,
    },

    /// Seed the database for testing
    Seed,

    /// Point this CLI at a server: lh connect https://admin.s.danbruder.com --token <TOKEN>
    Connect {
        /// Base URL of the litehouse server (e.g. https://admin.example.com)
        base_url: String,

        /// Admin token issued by the server
        #[arg(long)]
        token: String,
    },

    /// Check DNS configuration for the configured domain
    CheckDns,

    /// GitHub authentication used by `lh create` to commit deploy workflows
    /// and set deploy-token secrets.
    Github {
        #[command(subcommand)]
        command: GithubCmd,
    },

    /// Manage backups (run on-demand, check status)
    Backup {
        #[command(subcommand)]
        command: BackupCmd,
    },

    /// Restore all apps from the newest S3 backup (disaster recovery)
    Restore {
        /// Skip the confirmation prompt (for scripts/agents)
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum BackupCmd {
    /// Run a full backup now and print the report
    Run,
    /// Show the last backup date and report
    Status {
        /// Print raw JSON instead of a human-readable summary
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum GithubCmd {
    /// Run the GitHub device authorization flow and store the resulting
    /// token in the client config for future `lh create` runs.
    Login,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Configure S3 backup settings
    S3 {
        #[command(subcommand)]
        command: S3Cmd,
    },

    /// Configure the GitHub token used to pull private ghcr.io images
    Ghcr {
        #[command(subcommand)]
        command: GhcrCmd,
    },
}

#[derive(Subcommand)]
enum S3Cmd {
    /// Set S3 backup configuration
    Set {
        /// S3 Access Key ID
        #[arg(long)]
        access_key_id: String,

        /// S3 Secret Access Key
        #[arg(long)]
        secret_access_key: String,

        /// S3 Bucket name
        #[arg(long)]
        bucket: String,

        /// S3 Region (e.g., us-east-1)
        #[arg(long)]
        region: String,

        /// S3 Endpoint URL (optional, for S3-compatible services)
        #[arg(long)]
        endpoint: Option<String>,

        /// S3 Path prefix (optional, defaults to 'litehouse')
        #[arg(long)]
        path_prefix: Option<String>,
    },
    /// Get current S3 backup configuration
    Get,
    /// Delete S3 backup configuration
    Delete,
}

#[derive(Subcommand)]
enum GhcrCmd {
    /// Set the GHCR token (a GitHub PAT with read:packages scope)
    Set {
        /// GitHub personal access token with read:packages scope
        #[arg(long)]
        token: String,
    },
    /// Get current GHCR token configuration (redacted)
    Get,
    /// Delete the configured GHCR token
    Delete,
}

#[tracing::instrument]
pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Install {
            domain,
            skip_verify,
            s3_access_key,
            s3_secret_key,
            s3_bucket,
            s3_region,
            s3_endpoint,
            s3_path_prefix,
            ghcr_token,
        } => crate::commands::install::execute(
            &domain,
            skip_verify,
            s3_access_key.as_deref(),
            s3_secret_key.as_deref(),
            s3_bucket.as_deref(),
            s3_region.as_deref(),
            s3_endpoint.as_deref(),
            s3_path_prefix.as_deref(),
            ghcr_token.as_deref(),
        )
        .await,
        Commands::Upgrade { version, from_path } => {
            crate::commands::upgrade::execute(version.as_deref(), from_path.as_deref()).await
        }
        Commands::Serve => {
            let config = ServerConfig::load()?;
            server::execute(config).await
        }
        Commands::Connect { base_url, token } => {
            // Preserve any previously stored GitHub token — connecting to a
            // (possibly different) server has no bearing on GitHub auth.
            let mut config = ClientConfig::load().unwrap_or_default();
            config.base_url = format!("{}/api", base_url.trim_end_matches('/'));
            config.api_token = Some(token);
            config.save()?;
            println!("Connected to {}", config.base_url);
            Ok(())
        }
        _ => {
            // For all other commands, load client config and use API client
            let config = ClientConfig::load()?;
            let api_client = ApiClient::new(config.clone());

            match cli.command {
                Commands::Create {
                    app_name,
                    repo,
                    rotate_token,
                    json,
                } => run_create(&api_client, &config, &app_name, repo, rotate_token, json).await,
                Commands::Start { app_name } => api_client.start_app(&app_name).await,
                Commands::Stop { app_name } => api_client.stop_app(&app_name).await,
                Commands::Restart { app_name } => {
                    println!("Restarting not implemented for app: {}", app_name);
                    Ok(())
                }
                Commands::Delete { app_name } => api_client.delete_app(&app_name).await,
                Commands::Deploy { app_name, image, sha } => {
                    let result = api_client.deploy_app(&app_name, &image, sha.as_deref()).await?;
                    if result.status == "succeeded" {
                        println!("App '{}' deployed successfully (deploy {})", app_name, result.deploy_id);
                        Ok(())
                    } else {
                        eprintln!(
                            "Deploy {} failed: {}",
                            result.deploy_id,
                            result.error.as_deref().unwrap_or("unknown error")
                        );
                        std::process::exit(1);
                    }
                }
                Commands::Deploys {
                    app_name,
                    limit,
                    json,
                    wait,
                    timeout,
                } => run_deploys(&api_client, &app_name, limit, json, wait, timeout).await,
                Commands::Env {
                    app_name,
                    key,
                    value,
                    delete,
                } => api_client.set_env(&app_name, &key, &value, delete).await,
                Commands::Status { app_name } => api_client.get_status(app_name.as_deref()).await,
                Commands::Logs {
                    app_name,
                    lines,
                    follow,
                } => {
                    match api_client.get_logs(&app_name, lines, follow).await? {
                        crate::api_client::LogStream::Full(logs) => println!("{}", logs),
                        crate::api_client::LogStream::Lines(mut stream) => {
                            use futures_util::StreamExt;
                            while let Some(line) = stream.next().await {
                                match line {
                                    Ok(l) => print!("{}", l),
                                    Err(e) => eprintln!("Error: {}", e),
                                }
                            }
                        }
                    }
                    Ok(())
                }
                Commands::Config { command } => {
                    match command {
                        Some(ConfigCmd::S3 { command }) => match command {
                            S3Cmd::Set {
                                access_key_id,
                                secret_access_key,
                                bucket,
                                region,
                                endpoint,
                                path_prefix,
                            } => {
                                api_client
                                    .set_s3_config(
                                        &access_key_id,
                                        &secret_access_key,
                                        &bucket,
                                        &region,
                                        endpoint.as_deref(),
                                        path_prefix.as_deref(),
                                    )
                                    .await
                            }
                            S3Cmd::Get => api_client.get_s3_config().await,
                            S3Cmd::Delete => api_client.delete_s3_config().await,
                        },
                        Some(ConfigCmd::Ghcr { command }) => match command {
                            GhcrCmd::Set { token } => api_client.set_ghcr_token(&token).await,
                            GhcrCmd::Get => api_client.get_ghcr_token().await,
                            GhcrCmd::Delete => api_client.delete_ghcr_token().await,
                        },
                        None => {
                            // Default behavior: show client config
                            let client_config = ClientConfig::load()?;
                            let client_config_path = ClientConfig::get_config_path()?;

                            println!("Client config: {}", client_config_path.display());
                            println!("{}", toml::to_string(&client_config)?);

                            Ok(())
                        }
                    }
                }
                Commands::Seed => {
                    println!("Seeding the database...");
                    crate::db::seed().await;

                    Ok(())
                }
                Commands::CheckDns => {
                    crate::commands::check_dns::execute().await
                },
                Commands::Github { command } => match command {
                    GithubCmd::Login => crate::commands::github_login::execute().await,
                },
                Commands::Backup { command } => match command {
                    BackupCmd::Run => {
                        let report = api_client.run_backup().await?;
                        print_backup_report(&report);
                        if !report.failed.is_empty() {
                            std::process::exit(1);
                        }
                        Ok(())
                    }
                    BackupCmd::Status { json } => {
                        let status = api_client.backup_status().await?;
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "last_backup_date": status.last_backup_date,
                                    "last_backup_report": status.last_backup_report,
                                }))?
                            );
                        } else {
                            match &status.last_backup_date {
                                Some(d) => println!("Last backup date: {}", d),
                                None => println!("No backup has run yet"),
                            }
                            match &status.last_backup_report {
                                Some(report) => print_backup_report(report),
                                None => println!("No backup report available"),
                            }
                        }
                        Ok(())
                    }
                },
                Commands::Restore { yes } => {
                    if !yes {
                        eprint!(
                            "Restoring stops and recreates app containers from the newest S3 backup — continue? [y/N] "
                        );
                        use std::io::Write;
                        std::io::stderr().flush().ok();
                        let mut answer = String::new();
                        std::io::stdin()
                            .read_line(&mut answer)
                            .context("reading confirmation")?;
                        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
                            eprintln!("Aborted. Re-run with --yes to skip this prompt.");
                            std::process::exit(1);
                        }
                    }
                    let report = api_client.restore().await?;
                    println!("Restored {} app(s):", report.restored.len());
                    for name in &report.restored {
                        println!("  - {}", name);
                    }
                    if !report.skipped.is_empty() {
                        println!("Skipped {} app(s):", report.skipped.len());
                        for (name, reason) in &report.skipped {
                            println!("  - {}: {}", name, reason);
                        }
                    }
                    Ok(())
                }
                Commands::Install { .. }
                | Commands::Upgrade { .. }
                | Commands::Serve
                | Commands::Connect { .. } => {
                    unreachable!("Already handled above")
                }
            }
        }
    }
}

/// Print a `backup::BackupReport` in a human-readable form for `lh backup
/// run` / `lh backup status`.
fn print_backup_report(report: &crate::backup::BackupReport) {
    println!("Backup ran at: {}", report.ran_at);
    println!("Succeeded ({}): {}", report.succeeded.len(), report.succeeded.join(", "));
    if report.failed.is_empty() {
        println!("Failed: none");
    } else {
        println!("Failed ({}):", report.failed.len());
        for (name, err) in &report.failed {
            println!("  - {}: {}", name, err);
        }
    }
}

/// `lh deploys <app>`: show deploy history, optionally polling until the
/// newest deploy settles. This is the primitive CI/agents use to verify a
/// deploy actually finished: `lh deploys <app> --wait` exits 0 on success,
/// 1 (with the failure reason on stderr) on failure, 2 on timeout.
async fn run_deploys(
    api_client: &ApiClient,
    app_name: &str,
    limit: u32,
    json: bool,
    wait: bool,
    timeout_secs: u64,
) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    loop {
        let deploys = api_client.list_deploys(app_name, limit).await?;

        let settled = deploys.first().map(|d| d.status != "in_progress").unwrap_or(true);

        if !wait || settled || std::time::Instant::now() >= deadline {
            if json {
                println!("{}", serde_json::to_string_pretty(&deploys.iter().map(|d| {
                    serde_json::json!({
                        "id": d.id,
                        "status": d.status,
                        "image": d.image,
                        "git_sha": d.git_sha,
                        "error": d.error,
                        "created_at": d.created_at,
                        "updated_at": d.updated_at,
                    })
                }).collect::<Vec<_>>())?);
            } else {
                println!(
                    "{:<10} {:<12} {:<12} {:<40} {}",
                    "ID", "STATUS", "SHA", "IMAGE", "CREATED"
                );
                for d in &deploys {
                    let short_id = d.id.chars().take(8).collect::<String>();
                    let short_sha = d
                        .git_sha
                        .as_deref()
                        .map(|s| s.chars().take(10).collect::<String>())
                        .unwrap_or_else(|| "-".to_string());
                    println!(
                        "{:<10} {:<12} {:<12} {:<40} {}",
                        short_id, d.status, short_sha, d.image, d.created_at
                    );
                }
            }

            if !wait {
                return Ok(());
            }

            if !settled {
                eprintln!(
                    "Timed out after {}s waiting for deploy to finish (still in_progress)",
                    timeout_secs
                );
                std::process::exit(2);
            }

            // With --json, stdout must carry ONLY the JSON payload printed
            // above — final status goes to stderr / the exit code.
            return match deploys.first() {
                Some(d) if d.status == "succeeded" => {
                    if json {
                        eprintln!("Deploy {} succeeded", d.id);
                    } else {
                        println!("Deploy {} succeeded", d.id);
                    }
                    Ok(())
                }
                Some(d) => {
                    eprintln!(
                        "Deploy {} failed: {}",
                        d.id,
                        d.error.as_deref().unwrap_or("unknown error")
                    );
                    std::process::exit(1);
                }
                None => {
                    eprintln!("No deploys found for app '{}'", app_name);
                    std::process::exit(1);
                }
            };
        }

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

/// `lh create <app> [--repo owner/name] [--rotate-token] [--json]`: the
/// signature litehouse v2 UX — register the app on the server, commit a
/// deploy workflow to the GitHub repo, and set the deploy-token secret, so
/// that `git push` is the entire deploy story from then on.
async fn run_create(
    api_client: &ApiClient,
    config: &ClientConfig,
    app_name: &str,
    repo: Option<String>,
    rotate_token: bool,
    json: bool,
) -> Result<()> {
    let repo = match repo {
        Some(r) => r,
        None => infer_repo_from_git()?,
    };

    let (owner, repo_name) = repo.split_once('/').ok_or_else(|| {
        anyhow!(
            "--repo must be in 'owner/name' form, got '{}'",
            repo
        )
    })?;

    let create_result = match api_client.create_app(app_name, Some(&repo), rotate_token).await {
        Ok(r) => r,
        Err(e) if !rotate_token && e.to_string().contains("already exists") => {
            return Err(anyhow!(
                "App '{}' already exists. Re-run with --rotate-token to re-link it \
                 (mints a fresh deploy token and re-commits the deploy workflow).",
                app_name
            ));
        }
        Err(e) => return Err(e),
    };

    // The server's base_url already ends in /api; the deploy hook lives
    // alongside the rest of the admin API at /api/hooks/deploy.
    let hook_url = format!("{}/hooks/deploy", config.base_url.trim_end_matches('/'));

    // --json implies non-interactive: never block on a device-flow prompt
    // when the caller is a script/agent expecting a single JSON line.
    let allow_interactive = !json;

    let workflow_setup = async {
        let token = crate::commands::github_login::resolve_github_token(allow_interactive).await?;
        crate::github::actions::put_actions_secret(
            &token,
            owner,
            repo_name,
            "LITEHOUSE_DEPLOY_TOKEN",
            &create_result.deploy_token,
        )
        .await
        .context("setting LITEHOUSE_DEPLOY_TOKEN secret")?;

        let workflow = crate::workflow::render_deploy_workflow(owner, repo_name, &hook_url);
        crate::github::actions::put_file(
            &token,
            owner,
            repo_name,
            ".github/workflows/litehouse-deploy.yml",
            &workflow,
            "Add litehouse deploy workflow",
        )
        .await
        .context("committing .github/workflows/litehouse-deploy.yml")?;

        Ok::<(), anyhow::Error>(())
    }
    .await;

    match workflow_setup {
        Ok(()) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "name": create_result.name,
                        "url": create_result.url,
                        "repo": repo,
                        "workflow_committed": true,
                    }))?
                );
            } else {
                println!("App '{}' created", create_result.name);
                println!("  URL:  {}", create_result.url);
                println!("  Repo: {}", repo);
                println!("git push to deploy.");
            }
            Ok(())
        }
        Err(e) => {
            // The app already exists on the server at this point — don't
            // leave the user stranded not knowing that much succeeded.
            eprintln!(
                "App '{}' was created on the server, but setting up {} failed: {:#}",
                create_result.name, repo, e
            );
            eprintln!(
                "Hint: committing workflow files requires a GitHub token with the `workflow` \
                 scope — if you use the gh CLI, run: gh auth refresh -h github.com -s workflow"
            );
            eprintln!("To finish manually:");
            eprintln!(
                "  1. Set the repo secret LITEHOUSE_DEPLOY_TOKEN on {} \
                 (run `lh create {} --repo {} --rotate-token` to mint a fresh token if needed)",
                repo, app_name, repo
            );
            eprintln!(
                "  2. Commit .github/workflows/litehouse-deploy.yml to {} with a job that builds \
                 and pushes to ghcr.io, then POSTs to {}",
                repo, hook_url
            );
            std::process::exit(1);
        }
    }
}

/// Infer "owner/name" from the `origin` git remote in the current
/// directory. Supports both GitHub HTTPS and SSH remote URL forms.
fn infer_repo_from_git() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .context("running `git remote get-url origin`")?;

    if !output.status.success() {
        return Err(anyhow!(
            "Could not find a git remote named 'origin' in the current directory. \
             Pass --repo owner/name explicitly."
        ));
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let (owner, repo) = crate::github::api::parse_repo_url(&url).map_err(|_| {
        anyhow!(
            "The 'origin' remote ('{}') is not a github.com repo. Pass --repo owner/name explicitly.",
            url
        )
    })?;

    Ok(format!("{}/{}", owner, repo))
}
