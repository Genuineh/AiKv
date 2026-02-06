//! ak logs 命令 - 查看服务日志

use colored::*;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::constants::{files, DOCKER_PROJECT_NAME};
use crate::paths;
use crate::resources::config::AkConfig;
use crate::runtime::docker;
use crate::utils::helpers::{get_mode_name, get_state_subdir};
use crate::{LogsArgs, RunMode};

pub async fn execute(
    args: LogsArgs,
    mode: RunMode,
    is_cluster: bool,
    _config: &AkConfig,
    _project_root: &Path,
) -> anyhow::Result<()> {
    match mode {
        RunMode::Bin => {
            let log_path = paths::log_dir()?.join(files::AIKV_LOG);
            if !log_path.exists() {
                println!(
                    "{} {}",
                    "Hint:".yellow(),
                    "No log file found. Service may never have been started in background."
                );
                return Ok(());
            }

            println!("Showing AiKv log...");
            println!("   {} {:?}", "Log path:".blue(), log_path);
            println!("\n{}\n", "--- Log Content ---".bright_black());

            let mut cmd = Command::new("tail");
            cmd.arg("-n").arg(args.lines.to_string());
            if args.follow {
                cmd.arg("-f");
            }
            cmd.arg(&log_path);

            cmd.stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await?;
        }
        RunMode::Docker => {
            let state_subdir = get_state_subdir(is_cluster);
            let mode_name = get_mode_name(is_cluster);

            let run_dir = paths::run_dir()?.join(state_subdir);
            let staged_compose = run_dir.join(files::DOCKER_COMPOSE);

            if !staged_compose.exists() {
                anyhow::bail!("no run state found, {} not started", mode_name);
            }

            println!("Showing AiKv {} container logs...", mode_name);

            let mut cmd = docker::compose_command(DOCKER_PROJECT_NAME, &staged_compose).await?;
            cmd.current_dir(&run_dir);
            cmd.arg("logs");
            cmd.arg("-n").arg(args.lines.to_string());
            if args.follow {
                cmd.arg("-f");
            }

            cmd.stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await?;
        }
    }
    Ok(())
}
