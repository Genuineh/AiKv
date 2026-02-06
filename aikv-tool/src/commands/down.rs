//! ak down 命令 - 停止服务

use colored::*;
use std::fs;
use std::path::Path;
use sysinfo::{Pid, System};

use crate::constants::{files, DOCKER_PROJECT_NAME};
use crate::paths;
use crate::resources::config::AkConfig;
use crate::runtime::{docker, process};
use crate::utils::helpers::{get_mode_name, get_state_subdir};
use crate::{DownArgs, RunMode};

pub async fn execute(
    args: DownArgs,
    mode: RunMode,
    is_cluster: bool,
    config: &AkConfig,
    project_root: &Path,
) -> anyhow::Result<()> {
    match mode {
        RunMode::Bin => execute_bin(args, project_root).await,
        RunMode::Docker => execute_docker(args, is_cluster, config).await,
    }
}

async fn execute_bin(args: DownArgs, project_root: &Path) -> anyhow::Result<()> {
    println!("{}", "Stopping local binary process...".red().bold());

    let pid_path = paths::run_dir()?.join(files::PID_FILE);
    if !pid_path.exists() {
        println!(
            "   {} {}",
            "Hint:".yellow(),
            "No PID file found, AiKv may not be running."
        );
    } else {
        let pid_str = fs::read_to_string(&pid_path)?;
        let raw_pid = pid_str.trim().parse::<u32>()?;
        let pid = Pid::from(raw_pid as usize);

        let mut sys = System::new_all();
        sys.refresh_all();

        if let Some(proc) = sys.process(pid) {
            let name = proc.name();
            let name_str = name.to_string_lossy();
            if name_str.to_lowercase().contains("aikv") {
                println!(
                    "   Found process [{}] (PID: {})",
                    name_str.cyan(),
                    raw_pid
                );

                if process::stop_process(raw_pid)? {
                    println!("   {} {}", "Status:".blue(), "Stop signal sent".green());
                }
            }
        }
        process::cleanup_pid_file()?;
    }

    if args.remove_volumes {
        println!("   Cleaning local data dirs (./data, ./logs)...");
        let data_dir = project_root.join("data");
        let log_dir = project_root.join("logs");
        if data_dir.exists() {
            let _ = fs::remove_dir_all(data_dir);
        }
        if log_dir.exists() {
            let _ = fs::remove_dir_all(log_dir);
        }
        println!("   Data cleaned.");
    }

    Ok(())
}

async fn execute_docker(args: DownArgs, is_cluster: bool, _config: &AkConfig) -> anyhow::Result<()> {
    let mode_name = get_mode_name(is_cluster);
    let state_subdir = get_state_subdir(is_cluster);

    let run_dir = paths::run_dir()?.join(state_subdir);
    let staged_compose = run_dir.join(files::DOCKER_COMPOSE);

    if !staged_compose.exists() {
        println!(
            "{} No running {} found, nothing to stop.",
            "Hint:".blue(),
            mode_name
        );
        return Ok(());
    }

    println!("Stopping AiKv {} containers...", mode_name);

    let mut cmd = docker::compose_command(DOCKER_PROJECT_NAME, &staged_compose).await?;
    cmd.current_dir(&run_dir);
    cmd.arg("down");
    if args.remove_volumes {
        cmd.arg("-v");
        println!("   Removing volumes as well...");
    }

    let status = cmd.status().await?;
    if status.success() {
        println!(
            "   {} {}",
            "Status:".blue(),
            format!("Stopped and removed {} containers", mode_name).green()
        );
    } else {
        anyhow::bail!("Docker Compose down failed");
    }

    Ok(())
}
