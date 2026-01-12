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

#[derive(Args)]
struct PodmanArgs {
    #[command(subcommand)]
    command: PodmanCmd,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a fresh server for litehouse deployment
    Init {
        /// SSH target (e.g., root@192.168.1.1)
        ssh_target: String,

        /// Base domain for wildcard routing (e.g., lh.danbruder.com)
        #[arg(long)]
        domain: String,
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

    Podman(PodmanArgs),

    /// Seed the database for testing
    Seed,

    /// Configure a remote for an app
    Remote {
        /// Name of the app
        app_name: String,

        #[command(subcommand)]
        command: RemoteCmd,
    },
}

#[derive(Subcommand)]
enum PodmanCmd {
    /// Show podman version
    Version,
    /// Run a test container
    Run,
}

#[derive(Subcommand)]
enum RemoteCmd {
    /// Add a remote
    Add {

        /// Remote name
        remote: String,
    },
    /// Remove a remote
    Remove ,
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
        Commands::Init { ssh_target, domain } => {
            crate::commands::init::execute(&ssh_target, &domain).await
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
        Commands::Podman(args) => match args.command {
            PodmanCmd::Version => api_client.get_podman_version().await,
            PodmanCmd::Run => crate::podman::run("yeet", "redis:latest").await,
        },

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
        Commands::Init { .. } | Commands::Serve => unreachable!("Already handled above"),
            }
        }
    }
}
