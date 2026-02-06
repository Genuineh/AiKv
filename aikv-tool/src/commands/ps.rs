//! ak ps 命令 - 查看服务运行状态

use colored::Colorize;
use comfy_table::presets::UTF8_FULL;
use comfy_table::Table;

use crate::paths;
use crate::resources::config::AkConfig;
use crate::resources::services::ServicesStatus;
use crate::utils::helpers::{get_mode_name, get_state_subdir};
use crate::{OutputFormat, PsArgs};

pub async fn execute(
    args: PsArgs,
    is_cluster: bool,
    _config: &AkConfig,
) -> anyhow::Result<()> {
    // YAML 格式: 直接输出 docker-compose.yaml 原文
    if args.output == OutputFormat::Yaml {
        let run_dir = paths::run_dir()?;
        let state_subdir = get_state_subdir(is_cluster);
        let staged_compose = run_dir
            .join(state_subdir)
            .join(crate::constants::files::DOCKER_COMPOSE);
        if staged_compose.exists() {
            let content = std::fs::read_to_string(staged_compose)?;
            println!("{}", content);
            return Ok(());
        }
    }

    // 获取服务状态
    let status = ServicesStatus::get(is_cluster).await?;

    match args.output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        OutputFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&status)?);
        }
        OutputFormat::Table => {
            if !status.is_any_running() {
                println!("{}", "No AiKv services running.".red());
                return Ok(());
            }

            if let Some(ref bin) = status.bin {
                println!("\n{}", "Local binary:".blue().bold());
                let mut table = Table::new();
                table
                    .load_preset(UTF8_FULL)
                    .set_header(vec!["Name", "Status", "PID", "Memory", "Uptime"]);
                table.add_row(vec![
                    &bin.name,
                    &bin.status.green().to_string(),
                    &bin.pid.to_string(),
                    &bin.memory,
                    &bin.uptime,
                ]);
                println!("{table}");
            }

            if !status.docker.is_empty() {
                let mode_name = get_mode_name(is_cluster);
                println!("\nDocker containers ({}):", mode_name);
                let mut table = Table::new();
                table
                    .load_preset(UTF8_FULL)
                    .set_header(vec!["Name", "Status", "Ports"]);
                for svc in &status.docker {
                    table.add_row(vec![&svc.name, &svc.status, &svc.ports]);
                }
                println!("{table}");
            }
        }
    }

    Ok(())
}
