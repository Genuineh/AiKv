//! # ak - AiKv management CLI
//!
//! Entry binary: parses subcommands and delegates to command modules.
//! Config and paths follow XDG specification. 

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::fs::OpenOptions;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod commands;
mod constants;
mod paths;
mod resources;
mod runtime;
mod utils;

// Re-export for command modules
pub use resources::config::{AkConfig, RunMode, Topology};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Init XDG dirs
    let log_dir = paths::log_dir()?;
    let ak_log_path = log_dir.join(constants::files::AK_LOG);

    // 2. Logging (terminal + file)
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ak_log_path)?;

    let file_layer = fmt::layer().with_ansi(false).with_writer(file);
    let stdout_layer = fmt::layer().with_target(false);

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with(stdout_layer)
        .with(file_layer)
        .init();

    tracing::debug!("ak started (log path: {:?})", ak_log_path);

    let cli = Cli::parse();
    let config = AkConfig::load()?;

    match cli.command {
        Commands::Build(args) => {
            let root = config.detect_project_root()?;
            let mode = args.mode.unwrap_or(config.defaults.mode);
            let is_cluster = args.topo.unwrap_or(config.defaults.topo).is_cluster();
            commands::build::execute(args, mode, is_cluster, &config, &root).await?;
        }

        Commands::Up(args) => {
            let root = config.detect_project_root()?;
            let mode = args.mode.unwrap_or(config.defaults.mode);
            let is_cluster = args.topo.unwrap_or(config.defaults.topo).is_cluster();
            commands::up::execute(args, mode, is_cluster, &config, &root).await?;
        }

        Commands::Down(args) => {
            let root = config.detect_project_root()?;
            let mode = args.mode.unwrap_or(config.defaults.mode);
            let is_cluster = args.topo.unwrap_or(config.defaults.topo).is_cluster();
            commands::down::execute(args, mode, is_cluster, &config, &root).await?;
        }

        Commands::Restart(args) => {
            let root = config.detect_project_root()?;
            let mode = args.mode.unwrap_or(config.defaults.mode);
            let is_cluster = args.topo.unwrap_or(config.defaults.topo).is_cluster();
            commands::restart::execute(args, mode, is_cluster, &config, &root).await?;
        }

        Commands::Logs(args) => {
            let root = config.detect_project_root()?;
            let mode = args.mode.unwrap_or(config.defaults.mode);
            let is_cluster = args.topo.unwrap_or(config.defaults.topo).is_cluster();
            commands::logs::execute(args, mode, is_cluster, &config, &root).await?;
        }

        Commands::Ps(args) => {
            let is_cluster = args.topo.unwrap_or(config.defaults.topo).is_cluster();
            commands::ps::execute(args, is_cluster, &config).await?;
        }

        Commands::Config(args) => {
            commands::config::execute(args, config).await?;
        }

        Commands::Clean(args) => {
            let is_cluster = args.topo.unwrap_or(config.defaults.topo).is_cluster();
            let mode = args.mode.or(Some(config.defaults.mode));
            commands::clean::execute(args, mode, is_cluster, &config).await?;
        }

        Commands::Quick(args) => {
            let root = config.detect_project_root()?;
            let mode = args.mode.unwrap_or(config.defaults.mode);
            let is_cluster = args.topo.unwrap_or(config.defaults.topo).is_cluster();
            commands::quick::execute(args, mode, is_cluster, &config, &root).await?;
        }
    }

    Ok(())
}

/// ak - AiKv distributed KV store management CLI.
///
/// Docker/kubectl-style UX for local dev, cluster deploy, and production ops.
/// Follows XDG base directory specification.
#[derive(Parser)]
#[command(
    name = "ak",
    bin_name = "ak",
    author,
    version,
    about = "AiKv distributed KV store management CLI",
    long_about = "Docker/kubectl-style UX for local dev, cluster deploy, and production ops. Follows XDG.",
    disable_help_subcommand = true,
    disable_version_flag = true,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Show version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    pub version: (),
}

#[derive(Subcommand)]
pub enum Commands {
    /// Build and start in one shot (build then up)
    Quick(QuickArgs),

    /// Build binary or Docker image
    #[command(visible_alias = "b")]
    Build(BuildArgs),

    /// Start AiKv service
    Up(UpArgs),

    /// Stop and remove running services
    Down(DownArgs),

    /// Restart AiKv service
    Restart(RestartArgs),

    /// View service logs
    #[command(visible_alias = "l")]
    Logs(LogsArgs),

    /// Show service status
    Ps(PsArgs),

    /// Manage tool config
    Config(ConfigArgs),

    /// Clean temp data and logs
    Clean(CleanArgs),
}

// ─── Quick ────────────────────────────────────────────────

#[derive(Args)]
pub struct QuickArgs {
    /// Run target (bin|docker)
    #[arg(short = 'm', long, value_enum)]
    pub mode: Option<RunMode>,

    /// Topology (single|cluster)
    #[arg(short = 't', long, value_enum)]
    pub topo: Option<Topology>,

    /// Total nodes (node-only mode, conflicts with shards/replicas)
    #[arg(short = 'n', long, conflicts_with_all = ["shards", "replicas"])]
    pub nodes: Option<u32>,

    /// Number of shards (masters)
    #[arg(short = 's', long, conflicts_with = "nodes")]
    pub shards: Option<u32>,

    /// Replicas per shard (requires shards)
    #[arg(short = 'r', long, conflicts_with = "nodes", requires = "shards")]
    pub replicas: Option<u32>,

    /// Docker image (build + up)
    #[arg(short = 'i', long)]
    pub image: Option<String>,

    /// Force rebuild before start (bin: cargo clean; docker: overwrite image)
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Build in Release mode (bin only)
    #[arg(long)]
    pub release: bool,
}

// ─── Build ────────────────────────────────────────────────

#[derive(Args)]
pub struct BuildArgs {
    /// Run target (bin|docker)
    #[arg(short = 'm', long, value_enum)]
    pub mode: Option<RunMode>,

    /// Topology (single|cluster)
    #[arg(short = 't', long, value_enum)]
    pub topo: Option<Topology>,

    /// Docker image name (e.g. aikv:dev), only for docker mode
    #[arg(short = 'i', long)]
    pub image: Option<String>,

    /// Force rebuild: bin = cargo clean then build; docker = overwrite existing image
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Build in Release mode
    #[arg(short, long)]
    pub release: bool,
}

// ─── Up ───────────────────────────────────────────────────

#[derive(Args)]
pub struct UpArgs {
    /// Run target (bin|docker)
    #[arg(short = 'm', long, value_enum)]
    pub mode: Option<RunMode>,

    /// Topology (single|cluster)
    #[arg(short = 't', long, value_enum)]
    pub topo: Option<Topology>,

    /// Total nodes (node-only mode, no topology init)
    #[arg(short = 'n', long, conflicts_with_all = ["shards", "replicas"])]
    pub nodes: Option<u32>,

    /// Number of shards (masters)
    #[arg(short = 's', long, conflicts_with = "nodes")]
    pub shards: Option<u32>,

    /// Replicas per shard (slaves)
    #[arg(short = 'r', long, conflicts_with = "nodes", requires = "shards")]
    pub replicas: Option<u32>,

    /// Docker image (default aikv:latest)
    #[arg(short = 'i', long)]
    pub image: Option<String>,
}

// ─── Down ─────────────────────────────────────────────────

#[derive(Args)]
pub struct DownArgs {
    /// Run target (bin|docker)
    #[arg(short = 'm', long, value_enum)]
    pub mode: Option<RunMode>,

    /// Topology (single|cluster)
    #[arg(short = 't', long, value_enum)]
    pub topo: Option<Topology>,

    /// Also remove volumes
    #[arg(short = 'v', long)]
    pub remove_volumes: bool,
}

// ─── Restart ──────────────────────────────────────────────

#[derive(Args)]
pub struct RestartArgs {
    /// Run target (bin|docker)
    #[arg(short = 'm', long, value_enum)]
    pub mode: Option<RunMode>,

    /// Topology (single|cluster)
    #[arg(short = 't', long, value_enum)]
    pub topo: Option<Topology>,

    /// Full reset (clean data then start)
    #[arg(short = 'i', long)]
    pub init: bool,
}

// ─── Logs ─────────────────────────────────────────────────

#[derive(Args)]
pub struct LogsArgs {
    /// Run target (bin|docker)
    #[arg(short = 'm', long, value_enum)]
    pub mode: Option<RunMode>,

    /// Topology (single|cluster)
    #[arg(short = 't', long, value_enum)]
    pub topo: Option<Topology>,

    /// Follow log (like tail -f)
    #[arg(short = 'f', long)]
    pub follow: bool,

    /// Number of recent lines
    #[arg(short = 'n', long, default_value = "100")]
    pub lines: u32,
}

// ─── Ps ───────────────────────────────────────────────────

#[derive(Args)]
pub struct PsArgs {
    /// Run target (bin|docker), omit to show both
    #[arg(short = 'm', long, value_enum)]
    pub mode: Option<RunMode>,

    /// Topology (single|cluster)
    #[arg(short = 't', long, value_enum)]
    pub topo: Option<Topology>,

    /// Output format
    #[arg(short = 'o', long, value_enum, default_value = "table")]
    pub output: OutputFormat,
}

/// Output format (config get, ps, etc.)
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// JSON
    Json,
    /// YAML
    Yaml,
    /// Table
    Table,
}

// ─── Config ───────────────────────────────────────────────

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: ConfigAction,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Show effective config
    Get(ConfigGetArgs),

    /// Set option (e.g. ak config set project.root=/path)
    Set(ConfigSetArgs),

    /// Sync config to current schema
    Sync,

    /// Show config file path
    Path,
}

#[derive(Args)]
pub struct ConfigGetArgs {
    /// Output format
    #[arg(short = 'o', long, value_enum, default_value = "yaml")]
    pub output: OutputFormat,
}

#[derive(Args)]
pub struct ConfigSetArgs {
    /// Option (key=value)
    #[arg(value_name = "KEY=VALUE")]
    pub value: String,
}

// ─── Clean ────────────────────────────────────────────────

#[derive(Args)]
pub struct CleanArgs {
    /// Run target (bin|docker), omit to use config default when cleaning current scope
    #[arg(short = 'm', long, value_enum)]
    pub mode: Option<RunMode>,

    /// Topology (single|cluster)
    #[arg(short = 't', long, value_enum)]
    pub topo: Option<Topology>,

    /// Reset ak: clean all except config (like fresh install)
    #[arg(short = 'a', long)]
    pub all: bool,

    /// Force clean, skip run-state check
    #[arg(short = 'f', long)]
    pub force: bool,
}
