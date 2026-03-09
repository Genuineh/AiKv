//! Docker 运行时操作

use anyhow::{Context, Result};
use colored::Colorize;
use serde::Serialize;
use std::path::Path;
use std::process::Stdio;
use tera::{Context as TeraContext, Tera};
use tokio::process::Command;

use crate::constants::{templates, DOCKER_PORT_BASE, RAFT_PORT_BASE, DEFAULT_SHARDS, DEFAULT_REPLICAS};

/// 智能探测 Docker Compose 命令
pub async fn get_docker_compose_cmd() -> Result<Vec<String>> {
    // 1. 优先检查 'docker compose' (V2)
    let v2_check = Command::new("docker")
        .arg("compose")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    if let Ok(status) = v2_check {
        if status.success() {
            return Ok(vec!["docker".to_string(), "compose".to_string()]);
        }
    }

    // 2. 备选检查 'docker-compose' (V1)
    let v1_check = Command::new("docker-compose")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    if let Ok(status) = v1_check {
        if status.success() {
            return Ok(vec!["docker-compose".to_string()]);
        }
    }

    anyhow::bail!("Docker Compose not found; install Docker (V2) or docker-compose (V1)")
}

/// 检查 Docker 引擎是否正在运行
pub async fn check_docker_alive() -> Result<()> {
    let check = Command::new("docker")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    match check {
        Ok(status) if status.success() => Ok(()),
        _ => anyhow::bail!("Docker daemon not running or not accessible; ensure Docker is running"),
    }
}

/// 创建一个预配置好的 Docker Compose 命令
pub async fn compose_command(project_name: &str, compose_file: &Path) -> Result<Command> {
    let base_cmd = get_docker_compose_cmd().await?;
    let mut cmd = Command::new(&base_cmd[0]);

    for arg in &base_cmd[1..] {
        cmd.arg(arg);
    }

    cmd.arg("-p").arg(project_name);
    cmd.arg("-f").arg(compose_file);

    Ok(cmd)
}

/// 检查 Docker 镜像是否存在
pub async fn image_exists(image_name: &str) -> bool {
    let check_output = Command::new("docker")
        .arg("images")
        .arg("-q")
        .arg(image_name)
        .output()
        .await;

    match check_output {
        Ok(output) => !output.stdout.is_empty(),
        Err(_) => false,
    }
}

/// 检查 Docker 网络是否存在
pub async fn network_exists(network_name: &str) -> Result<bool> {
    let check_output = Command::new("docker")
        .arg("network")
        .arg("ls")
        .arg("--format")
        .arg("{{.Name}}")
        .arg("--filter")
        .arg(format!("name={}", network_name))
        .output()
        .await
        .context("Failed to check network existence")?;

    let output_str = String::from_utf8_lossy(&check_output.stdout);
    Ok(output_str.lines().any(|line| line == network_name))
}

/// 创建 Docker 网络（如果不存在）
#[allow(dead_code)]
pub async fn ensure_network_exists(network_name: &str, create_if_missing: bool) -> Result<bool> {
    if network_exists(network_name).await? {
        println!("   {} Network '{}' already exists (using as external)", "Info:".cyan(), network_name);
        return Ok(true);
    }

    if create_if_missing {
        println!("   {} Creating network '{}'...", "Info:".cyan(), network_name);

        let create_output = Command::new("docker")
            .arg("network")
            .arg("create")
            .arg("--driver")
            .arg("bridge")
            .arg(network_name)
            .output()
            .await
            .context("Failed to create network")?;

        if !create_output.status.success() {
            let error_msg = String::from_utf8_lossy(&create_output.stderr);
            anyhow::bail!("Failed to create network '{}': {}", network_name, error_msg);
        }

        println!("   {} Network '{}' created successfully", "Success:".green(), network_name);
        Ok(true)
    } else {
        Ok(false)
    }
}

#[allow(dead_code)]
/// 创建 Docker 网络（如果不存在），失败时给出警告但不退出
pub async fn create_network_if_missing(network_name: &str) -> Result<bool> {
    if network_exists(network_name).await? {
        return Ok(true);
    }

    println!("   {} Creating network '{}'...", "Info:".cyan(), network_name);

    let create_output = Command::new("docker")
        .arg("network")
        .arg("create")
        .arg("--driver")
        .arg("bridge")
        .arg(network_name)
        .output()
        .await
        .context("Failed to create network")?;

    if !create_output.status.success() {
        let error_msg = String::from_utf8_lossy(&create_output.stderr);
        // 输出警告但不退出，让 Docker Compose 尝试创建
        println!(
            "{} Failed to create network '{}': {}. Docker Compose will try to create it.",
            "Warning:".yellow().bold(),
            network_name,
            error_msg
        );
        return Ok(false);
    }

    println!("   {} Network '{}' created successfully", "Success:".green(), network_name);
    Ok(true)
}

#[allow(dead_code)]
/// 删除 Docker 网络（如果存在）
pub async fn remove_network(network_name: &str) -> Result<bool> {
    if !network_exists(network_name).await? {
        return Ok(false);
    }

    println!("   {} Removing network '{}'...", "Info:".cyan(), network_name);

    let remove_output = Command::new("docker")
        .arg("network")
        .arg("rm")
        .arg(network_name)
        .output()
        .await
        .context("Failed to remove network")?;

    if !remove_output.status.success() {
        let error_msg = String::from_utf8_lossy(&remove_output.stderr);
        println!(
            "{} Failed to remove network '{}': {}",
            "Warning:".yellow().bold(),
            network_name,
            error_msg
        );
        return Ok(false);
    }

    println!("   {} Network '{}' removed successfully", "Success:".green(), network_name);
    Ok(true)
}

#[derive(Serialize)]
struct ClusterNode {
    id: u32,
    name: String,
    port: u32,
    raft_port: u32,
    is_master: bool,
    master_id: Option<u32>,
}

/// Docker 资源限制 (用于模板，标准化测试时可复现 CPU/内存)
#[derive(Serialize)]
struct DockerResourceLimits {
    cpus: String,
    memory: String,
}

/// 生成动态配置文件(docker-compose.yaml 和节点配置)
///
/// 当 `resource_limits` 为 `Some((cpus, memory))` 时，生成的 compose 会包含 deploy.resources.limits，用于可复现的标准化测试。
pub fn generate_dynamic_configs(
    run_dir: &Path,
    image: &str,
    nodes_count: Option<u32>,
    shards: Option<u32>,
    replicas: Option<u32>,
    network_external: bool,
    resource_limits: Option<(String, String)>,
) -> Result<()> {
    let mut nodes = Vec::new();

    if let Some(count) = nodes_count {
        // 模式 A: 纯节点模式, 仅启动 N 个节点而不进行拓扑划分
        for i in 1..=count {
            nodes.push(ClusterNode {
                id: i,
                name: format!("aikv{}", i),
                port: DOCKER_PORT_BASE as u32 + i,
                raft_port: RAFT_PORT_BASE as u32 + i,
                is_master: true,
                master_id: None,
            });
        }
    } else {
        // 模式 B: 集群拓扑模式 (Shards + Replicas)
        let s_count = shards.unwrap_or(DEFAULT_SHARDS);
        let r_count = replicas.unwrap_or(DEFAULT_REPLICAS);
        let mut current_node_id = 0;

        for _ in 0..s_count {
            current_node_id += 1;
            let master_idx = current_node_id;
            nodes.push(ClusterNode {
                id: master_idx,
                name: format!("aikv{}", master_idx),
                port: DOCKER_PORT_BASE as u32 + master_idx,
                raft_port: RAFT_PORT_BASE as u32 + master_idx,
                is_master: true,
                master_id: None,
            });

            for _ in 0..r_count {
                current_node_id += 1;
                nodes.push(ClusterNode {
                    id: current_node_id,
                    name: format!("aikv{}", current_node_id),
                    port: DOCKER_PORT_BASE as u32 + current_node_id,
                    raft_port: RAFT_PORT_BASE as u32 + current_node_id,
                    is_master: false,
                    master_id: Some(master_idx),
                });
            }
        }
    }

    let mut tera = Tera::default();

    // 从 constants 中获取编译时嵌入的模板
    let compose_tmpl = templates::DOCKER_COMPOSE_J2;
    let config_tmpl = templates::AIKV_CONFIG_J2;

    let mut context = TeraContext::new();
    context.insert("nodes", &nodes);
    context.insert("image", image);
    context.insert("network_external", &network_external);
    if let Some((cpus, memory)) = resource_limits {
        context.insert(
            "resource_limits",
            &DockerResourceLimits { cpus, memory },
        );
    }

    // 生成 docker-compose.yaml
    let compose_content = tera
        .render_str(compose_tmpl, &context)
        .context("failed to render Docker Compose template")?;
    std::fs::write(run_dir.join(crate::constants::files::DOCKER_COMPOSE), compose_content)?;

    // 生成每个节点的配置文件
    let config_dir = run_dir.join(crate::constants::dirs::CONFIG);
    std::fs::create_dir_all(&config_dir)?;

    for node in nodes {
        let mut node_ctx = TeraContext::new();
        node_ctx.insert("node", &node);
        let config_content = tera
            .render_str(config_tmpl, &node_ctx)
            .context(format!("failed to render config for node {}", node.name))?;
        std::fs::write(config_dir.join(format!("{}.toml", node.name)), config_content)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── ClusterNode 生成逻辑 ────────────────────────────

    #[test]
    fn generate_single_node() {
        let tmp_dir = tempfile::tempdir().unwrap();
        generate_dynamic_configs(tmp_dir.path(), "aikv:test", Some(1), None, None, false, None).unwrap();

        // 验证 docker-compose.yaml 生成
        let compose_path = tmp_dir
            .path()
            .join(crate::constants::files::DOCKER_COMPOSE);
        assert!(compose_path.exists());

        let compose_content = std::fs::read_to_string(&compose_path).unwrap();
        assert!(compose_content.contains("aikv1"));
        assert!(compose_content.contains("aikv:test"));
        // 单节点 id=1, 端口 = DOCKER_PORT_BASE + 1 = 6379
        assert!(compose_content.contains("6379"));

        // 验证节点配置生成
        let config_dir = tmp_dir.path().join(crate::constants::dirs::CONFIG);
        assert!(config_dir.exists());
        let node_config = config_dir.join("aikv1.toml");
        assert!(node_config.exists());

        let node_content = std::fs::read_to_string(&node_config).unwrap();
        assert!(node_content.contains("port = 6379"));
        assert!(node_content.contains("is_bootstrap = true"));
    }

    #[test]
    fn generate_pure_nodes_mode() {
        let tmp_dir = tempfile::tempdir().unwrap();
        generate_dynamic_configs(tmp_dir.path(), "aikv:dev", Some(3), None, None, false, None).unwrap();

        let compose_content = std::fs::read_to_string(
            tmp_dir
                .path()
                .join(crate::constants::files::DOCKER_COMPOSE),
        )
        .unwrap();

        // 3 个节点
        assert!(compose_content.contains("aikv1"));
        assert!(compose_content.contains("aikv2"));
        assert!(compose_content.contains("aikv3"));
        // 不应有第 4 个
        assert!(!compose_content.contains("aikv4"));

        // 验证所有节点配置文件
        let config_dir = tmp_dir.path().join(crate::constants::dirs::CONFIG);
        assert!(config_dir.join("aikv1.toml").exists());
        assert!(config_dir.join("aikv2.toml").exists());
        assert!(config_dir.join("aikv3.toml").exists());
        assert!(!config_dir.join("aikv4.toml").exists());
    }

    #[test]
    fn generate_cluster_shards_no_replicas() {
        let tmp_dir = tempfile::tempdir().unwrap();
        // 3 分片, 0 副本 → 3 个 master 节点
        generate_dynamic_configs(tmp_dir.path(), "aikv:latest", None, Some(3), Some(0), false, None).unwrap();

        let config_dir = tmp_dir.path().join(crate::constants::dirs::CONFIG);
        assert!(config_dir.join("aikv1.toml").exists());
        assert!(config_dir.join("aikv2.toml").exists());
        assert!(config_dir.join("aikv3.toml").exists());
        assert!(!config_dir.join("aikv4.toml").exists());

        // 所有节点都应是 master (is_bootstrap 检查只针对 id==1)
        let node1 = std::fs::read_to_string(config_dir.join("aikv1.toml")).unwrap();
        assert!(node1.contains("is_bootstrap = true"));

        let node2 = std::fs::read_to_string(config_dir.join("aikv2.toml")).unwrap();
        assert!(node2.contains("is_bootstrap = false"));
    }

    #[test]
    fn generate_cluster_with_replicas() {
        let tmp_dir = tempfile::tempdir().unwrap();
        // 2 分片, 1 副本 → 4 个节点 (2 master + 2 slave)
        generate_dynamic_configs(tmp_dir.path(), "aikv:latest", None, Some(2), Some(1), false, None).unwrap();

        let config_dir = tmp_dir.path().join(crate::constants::dirs::CONFIG);
        assert!(config_dir.join("aikv1.toml").exists()); // master 1
        assert!(config_dir.join("aikv2.toml").exists()); // slave of master 1
        assert!(config_dir.join("aikv3.toml").exists()); // master 2
        assert!(config_dir.join("aikv4.toml").exists()); // slave of master 2
        assert!(!config_dir.join("aikv5.toml").exists());

        // 验证 docker-compose 包含所有节点
        let compose_content = std::fs::read_to_string(
            tmp_dir
                .path()
                .join(crate::constants::files::DOCKER_COMPOSE),
        )
        .unwrap();
        assert!(compose_content.contains("aikv1"));
        assert!(compose_content.contains("aikv4"));

        // 验证端口递增
        let port_base = DOCKER_PORT_BASE as u32;
        assert!(compose_content.contains(&format!("{}", port_base + 1))); // 6379
        assert!(compose_content.contains(&format!("{}", port_base + 4))); // 6382
    }

    #[test]
    fn generate_cluster_3_shards_2_replicas() {
        let tmp_dir = tempfile::tempdir().unwrap();
        // 3 分片, 2 副本 → 9 个节点
        generate_dynamic_configs(tmp_dir.path(), "aikv:latest", None, Some(3), Some(2), false, None).unwrap();

        let config_dir = tmp_dir.path().join(crate::constants::dirs::CONFIG);
        for i in 1..=9 {
            assert!(
                config_dir.join(format!("aikv{}.toml", i)).exists(),
                "aikv{}.toml 应该存在",
                i
            );
        }
        assert!(!config_dir.join("aikv10.toml").exists());
    }

    #[test]
    fn compose_yaml_has_required_sections() {
        let tmp_dir = tempfile::tempdir().unwrap();
        generate_dynamic_configs(tmp_dir.path(), "aikv:test", Some(2), None, None, false, None).unwrap();

        let content = std::fs::read_to_string(
            tmp_dir
                .path()
                .join(crate::constants::files::DOCKER_COMPOSE),
        )
        .unwrap();

        // docker-compose 应包含必要的顶层 key
        assert!(content.contains("services:"));
        assert!(content.contains("volumes:"));
        assert!(content.contains("networks:"));
        assert!(content.contains("aikv"));
    }

    #[test]
    fn compose_yaml_includes_resource_limits_when_provided() {
        let tmp_dir = tempfile::tempdir().unwrap();
        generate_dynamic_configs(
            tmp_dir.path(),
            "aikv:test",
            Some(1),
            None,
            None,
            false,
            Some(("2".to_string(), "1G".to_string())),
        )
        .unwrap();

        let content = std::fs::read_to_string(
            tmp_dir
                .path()
                .join(crate::constants::files::DOCKER_COMPOSE),
        )
        .unwrap();

        assert!(content.contains("deploy:"));
        assert!(content.contains("resources:"));
        assert!(content.contains("cpus: \"2\""));
        assert!(content.contains("memory: \"1G\""));
    }

    #[test]
    fn compose_yaml_network_external_true() {
        let tmp_dir = tempfile::tempdir().unwrap();
        generate_dynamic_configs(tmp_dir.path(), "aikv:test", Some(1), None, None, true, None).unwrap();

        let content = std::fs::read_to_string(
            tmp_dir
                .path()
                .join(crate::constants::files::DOCKER_COMPOSE),
        )
        .unwrap();

        // external: true 时应包含 external: true
        assert!(content.contains("external: true"));
        // 不应包含 driver 或 name（创建模式的字段）
        assert!(!content.contains("driver: bridge"));
    }

    #[test]
    fn compose_yaml_network_external_false() {
        let tmp_dir = tempfile::tempdir().unwrap();
        generate_dynamic_configs(tmp_dir.path(), "aikv:test", Some(1), None, None, false, None).unwrap();

        let content = std::fs::read_to_string(
            tmp_dir
                .path()
                .join(crate::constants::files::DOCKER_COMPOSE),
        )
        .unwrap();

        // 非 external 时应包含网络定义
        assert!(content.contains("aikv"));
        assert!(content.contains("driver: bridge"));
        assert!(content.contains("name: aikv"));
    }

    #[test]
    fn node_config_has_required_sections() {
        let tmp_dir = tempfile::tempdir().unwrap();
        generate_dynamic_configs(tmp_dir.path(), "aikv:test", Some(1), None, None, false, None).unwrap();

        let content = std::fs::read_to_string(
            tmp_dir
                .path()
                .join(crate::constants::dirs::CONFIG)
                .join("aikv1.toml"),
        )
        .unwrap();

        assert!(content.contains("[server]"));
        assert!(content.contains("[cluster]"));
        assert!(content.contains("[storage]"));
        assert!(content.contains("[logging]"));
        assert!(content.contains("host = \"0.0.0.0\""));
        assert!(content.contains("engine = \"aidb\""));
    }

    #[test]
    fn compose_volumes_match_nodes() {
        let tmp_dir = tempfile::tempdir().unwrap();
        generate_dynamic_configs(tmp_dir.path(), "aikv:latest", Some(3), None, None, false, None).unwrap();

        let content = std::fs::read_to_string(
            tmp_dir
                .path()
                .join(crate::constants::files::DOCKER_COMPOSE),
        )
        .unwrap();

        // 每个节点应有 data 和 logs 卷
        for i in 1..=3 {
            assert!(
                content.contains(&format!("aikv{}-data", i)),
                "应包含 aikv{}-data 卷",
                i
            );
            assert!(
                content.contains(&format!("aikv{}-logs", i)),
                "应包含 aikv{}-logs 卷",
                i
            );
        }
    }

    #[test]
    fn raft_ports_are_set() {
        let tmp_dir = tempfile::tempdir().unwrap();
        generate_dynamic_configs(tmp_dir.path(), "aikv:latest", Some(2), None, None, false, None).unwrap();

        let node1 = std::fs::read_to_string(
            tmp_dir
                .path()
                .join(crate::constants::dirs::CONFIG)
                .join("aikv1.toml"),
        )
        .unwrap();

        let expected_raft_port = RAFT_PORT_BASE as u32 + 1;
        assert!(
            node1.contains(&format!("aikv1:{}", expected_raft_port)),
            "节点 1 应有正确的 raft 地址"
        );
    }
}
