use anyhow::Result;
use clap::{Args, Parser, Subcommand};

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
    },

    /// Upgrade litehouse binary and container image (run as root)
    Upgrade {
        /// Specific version to upgrade to (default: latest)
        #[arg(long)]
        version: Option<String>,
    },

    /// Create a new app
    Create {
        /// Name of the app (optional if using --from-github)
        app_name: Option<String>,

        /// Create from a GitHub repository (format: owner/repo)
        #[arg(long)]
        from_github: Option<String>,
    },

    /// Delete an app
    Delete {
        /// Name of the app
        app_name: String,
    },

    /// Deploy a binary to an app
    Deploy {
        /// Name of the app
        app_name: String,

        /// Path to the binary file
        binary_path: String,
    },

    /// Build an app
    Build {
        /// Name of the app
        app_name: String,
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

    /// Configure a remote for an app
    Remote {
        /// Name of the app
        app_name: String,

        #[command(subcommand)]
        command: RemoteCmd,
    },

    /// Manage GitHub integration
    Github {
        #[command(subcommand)]
        command: GithubCmd,
    },
}

#[derive(Subcommand)]
enum RemoteCmd {
    /// Add a remote
    Add {
        /// Remote name
        remote: String,
    },
    /// Remove a remote
    Remove,
}

#[derive(Subcommand)]
enum GithubCmd {
    /// Connect your GitHub account
    Connect,

    /// Disconnect GitHub account
    Disconnect,

    /// Show connection status
    Status,

    /// List your repositories
    Repos {
        /// Maximum number of repositories to list
        #[arg(short, long, default_value = "30")]
        limit: u32,
    },

    /// Search repositories
    Search {
        /// Search query
        query: String,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Configure S3 backup settings
    S3 {
        #[command(subcommand)]
        command: S3Cmd,
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

#[tracing::instrument]
pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Install {
            domain,
            skip_verify,
        } => crate::commands::install::execute(&domain, skip_verify).await,
        Commands::Upgrade { version } => {
            crate::commands::upgrade::execute(version.as_deref()).await
        }
        Commands::Serve => {
            let config = ServerConfig::load()?;
            server::execute(config).await
        }
        _ => {
            // For all other commands, load client config and use API client
            let config = ClientConfig::load()?;
            let api_client = ApiClient::new(config);

            match cli.command {
                Commands::Create {
                    app_name,
                    from_github,
                } => {
                    match (app_name, from_github) {
                        (Some(name), Some(repo)) => {
                            // Create app with explicit name from GitHub repo
                            api_client.create_app_from_github(&name, &repo).await?;
                            println!("Run 'lh build {}' to build and deploy", name);
                            Ok(())
                        }
                        (None, Some(repo)) => {
                            // Derive app name from repo name
                            let name = repo.split('/').last().unwrap_or(&repo);
                            api_client.create_app_from_github(name, &repo).await?;
                            println!("Run 'lh build {}' to build and deploy", name);
                            Ok(())
                        }
                        (Some(name), None) => {
                            // Standard app creation
                            api_client.create_app(&name).await
                        }
                        (None, None) => {
                            anyhow::bail!("Either app_name or --from-github must be provided");
                        }
                    }
                }
                Commands::Start { app_name } => api_client.start_app(&app_name).await,
                Commands::Stop { app_name } => api_client.stop_app(&app_name).await,
                Commands::Restart { app_name } => {
                    println!("Restarting not implemented for app: {}", app_name);
                    Ok(())
                }
                Commands::Delete { app_name } => api_client.delete_app(&app_name).await,
                Commands::Deploy {
                    app_name,
                    binary_path,
                } => api_client.deploy_app(&app_name, &binary_path).await,
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
                Commands::Remote { app_name, command } => match command {
                    RemoteCmd::Add { remote } => api_client.remote_add(&app_name, &remote).await,
                    RemoteCmd::Remove => api_client.remote_remove(&app_name).await,
                },
                Commands::Build { app_name } => api_client.build(&app_name).await,
                Commands::Github { command } => match command {
                    GithubCmd::Connect => {
                        crate::commands::github::connect::execute(&api_client).await
                    }
                    GithubCmd::Disconnect => {
                        crate::commands::github::disconnect::execute(&api_client).await
                    }
                    GithubCmd::Status => {
                        crate::commands::github::status::execute(&api_client).await
                    }
                    GithubCmd::Repos { limit } => {
                        crate::commands::github::repos::execute(&api_client, limit).await
                    }
                    GithubCmd::Search { query } => {
                        crate::commands::github::search::execute(&api_client, &query).await
                    }
                },
                Commands::Install { .. } | Commands::Upgrade { .. } | Commands::Serve => {
                    unreachable!("Already handled above")
                }
            }
        }
    }
}
