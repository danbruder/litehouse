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

    Config,

    Podman(PodmanArgs),
}

#[derive(Subcommand)]
enum PodmanCmd {
    /// Show podman version
    Version,
}

#[tracing::instrument]
pub async fn run() -> Result<()> {
    let cli = Cli::parse();
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
        Commands::Serve => {
            let config = ServerConfig::default();
            server::execute(config).await
        }
        Commands::Config => {
            let client_config = ClientConfig::load()?;
            let client_config_path = ClientConfig::get_config_path()?;

            println!("Client config: {}", client_config_path.display());
            println!("{}", toml::to_string(&client_config)?);

            Ok(())
        }
        Commands::Podman(args) => match args.command {
            PodmanCmd::Version => api_client.get_podman_version().await,
        },
    }
}
