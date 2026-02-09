//! ak restart 命令 - 重启服务

use std::fs;

use crate::constants::{files, DOCKER_PROJECT_NAME, DEFAULT_SHARDS};
use crate::paths;
use crate::resources::config::AkConfig;
use crate::runtime::docker;
use crate::utils::helpers::{get_mode_name, get_state_subdir};
use crate::{CleanArgs, DownArgs, RestartArgs, RunMode, UpArgs};

use super::{clean, down, up};

pub async fn execute(
    args: RestartArgs,
    mode: RunMode,
    is_cluster: bool,
    config: &AkConfig,
    project_root: &std::path::Path,
) -> anyhow::Result<()> {
    let is_bin = mode == RunMode::Bin;

    if args.init {
        // 深度重置
        let mode_desc = if is_bin {
            "Local binary"
        } else {
            get_mode_name(is_cluster)
        };
        println!("Performing full reset ({})...", mode_desc);

        // 自动提取当前运行的分片/副本数 (针对 Docker)
        let (shards, replicas) = if !is_bin {
            extract_topology_from_compose(is_cluster)?
        } else {
            (3, 1)
        };

        // 1. 停止并删除卷
        down::execute(
            DownArgs { mode: None, topo: None, remove_volumes: true },
            mode,
            is_cluster,
            config,
            project_root,
        )
        .await?;

        // 2. 清理当前模式状态 + 日志
        clean::execute(
            CleanArgs {
                mode: Some(mode),
                topo: None,
                all: false,
                force: true,
            },
            Some(mode),
            is_cluster,
            config,
        )
        .await?;

        // 3. 重新启动
        let up_args = UpArgs {
            mode: None,
            topo: None,
            nodes: None,
            shards: if is_cluster { Some(shards) } else { None },
            replicas: if is_cluster { Some(replicas) } else { None },
            image: None,
        };

        up::execute(up_args, mode, is_cluster, config, project_root).await?;
    } else {
        // 普通重启
        match mode {
            RunMode::Bin => {
                println!("Restarting local binary process...");
                down::execute(
                    DownArgs { mode: None, topo: None, remove_volumes: false },
                    RunMode::Bin,
                    is_cluster,
                    config,
                    project_root,
                )
                .await?;
                up::execute(
                    UpArgs {
                        mode: None,
                        topo: None,
                        nodes: None,
                        shards: None,
                        replicas: None,
                        image: None,
                    },
                    RunMode::Bin,
                    is_cluster,
                    config,
                    project_root,
                )
                .await?;
            }
            RunMode::Docker => {
                let state_subdir = get_state_subdir(is_cluster);
                let run_dir = paths::run_dir()?.join(state_subdir);
                let staged_compose = run_dir.join(files::DOCKER_COMPOSE);

                if !staged_compose.exists() {
                    anyhow::bail!("no running config found, run 'ak up' first");
                }

                let mode_name = get_mode_name(is_cluster);
                println!("Restarting AiKv {} containers...", mode_name);

                let mut cmd = docker::compose_command(DOCKER_PROJECT_NAME, &staged_compose).await?;
                cmd.current_dir(&run_dir);
                cmd.arg("restart");
                let status = cmd.status().await?;
                if !status.success() {
                    anyhow::bail!("Docker Compose restart failed");
                }
                println!("   Restart done.");
            }
        }
    }

    Ok(())
}

fn extract_topology_from_compose(is_cluster: bool) -> anyhow::Result<(u32, u32)> {
    let state_subdir = get_state_subdir(is_cluster);
    let staged_compose = paths::run_dir()?
        .join(state_subdir)
        .join(files::DOCKER_COMPOSE);

    if staged_compose.exists() {
        if let Ok(content) = fs::read_to_string(&staged_compose) {
            let count = content.split("container_name:").count() - 1;
            if is_cluster && count > 1 {
                if count % 2 == 0 {
                    return Ok(((count / 2) as u32, 1));
                } else {
                    return Ok((count as u32, 0));
                }
            }
        }
    }
    Ok((DEFAULT_SHARDS, 1))
}
