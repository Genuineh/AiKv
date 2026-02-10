//! ak otel 命令 - OTel 观测栈管理
//!
//! 提供一键部署、管理 AiKv 可观测性栈的功能.
//! 支持启动、停止、重启 OTel 组件(Grafana, Prometheus, Jaeger, Loki, Tempo, Pyroscope 等).

use std::path::Path;
use std::process::Stdio;

use colored::*;
use tokio::process::Command;

use crate::constants::NETWORK_NAME;
use crate::resources::config::AkConfig;
use crate::runtime::docker;
use crate::{OtelAction, OtelsArgs};

pub async fn execute(
    args: OtelsArgs,
    config: &AkConfig,
    project_root: &Path,
) -> anyhow::Result<()> {
    match args.action {
        OtelAction::Up => execute_up(args, config, project_root).await,
        OtelAction::Down => execute_down(args, config).await,
        OtelAction::Restart => execute_restart(args, config).await,
        OtelAction::Logs => execute_logs(args, config).await,
        OtelAction::Status => execute_status(args, config).await,
    }
}

async fn execute_up(
    _args: OtelsArgs,
    _config: &AkConfig,
    project_root: &Path,
) -> anyhow::Result<()> {
    // 预检 Docker 引擎
    docker::check_docker_alive().await?;

    // 检查网络状态（如果存在就使用，不存在让 Docker Compose 创建）
    let network_exists = docker::network_exists(NETWORK_NAME).await?;
    if network_exists {
        println!("   {} Network '{}' exists", "Info:".cyan(), NETWORK_NAME);
    } else {
        println!("   {} Network '{}' does not exist (Docker Compose will create it)", "Info:".cyan(), NETWORK_NAME);
    }

    // 确定 OTel compose 文件路径（按优先级查找）
    let compose_paths = [
        // 1. aikv-tool 目录下的 otel 配置（打包时使用）
        project_root.join("otel").join("docker-compose.yaml"),
        // 2. aikv-tool 目录下的根目录配置
        project_root.join("docker-compose.otel.yaml"),
        // 3. 原始 AiKv 项目根目录（兼容旧路径）
        project_root.parent().unwrap_or(project_root).join("docker-compose.otel.yaml"),
        // 4. 原始 AiKv 项目 otel 目录
        project_root.parent().unwrap_or(project_root).join("otel").join("docker-compose.yaml"),
    ];

    let mut compose_path = None;
    let mut compose_dir = project_root;

    for path in &compose_paths {
        if path.exists() {
            compose_path = Some(path.clone());
            compose_dir = path.parent().unwrap_or(compose_dir);
            println!("Found OTel config at: {}", path.display().to_string().cyan());
            break;
        }
    }

    match compose_path {
        Some(path) => {
            println!("Starting OTel stack...");

            let mut cmd = docker::compose_command("aikv-otel", &path).await?;
            cmd.current_dir(&compose_dir);
            cmd.arg("up").arg("-d");

            let status = cmd
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await?;

            if status.success() {
                println!("\n{} OTel stack started successfully!", "Success".green().bold());
                print_access_info();
            } else {
                anyhow::bail!("Failed to start OTel stack");
            }
        }
        None => {
            anyhow::bail!(
                "OTel compose file not found. Searched in: {:?}",
                compose_paths.iter().map(|p| p.display()).collect::<Vec<_>>()
            );
        }
    }

    Ok(())
}

async fn execute_down(args: OtelsArgs, _config: &AkConfig) -> anyhow::Result<()> {
    // 预检 Docker 引擎
    docker::check_docker_alive().await?;

    println!("Stopping OTel stack...");

    // 尝试查找并停止 OTel compose
    let project_root = std::env::current_dir()?;
    let compose_paths = [
        project_root.join("otel").join("docker-compose.yaml"),
        project_root.join("docker-compose.otel.yaml"),
        project_root.parent().unwrap_or(&project_root).join("docker-compose.otel.yaml"),
        project_root.parent().unwrap_or(&project_root).join("otel").join("docker-compose.yaml"),
    ];

    let mut found = false;
    for compose_path in &compose_paths {
        if compose_path.exists() {
            found = true;
            let mut cmd = docker::compose_command("aikv-otel", compose_path).await?;
            cmd.current_dir(compose_path.parent().unwrap_or(&project_root));
            cmd.arg("down").arg("--remove-orphans");

            // 如果指定了 -v 选项，删除卷
            if args.remove_volumes {
                cmd.arg("-v");
            }

            let status = cmd
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await?;

            if status.success() {
                println!("{} OTel stack stopped", "Success".green().bold());
                if args.remove_volumes {
                    println!("{} Volumes removed", "Info".cyan());
                }
            } else {
                anyhow::bail!("Failed to stop OTel stack");
            }
            break;
        }
    }

    if !found {
        println!("{} No OTel compose file found", "Info".cyan());
    }

    Ok(())
}

async fn execute_restart(_args: OtelsArgs, config: &AkConfig) -> anyhow::Result<()> {
    let project_root = std::env::current_dir()?;

    // 先停止
    execute_down(OtelsArgs {
        action: OtelAction::Down,
        follow: false,
        lines: 100,
        remove_volumes: false,
    }, config).await?;

    // 等待一下
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // 再启动
    execute_up(OtelsArgs {
        action: OtelAction::Up,
        follow: false,
        lines: 100,
        remove_volumes: false,
    }, config, &project_root).await
}

async fn execute_logs(args: OtelsArgs, _config: &AkConfig) -> anyhow::Result<()> {
    // 预检 Docker 引擎
    docker::check_docker_alive().await?;

    let project_root = std::env::current_dir()?;
    let compose_paths = [
        project_root.join("otel").join("docker-compose.yaml"),
        project_root.join("docker-compose.otel.yaml"),
        project_root.parent().unwrap_or(&project_root).join("docker-compose.otel.yaml"),
        project_root.parent().unwrap_or(&project_root).join("otel").join("docker-compose.yaml"),
    ];

    let mut found = false;
    for compose_path in &compose_paths {
        if compose_path.exists() {
            found = true;
            let mut cmd = docker::compose_command("aikv-otel", compose_path).await?;
            cmd.current_dir(compose_path.parent().unwrap_or(&project_root));
            cmd.arg("logs");

            if args.follow {
                cmd.arg("-f");
            }

            if args.lines > 0 {
                cmd.arg("--tail").arg(args.lines.to_string());
            }

            let status = cmd
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await?;

            if !status.success() {
                anyhow::bail!("Failed to show logs");
            }
            break;
        }
    }

    if !found {
        println!("{} No OTel compose file found", "Info".cyan());
    }

    Ok(())
}

async fn execute_status(_args: OtelsArgs, _config: &AkConfig) -> anyhow::Result<()> {
    // 预检 Docker 引擎
    docker::check_docker_alive().await?;

    println!("{} OTel services status:", "OTel".cyan().bold());
    println!("{}", "─".repeat(50));

    // 检查网络
    let network_exists = docker::network_exists(NETWORK_NAME).await?;
    if network_exists {
        println!("{} Network '{}' exists", "✓".green(), NETWORK_NAME);
    } else {
        println!("{} Network '{}' not found", "✗".red(), NETWORK_NAME);
    }

    // 获取 OTel 相关容器的状态
    let otel_containers = [
        "grafana", "prometheus", "jaeger", "loki", "tempo",
        "tempo-proxy", "pyroscope", "alloy", "promtail",
        "otel-collector", "node-exporter", "redis-exporter"
    ];

    let mut all_containers = Vec::new();
    for container in &otel_containers {
        let output = Command::new("docker")
            .arg("ps")
            .arg("--filter")
            .arg(format!("name={}", container))
            .arg("--format")
            .arg("table {{.Names}}\t{{.Status}}\t{{.Ports}}")
            .output()
            .await?;

        if output.status.success() {
            let status_output = String::from_utf8_lossy(&output.stdout).into_owned();
            let lines: Vec<&str> = status_output.lines().collect();
            if lines.len() > 1 {
                all_containers.push(status_output);
            }
        }
    }

    if !all_containers.is_empty() {
        println!("\n{} Running containers:", "Containers".cyan().bold());
        for container in all_containers {
            println!("{}", container);
        }
    } else {
        println!("\n{} No OTel containers running", "Info".cyan());
    }

    Ok(())
}

fn print_access_info() {
    println!("\n{}", "─".repeat(50));
    println!("{} Access URLs:", "OTel".cyan().bold());
    println!("  {} Grafana:      http://localhost:3000  (admin/admin)", "→".green());
    println!("  {} Prometheus:   http://localhost:9090", "→".green());
    println!("  {} Jaeger UI:    http://localhost:16686", "→".green());
    println!("  {} Loki:         http://localhost:3100", "→".green());
    println!("  {} Tempo API:    http://localhost:3200", "→".green());
    println!("  {} Pyroscope:    http://localhost:4040", "→".green());
    println!("{}", "─".repeat(50));
}
