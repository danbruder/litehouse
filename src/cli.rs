use anyhow::Result;
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

    /// Create a new app
    Create {
        /// Name of the app
        app_name: String,
    },

    /// Delete an app
    Delete {
        /// Name of the app
        app_name: String,
    },

    /// Deploy a Docker image tarball to an app
    Deploy {
        /// Name of the app
        app_name: String,

        /// Path to the Docker image tarball
        image_path: String,

        /// Image tag (e.g. myapp:abc123)
        #[arg(long)]
        image_tag: Option<String>,

        /// Git commit hash
        #[arg(long)]
        git_commit: Option<String>,

        /// Don't auto-start the app after deploying
        #[arg(long)]
        no_start: bool,
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
            s3_access_key,
            s3_secret_key,
            s3_bucket,
            s3_region,
            s3_endpoint,
            s3_path_prefix,
        } => crate::commands::install::execute(
            &domain,
            skip_verify,
            s3_access_key.as_deref(),
            s3_secret_key.as_deref(),
            s3_bucket.as_deref(),
            s3_region.as_deref(),
            s3_endpoint.as_deref(),
            s3_path_prefix.as_deref(),
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
            let config = ClientConfig {
                base_url: format!("{}/api", base_url.trim_end_matches('/')),
                api_token: Some(token),
            };
            config.save()?;
            println!("Connected to {}", config.base_url);
            Ok(())
        }
        _ => {
            // For all other commands, load client config and use API client
            let config = ClientConfig::load()?;
            let api_client = ApiClient::new(config);

            match cli.command {
                Commands::Create { app_name } => api_client.create_app(&app_name).await,
                Commands::Start { app_name } => api_client.start_app(&app_name).await,
                Commands::Stop { app_name } => api_client.stop_app(&app_name).await,
                Commands::Restart { app_name } => {
                    println!("Restarting not implemented for app: {}", app_name);
                    Ok(())
                }
                Commands::Delete { app_name } => api_client.delete_app(&app_name).await,
                Commands::Deploy {
                    app_name,
                    image_path,
                    image_tag,
                    git_commit,
                    no_start,
                } => {
                    api_client
                        .deploy_app(
                            &app_name,
                            &image_path,
                            image_tag.as_deref(),
                            git_commit.as_deref(),
                            no_start,
                        )
                        .await
                }
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
                Commands::CheckDns => {
                    crate::commands::check_dns::execute().await
                },
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
