//! 全局常量定义 - AiKv 工具链.
//!
//! 集中管理所有硬编码值, 便于维护和修改.

// === 应用标识 ===

/// 应用名称 (用于 XDG 目录, 日志前缀等)
pub const APP_NAME: &str = "ak";

/// AiKv 服务名称(预留, 用于未来扩展)
#[allow(dead_code)]
pub const SERVICE_NAME: &str = "aikv";

/// Docker Compose 项目名称
pub const DOCKER_PROJECT_NAME: &str = "aikv";

// === 网络配置 ===

/// 默认服务端口
pub const DEFAULT_PORT: u16 = 6379;

/// Docker 容器端口基数(实际端口 = 基数 + 节点ID)
pub const DOCKER_PORT_BASE: u16 = 6378;

/// Raft 端口基数
pub const RAFT_PORT_BASE: u16 = 50050;

/// Docker 网络名称 (用于连接 AiKv 和 OTel 观测栈)
pub const NETWORK_NAME: &str = "aikv";

// === Docker 配置 ===

/// 默认 Docker 镜像
pub const DEFAULT_DOCKER_IMAGE: &str = "aikv:latest";

// === 集群配置 ===

/// 默认分片数
pub const DEFAULT_SHARDS: u32 = 3;

/// 默认副本数
pub const DEFAULT_REPLICAS: u32 = 0;

// === XDG 基础规范 ===

/// XDG 环境变量名与默认路径片段
///
/// 遵循 [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html): 
/// - `XDG_CONFIG_HOME`: 用户配置文件(默认 ~/.config)
/// - `XDG_DATA_HOME`: 用户数据文件(默认 ~/.local/share)
/// - `XDG_STATE_HOME`: 用户状态文件(默认 ~/.local/state)
/// - `XDG_CACHE_HOME`: 用户缓存文件(默认 ~/.cache)
/// - `XDG_RUNTIME_DIR`: 运行时文件(通常 /run/user/$UID)
pub mod xdg {
    /// XDG_STATE_HOME 环境变量名
    pub const ENV_STATE_HOME: &str = "XDG_STATE_HOME";
    /// XDG_CONFIG_HOME 环境变量名(预留)
    #[allow(dead_code)]
    pub const ENV_CONFIG_HOME: &str = "XDG_CONFIG_HOME";
    /// XDG_CACHE_HOME 环境变量名(预留)
    #[allow(dead_code)]
    pub const ENV_CACHE_HOME: &str = "XDG_CACHE_HOME";

    /// XDG_STATE_HOME 的默认相对路径片段
    pub const DEFAULT_STATE_SUBPATH: &[&str] = &[".local", "state"];
    /// XDG_DATA_HOME 的默认相对路径片段(预留)
    #[allow(dead_code)]
    pub const DEFAULT_DATA_SUBPATH: &[&str] = &[".local", "share"];
}

// === 应用目录结构 ===

/// 应用内部目录结构常量
pub mod dirs {
    /// 单节点模式子目录
    pub const SINGLE: &str = "single";
    /// 集群模式子目录
    pub const CLUSTER: &str = "cluster";
    /// 运行时状态子目录(存放 PID, 动态配置等)
    pub const RUN: &str = "run";
    /// 日志子目录
    pub const LOGS: &str = "logs";
    /// 配置子目录(Docker 节点配置等)
    pub const CONFIG: &str = "config";
}


// === 文件名 ===

/// 配置, 日志, PID, Compose 等文件名常量
pub mod files {
    /// ak 工具配置文件
    pub const AK_CONFIG: &str = "ak.toml";
    /// aikv 服务配置文件
    pub const AIKV_CONFIG: &str = "aikv.toml";
    /// PID 文件
    pub const PID_FILE: &str = "aikv.pid";
    /// aikv 服务日志
    pub const AIKV_LOG: &str = "aikv.log";
    /// ak 工具日志
    pub const AK_LOG: &str = "ak.log";
    /// Docker Compose 文件
    pub const DOCKER_COMPOSE: &str = "docker-compose.yaml";
}

// === 模板文件(编译时嵌入) ===

pub mod templates {
    /// Docker Compose 模板(编译时嵌入)
    pub const DOCKER_COMPOSE_J2: &str = include_str!("../templates/docker-compose.yaml.j2");
    /// AiKv 节点配置模板(编译时嵌入)
    pub const AIKV_CONFIG_J2: &str = include_str!("../templates/aikv.toml.j2");
}

// === 显示文本 ===

pub mod display {
    /// Single-node mode display name
    pub const MODE_SINGLE: &str = "Single";
    /// Cluster mode display name
    pub const MODE_CLUSTER: &str = "Cluster";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_ak() {
        assert_eq!(APP_NAME, "ak");
    }

    #[test]
    fn default_port_is_6379() {
        assert_eq!(DEFAULT_PORT, 6379);
    }

    #[test]
    fn docker_port_base_less_than_default() {
        // 确保端口基数 + 1 == 默认端口 (6378 + 1 = 6379)
        assert_eq!(DOCKER_PORT_BASE + 1, DEFAULT_PORT);
    }

    #[test]
    fn default_docker_image_not_empty() {
        assert!(!DEFAULT_DOCKER_IMAGE.is_empty());
        assert!(DEFAULT_DOCKER_IMAGE.contains(':'), "镜像应包含 tag");
    }

    #[test]
    fn xdg_env_var_names() {
        assert_eq!(xdg::ENV_STATE_HOME, "XDG_STATE_HOME");
        assert_eq!(xdg::ENV_CONFIG_HOME, "XDG_CONFIG_HOME");
        assert_eq!(xdg::ENV_CACHE_HOME, "XDG_CACHE_HOME");
    }

    #[test]
    fn xdg_default_state_subpath() {
        assert_eq!(xdg::DEFAULT_STATE_SUBPATH, &[".local", "state"]);
    }

    #[test]
    fn dir_constants_non_empty() {
        assert!(!dirs::SINGLE.is_empty());
        assert!(!dirs::CLUSTER.is_empty());
        assert!(!dirs::RUN.is_empty());
        assert!(!dirs::LOGS.is_empty());
        assert!(!dirs::CONFIG.is_empty());
    }

    #[test]
    fn file_constants_have_extensions() {
        assert!(files::AK_CONFIG.ends_with(".toml"));
        assert!(files::AIKV_CONFIG.ends_with(".toml"));
        assert!(files::PID_FILE.ends_with(".pid"));
        assert!(files::AIKV_LOG.ends_with(".log"));
        assert!(files::AK_LOG.ends_with(".log"));
        assert!(files::DOCKER_COMPOSE.ends_with(".yaml"));
    }

    #[test]
    fn templates_non_empty() {
        assert!(!templates::DOCKER_COMPOSE_J2.is_empty());
        assert!(!templates::AIKV_CONFIG_J2.is_empty());
    }

    #[test]
    fn default_cluster_config() {
        assert!(DEFAULT_SHARDS >= 1, "分片数至少为 1");
        assert_eq!(DEFAULT_REPLICAS, 0, "默认副本数为 0");
    }
}
