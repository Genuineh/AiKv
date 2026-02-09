//! ak quick 命令 - 一键构建并启动 (build then up)
//!
//! 整合 build 与 up，方便一行命令完成部署。
//! 使用 -f/--force 时会先 down、再 clean 当前模式状态，再强制 build 后 up，实现彻底重新部署。

use std::path::Path;

use crate::resources::config::AkConfig;
use crate::{BuildArgs, CleanArgs, DownArgs, QuickArgs, RunMode, UpArgs};

use super::{build, clean, down, up};

pub async fn execute(
    args: QuickArgs,
    mode: RunMode,
    is_cluster: bool,
    config: &AkConfig,
    project_root: &Path,
) -> anyhow::Result<()> {
    if args.force {
        // 强制重新部署: 先停服务、清理 run 状态，再 build + up
        down::execute(
            DownArgs {
                mode: Some(mode),
                topo: args.topo,
                remove_volumes: true,
            },
            mode,
            is_cluster,
            config,
            project_root,
        )
        .await?;

        clean::execute(
            CleanArgs {
                mode: Some(mode),
                topo: args.topo,
                all: false,
                force: true,
            },
            Some(mode),
            is_cluster,
            config,
        )
        .await?;
    }

    let build_args = BuildArgs {
        mode: Some(mode),
        topo: args.topo,
        image: args.image.clone(),
        force: args.force,
        release: args.release,
    };

    build::execute(build_args, mode, is_cluster, config, project_root).await?;

    let up_args = UpArgs {
        mode: Some(mode),
        topo: args.topo,
        nodes: args.nodes,
        shards: args.shards,
        replicas: args.replicas,
        image: args.image,
    };

    up::execute(up_args, mode, is_cluster, config, project_root).await?;

    Ok(())
}
