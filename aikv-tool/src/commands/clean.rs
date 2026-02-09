//! ak clean 命令 - 清理环境

use colored::*;
use std::fs;
use sysinfo::{Pid, System};

use crate::constants::{files, DOCKER_PROJECT_NAME};
use crate::paths;
use crate::resources::config::AkConfig;
use crate::runtime::docker;
use crate::utils::helpers::{get_mode_name, get_state_subdir};
use crate::{CleanArgs, RunMode};

pub async fn execute(
    args: CleanArgs,
    mode: Option<RunMode>,
    is_cluster: bool,
    config: &AkConfig,
) -> anyhow::Result<()> {
    if !args.force {
        // 检查是否有服务正在运行
        let run_dir = paths::run_dir()?;
        let mut running_modes = Vec::new();

        // 检查本地二进制
        let pid_path = run_dir.join(files::PID_FILE);
        if pid_path.exists() {
            let pid_str = fs::read_to_string(&pid_path)?;
            if let Ok(raw_pid) = pid_str.trim().parse::<u32>() {
                let mut sys = System::new_all();
                sys.refresh_processes(
                    sysinfo::ProcessesToUpdate::Some(&[Pid::from(raw_pid as usize)]),
                    true,
                );
                if sys.process(Pid::from(raw_pid as usize)).is_some() {
                    running_modes.push("Local binary");
                }
            }
        }

        // 检查 Docker
        for mode in &["single", "cluster"] {
            let compose_file = run_dir.join(mode).join(files::DOCKER_COMPOSE);
            if compose_file.exists() {
                let mut cmd = docker::compose_command(DOCKER_PROJECT_NAME, &compose_file).await?;
                cmd.arg("ps").arg("--format").arg("json");
                if let Ok(output) = cmd.output().await {
                    let out_str = String::from_utf8_lossy(&output.stdout);
                    if !out_str.trim().is_empty() && out_str.trim() != "[]" {
                        running_modes.push(if *mode == "cluster" {
                            "Docker cluster"
                        } else {
                            "Docker single"
                        });
                    }
                }
            }
        }

        if !running_modes.is_empty() {
            println!("{}", "Refusing to clean: the following services are running: ".red().bold());
            for m in running_modes {
                println!("   - {}", m.yellow());
            }
            println!(
                "\n{} Run {} to stop services, or {} to force clean.",
                "Hint:".bright_black(),
                "ak down".green(),
                "ak clean --force".cyan()
            );
            return Ok(());
        }
    }

    println!("Cleaning AiKv environment...");

    let run_dir = paths::run_dir()?;
    let log_dir = paths::log_dir()?;

    let resolved_mode = mode.unwrap_or(config.defaults.mode);

    if args.all {
        // 重置: 除 config 外全部清理, 如同新安装
        if run_dir.exists() {
            fs::remove_dir_all(&run_dir)?;
            fs::create_dir_all(&run_dir)?;
            println!("   Cleared all run state (~/.local/state/ak/run/)");
        }
        if log_dir.exists() {
            fs::remove_dir_all(&log_dir)?;
            fs::create_dir_all(&log_dir)?;
            println!("   Cleared cache and logs (~/.cache/ak/logs/)");
        }
    } else {
        // 按 -m/--mode 清理当前目标
        match resolved_mode {
            RunMode::Bin => {
                let pid_path = run_dir.join(files::PID_FILE);
                if pid_path.exists() {
                    let _ = fs::remove_file(&pid_path);
                    println!("   Cleared run state (local binary)");
                }
            }
            RunMode::Docker => {
                let target_dir = run_dir.join(get_state_subdir(is_cluster));
                if target_dir.exists() {
                    fs::remove_dir_all(&target_dir)?;
                    println!(
                        "   Cleared run state for {}",
                        get_mode_name(is_cluster)
                    );
                }
            }
        }
        if log_dir.exists() {
            fs::remove_dir_all(&log_dir)?;
            fs::create_dir_all(&log_dir)?;
            println!("   Cleared logs (~/.cache/ak/logs/)");
        }
    }

    // 清理 PID 文件 (all 时 run_dir 已清空; 非 all 时 bin 分支已删或此处删残留)
    let pid_path = run_dir.join(files::PID_FILE);
    if pid_path.exists() {
        let _ = fs::remove_file(pid_path);
    }

    println!("{}", "Clean finished.".green().bold());
    Ok(())
}
