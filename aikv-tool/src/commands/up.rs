//! ak up 命令 - 启动服务

use colored::*;
use std::fs;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::constants::{files, DOCKER_PROJECT_NAME, DEFAULT_SHARDS, DEFAULT_REPLICAS, NETWORK_NAME};
use crate::paths;
use crate::resources::config::AkConfig;
use crate::runtime::{docker, process};
use crate::utils::helpers::{get_mode_name, get_state_subdir};
use crate::{RunMode, UpArgs};

pub async fn execute(
    args: UpArgs,
    mode: RunMode,
    is_cluster: bool,
    config: &AkConfig,
    project_root: &Path,
) -> anyhow::Result<()> {
    match mode {
        RunMode::Bin => execute_bin(args, config, project_root).await,
        RunMode::Docker => execute_docker(args, is_cluster, config).await,
    }
}

async fn execute_bin(_args: UpArgs, config: &AkConfig, project_root: &Path) -> anyhow::Result<()> {
    // 预检端口
    if let Err(e) = process::check_port_availability(config.defaults.port) {
        anyhow::bail!(
            "failed to start local process: {}. Check if another AiKv instance is running",
            e
        );
    }

    let mut bin_path = project_root.join("target/release/aikv");
    if !bin_path.exists() {
        bin_path = project_root.join("target/debug/aikv");
    }

    if !bin_path.exists() {
        anyhow::bail!("build artifact not found, run 'ak build' first");
    }

    let config_path = project_root.join(files::AIKV_CONFIG);
    let mut cmd = Command::new(&bin_path);
    cmd.current_dir(project_root);

    if config_path.exists() {
        cmd.arg("--config").arg(&config_path);
    } else {
        cmd.arg("--host").arg("127.0.0.1");
    }

    println!("Starting AiKv in background...");

    let log_file_path = paths::log_dir()?.join(files::AIKV_LOG);
    let log_file = fs::File::create(&log_file_path)?;

    let child = cmd
        .stdout(Stdio::from(log_file.try_clone()?))
        .stderr(Stdio::from(log_file))
        .spawn()?;

    let pid = child
        .id()
        .ok_or_else(|| anyhow::anyhow!("could not get process PID"))?;

    process::write_pid_file(pid)?;

    println!("   {} {}", "Status:".blue(), "Started".green());
    println!("   {} {}", "PID:".blue(), pid.to_string().yellow());
    println!("   {} {:?}", "Log:".blue(), log_file_path);
    println!(
        "   {} {}",
        "Hint:".bright_black(),
        "Use 'ak down' to stop"
    );

    Ok(())
}

async fn execute_docker(args: UpArgs, is_cluster: bool, config: &AkConfig) -> anyhow::Result<()> {
    // 预检 Docker 引擎
    docker::check_docker_alive().await?;

    let docker_image = args
        .image
        .unwrap_or_else(|| config.defaults.docker_image.clone());
    let mode_name = get_mode_name(is_cluster);
    let state_subdir = get_state_subdir(is_cluster);
    // 标准化测试: 从配置读取 Docker 资源限制 (需同时配置 cpus 与 memory 才生效)
    let resource_limits = match (&config.defaults.docker_cpus, &config.defaults.docker_memory) {
        (Some(cpus), Some(memory)) => Some((cpus.clone(), memory.clone())),
        _ => None,
    };

    // 参数校验
    if is_cluster {
        if let Some(s) = args.shards {
            if s == 0 {
                anyhow::bail!("shards must be at least 1");
            }
        }
    }

    // 端口预检
    let count = if is_cluster {
        if let Some(n) = args.nodes {
            n
        } else {
            args.shards.unwrap_or(DEFAULT_SHARDS) * (1 + args.replicas.unwrap_or(DEFAULT_REPLICAS))
        }
    } else {
        1
    };

    let mut occupied = Vec::new();
    for i in 0..count {
        let port = 6379 + i as u16;
        if process::check_port_availability(port).is_err() {
            occupied.push(port);
        }
    }

    if !occupied.is_empty() {
        println!(
            "{} Port(s) already in use: {:?}",
            "Refusing to start:".red().bold(),
            occupied
        );
        println!(
            "   {} Run {} or clean up manually.",
            "Hint:".bright_black(),
            "ak down".green()
        );
        anyhow::bail!("port conflict");
    }

    let run_dir = paths::run_dir()?.join(state_subdir);
    fs::create_dir_all(&run_dir)?;
    let staged_compose = run_dir.join(files::DOCKER_COMPOSE);

    // 检查网络状态并确定网络配置模式
    // 如果网络存在，使用 external: true；如果不存在，让 Docker Compose 创建网络
    let network_exists = docker::network_exists(NETWORK_NAME).await?;
    let network_external = network_exists;

    if network_exists {
        println!("   {} Network '{}' exists (using as external)", "Info:".cyan(), NETWORK_NAME);
    } else {
        println!("   {} Network '{}' does not exist (Docker Compose will create it)", "Info:".cyan(), NETWORK_NAME);
    }

    // 生成配置
    if is_cluster {
        if let Some(n) = args.nodes {
            println!(
                "Generating {} config (node mode, {} nodes)...",
                mode_name,
                n
            );
            docker::generate_dynamic_configs(&run_dir, &docker_image, Some(n), None, None, network_external, resource_limits.clone())?;
        } else {
            let s = args.shards.unwrap_or(DEFAULT_SHARDS);
            let r = args.replicas.unwrap_or(DEFAULT_REPLICAS);
            println!(
                "Generating {} topology ({} shards, {} replicas per shard)...",
                mode_name,
                s,
                r
            );
            docker::generate_dynamic_configs(&run_dir, &docker_image, None, Some(s), Some(r), network_external, resource_limits.clone())?;
        }
    } else {
        println!("Generating {} config...", mode_name);
        docker::generate_dynamic_configs(&run_dir, &docker_image, Some(1), None, None, network_external, resource_limits)?;
    }

    println!("Starting AiKv {} containers (background)...", mode_name);

    // 镜像检查
    if !docker::image_exists(&docker_image).await {
        if docker_image == "aikv:latest" || docker_image == config.defaults.docker_image {
            println!(
                "Image {} not found locally.",
                docker_image.cyan()
            );
            println!(
                "   {} Run {} to build, or {} to use a remote image.",
                "Hint:".bright_black(),
                "ak build -m docker".bold().green(),
                "--image".bold().cyan()
            );
            anyhow::bail!("required Docker image missing");
        } else {
            println!(
                "Image {} not found locally, will try to pull from registry...",
                docker_image.cyan()
            );
        }
    }

    let mut cmd = docker::compose_command(DOCKER_PROJECT_NAME, &staged_compose).await?;
    cmd.current_dir(&run_dir);
    cmd.arg("up").arg("-d");

    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await?;

    if status.success() {
        println!("\n{} containers started.", mode_name);
        println!(
            "   {} Use 'ak logs -f' for logs, 'ak down' to stop.",
            "Hint:".bright_black()
        );
    } else {
        anyhow::bail!("Docker Compose up failed");
    }

    Ok(())
}
