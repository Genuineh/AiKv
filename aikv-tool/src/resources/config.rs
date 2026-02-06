//! 配置模块
//!
//! 包含配置数据模型, 类型安全枚举, 以及配置的加载与保存.
//!
//! ## 设计原则
//!
//! - **类型安全**: `RunMode` / `Topology` 为枚举, 消除字符串匹配.
//! - **扁平配置**: 仅 `[project]` + `[defaults]` 两个 section, 参考 Docker/kubectl 风格.
//!
//! ## 加载优先级
//!
//! 1. 本地项目配置 (从 CWD 向上查找 `ak.toml`)
//! 2. XDG 全局配置 (`~/.config/ak/ak.toml`)
//! 3. 内置默认值

use anyhow::{anyhow, bail, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::paths;

/// 当前配置 schema 版本
pub const SCHEMA_VERSION: u32 = 1;

// ─── 类型安全枚举 ─────────────────────────────────────────

/// 运行模式
#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    /// 本地二进制进程
    #[default]
    Bin,
    /// Docker 容器
    Docker,
}

impl std::fmt::Display for RunMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bin => write!(f, "bin"),
            Self::Docker => write!(f, "docker"),
        }
    }
}

impl std::str::FromStr for RunMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bin" => Ok(Self::Bin),
            "docker" => Ok(Self::Docker),
            _ => Err(format!("only 'bin' or 'docker' supported, got: '{s}'")),
        }
    }
}

/// 部署拓扑
#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Topology {
    /// 单节点
    #[default]
    Single,
    /// 集群
    Cluster,
}

impl Topology {
    pub fn is_cluster(&self) -> bool {
        *self == Self::Cluster
    }
}

impl std::fmt::Display for Topology {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Single => write!(f, "single"),
            Self::Cluster => write!(f, "cluster"),
        }
    }
}

impl std::str::FromStr for Topology {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "single" => Ok(Self::Single),
            "cluster" => Ok(Self::Cluster),
            _ => Err(format!("only 'single' or 'cluster' supported, got: '{s}'")),
        }
    }
}

// ─── 配置结构 ─────────────────────────────────────────────

/// AiKv 工具配置 (对应 ak.toml)
///
/// ```toml
/// schema_version = 1
///
/// [project]
/// root = "/path/to/aikv"
///
/// [defaults]
/// mode = "bin"
/// topo = "single"
/// port = 6379
/// docker_image = "aikv:latest"
/// ```
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AkConfig {
    /// 配置 schema 版本号
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    /// 项目配置
    #[serde(default)]
    pub project: ProjectConfig,

    /// 运行时默认值
    #[serde(default)]
    pub defaults: DefaultsConfig,

    /// 配置文件来源 (运行时元数据, 不序列化)
    #[serde(skip)]
    pub source: Option<PathBuf>,
}

/// 项目配置 `[project]`
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ProjectConfig {
    /// AiKv 项目根目录
    pub root: Option<PathBuf>,
}

/// 运行时默认值 `[defaults]`
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DefaultsConfig {
    /// 运行模式 (bin | docker)
    #[serde(default)]
    pub mode: RunMode,

    /// 部署拓扑 (single | cluster)
    #[serde(default)]
    pub topo: Topology,

    /// 服务端口
    #[serde(default = "default_port")]
    pub port: u16,

    /// Docker 镜像
    #[serde(default = "default_docker_image")]
    pub docker_image: String,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            mode: RunMode::default(),
            topo: Topology::default(),
            port: default_port(),
            docker_image: default_docker_image(),
        }
    }
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

fn default_port() -> u16 {
    crate::constants::DEFAULT_PORT
}

fn default_docker_image() -> String {
    crate::constants::DEFAULT_DOCKER_IMAGE.to_string()
}

impl Default for AkConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            project: ProjectConfig::default(),
            defaults: DefaultsConfig::default(),
            source: None,
        }
    }
}

// ─── 加载 / 保存 ─────────────────────────────────────────

impl AkConfig {
    /// 加载配置
    ///
    /// 优先级: 本地 ak.toml > 全局 ~/.config/ak/ak.toml > 默认值
    pub fn load() -> Result<Self> {
        // 1. 本地项目配置优先
        if let Some(path) = paths::find_local_config() {
            return Self::load_from_file(&path);
        }

        // 2. 全局配置
        if let Some(path) = paths::config_path() {
            if path.exists() {
                return Self::load_from_file(&path);
            }
        }

        // 3. 无配置文件 → 使用默认值
        Ok(Self::default())
    }

    /// 从指定文件加载配置
    fn load_from_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let mut config: Self =
            toml::from_str(&content).map_err(|e| anyhow!("config parse error: {e}\npath: {path:?}"))?;

        config.source = Some(path.to_path_buf());

        // schema 版本校验 (仅警告, 不阻断)
        if config.schema_version != SCHEMA_VERSION {
            eprintln!(
                "Config version mismatch (file: v{}, current: v{}). Run `ak config sync` to upgrade.",
                config.schema_version,
                SCHEMA_VERSION
            );
        }

        // 相对路径 → 基于配置文件所在目录解析为绝对路径
        if let Some(ref mut root) = config.project.root {
            if root.is_relative() {
                if let Some(parent) = path.parent() {
                    let abs = parent
                        .join(&*root)
                        .canonicalize()
                        .unwrap_or_else(|_| parent.join(&*root));
                    *root = abs;
                }
            }
        }

        Ok(config)
    }

    /// 保存配置到磁盘
    pub fn save(&self) -> Result<()> {
        let path = self
            .source
            .clone()
            .or_else(paths::config_path)
            .ok_or_else(|| anyhow!("could not determine save path"))?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // 写入时始终使用最新 schema 版本
        let mut to_save = self.clone();
        to_save.schema_version = SCHEMA_VERSION;

        let content = toml::to_string_pretty(&to_save)?;
        fs::write(&path, content)?;

        println!("Config saved to {:?}", path);
        Ok(())
    }
}

// ─── 字段操作 ─────────────────────────────────────────────

impl AkConfig {
    /// 探测并验证项目根目录
    pub fn detect_project_root(&self) -> Result<PathBuf> {
        let root = self.project.root.clone().ok_or_else(|| {
            anyhow!(
                "project root not set\n\n\
                 To fix:\n\
                 1. Add ak.toml in your AiKv project dir, or\n\
                 2. Run: ak config set project.root=/path/to/aikv"
            )
        })?;

        if root.exists() {
            Ok(root)
        } else {
            Err(anyhow!(
                "project root path does not exist: {:?}\n\n\
                 Run: ak config set project.root=/correct/path",
                root
            ))
        }
    }

    /// 设置配置项 (统一的 key=value 入口)
    ///
    /// 支持完整路径 (`defaults.mode`) 和简写 (`mode`).
    pub fn set_field(&mut self, key: &str, value: &str) -> Result<String> {
        match key {
            "project.root" | "root" => {
                let path = PathBuf::from(value);
                let resolved = if path.is_absolute() {
                    path
                } else {
                    std::env::current_dir()?.join(path).canonicalize()?
                };
                self.project.root = Some(resolved.clone());
                Ok(format!("project.root = {:?}", resolved))
            }
            "defaults.mode" | "mode" => {
                let mode: RunMode = value.parse().map_err(|e: String| anyhow!(e))?;
                self.defaults.mode = mode;
                Ok(format!("defaults.mode = {}", mode))
            }
            "defaults.topo" | "topo" => {
                let topo: Topology = value.parse().map_err(|e: String| anyhow!(e))?;
                self.defaults.topo = topo;
                Ok(format!("defaults.topo = {}", topo))
            }
            "defaults.port" | "port" => {
                let port: u16 = value.parse()?;
                self.defaults.port = port;
                Ok(format!("defaults.port = {}", port))
            }
            "defaults.docker_image" | "docker_image" | "image" => {
                self.defaults.docker_image = value.to_string();
                Ok(format!("defaults.docker_image = {}", value))
            }
            _ => bail!(
                "unknown config key: '{}'\n\nAvailable keys:\n  {}",
                key,
                Self::known_keys().join(", ")
            ),
        }
    }

    /// 所有可用的配置项名称
    pub fn known_keys() -> &'static [&'static str] {
        &[
            "project.root",
            "defaults.mode",
            "defaults.topo",
            "defaults.port",
            "defaults.docker_image",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrip() {
        let config = AkConfig::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: AkConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.schema_version, SCHEMA_VERSION);
        assert_eq!(deserialized.defaults.mode, RunMode::Bin);
        assert_eq!(deserialized.defaults.topo, Topology::Single);
        assert_eq!(deserialized.defaults.port, crate::constants::DEFAULT_PORT);
    }

    #[test]
    fn set_field_mode() {
        let mut config = AkConfig::default();
        config.set_field("mode", "docker").unwrap();
        assert_eq!(config.defaults.mode, RunMode::Docker);
    }

    #[test]
    fn set_field_invalid_mode() {
        let mut config = AkConfig::default();
        assert!(config.set_field("mode", "invalid").is_err());
    }

    #[test]
    fn set_field_topo() {
        let mut config = AkConfig::default();
        config.set_field("topo", "cluster").unwrap();
        assert_eq!(config.defaults.topo, Topology::Cluster);
        assert!(config.defaults.topo.is_cluster());
    }

    #[test]
    fn set_field_port() {
        let mut config = AkConfig::default();
        config.set_field("port", "8080").unwrap();
        assert_eq!(config.defaults.port, 8080);
    }

    #[test]
    fn set_field_unknown_key() {
        let mut config = AkConfig::default();
        assert!(config.set_field("nonexistent", "value").is_err());
    }

    #[test]
    fn run_mode_display_and_parse() {
        assert_eq!(RunMode::Bin.to_string(), "bin");
        assert_eq!(RunMode::Docker.to_string(), "docker");
        assert_eq!("bin".parse::<RunMode>().unwrap(), RunMode::Bin);
        assert_eq!("docker".parse::<RunMode>().unwrap(), RunMode::Docker);
        assert!("invalid".parse::<RunMode>().is_err());
    }

    #[test]
    fn topology_display_and_parse() {
        assert_eq!(Topology::Single.to_string(), "single");
        assert_eq!(Topology::Cluster.to_string(), "cluster");
        assert_eq!("single".parse::<Topology>().unwrap(), Topology::Single);
        assert_eq!("cluster".parse::<Topology>().unwrap(), Topology::Cluster);
        assert!("invalid".parse::<Topology>().is_err());
    }

    // ─── 枚举默认值 ──────────────────────────────────────

    #[test]
    fn run_mode_default_is_bin() {
        assert_eq!(RunMode::default(), RunMode::Bin);
    }

    #[test]
    fn topology_default_is_single() {
        assert_eq!(Topology::default(), Topology::Single);
    }

    #[test]
    fn topology_is_cluster() {
        assert!(!Topology::Single.is_cluster());
        assert!(Topology::Cluster.is_cluster());
    }

    // ─── 大小写不敏感解析 ──────────────────────────────────

    #[test]
    fn run_mode_parse_case_insensitive() {
        assert_eq!("BIN".parse::<RunMode>().unwrap(), RunMode::Bin);
        assert_eq!("Bin".parse::<RunMode>().unwrap(), RunMode::Bin);
        assert_eq!("DOCKER".parse::<RunMode>().unwrap(), RunMode::Docker);
        assert_eq!("Docker".parse::<RunMode>().unwrap(), RunMode::Docker);
    }

    #[test]
    fn topology_parse_case_insensitive() {
        assert_eq!("SINGLE".parse::<Topology>().unwrap(), Topology::Single);
        assert_eq!("Single".parse::<Topology>().unwrap(), Topology::Single);
        assert_eq!("CLUSTER".parse::<Topology>().unwrap(), Topology::Cluster);
        assert_eq!("Cluster".parse::<Topology>().unwrap(), Topology::Cluster);
    }

    // ─── set_field 完整路径和简写 ────────────────────────────

    #[test]
    fn set_field_full_path_mode() {
        let mut config = AkConfig::default();
        config.set_field("defaults.mode", "docker").unwrap();
        assert_eq!(config.defaults.mode, RunMode::Docker);
    }

    #[test]
    fn set_field_full_path_topo() {
        let mut config = AkConfig::default();
        config.set_field("defaults.topo", "cluster").unwrap();
        assert_eq!(config.defaults.topo, Topology::Cluster);
    }

    #[test]
    fn set_field_full_path_port() {
        let mut config = AkConfig::default();
        config.set_field("defaults.port", "9999").unwrap();
        assert_eq!(config.defaults.port, 9999);
    }

    #[test]
    fn set_field_docker_image_aliases() {
        let mut config = AkConfig::default();

        config.set_field("docker_image", "test:v1").unwrap();
        assert_eq!(config.defaults.docker_image, "test:v1");

        config.set_field("image", "test:v2").unwrap();
        assert_eq!(config.defaults.docker_image, "test:v2");

        config
            .set_field("defaults.docker_image", "test:v3")
            .unwrap();
        assert_eq!(config.defaults.docker_image, "test:v3");
    }

    #[test]
    fn set_field_invalid_port() {
        let mut config = AkConfig::default();
        assert!(config.set_field("port", "not_a_number").is_err());
    }

    #[test]
    fn set_field_invalid_topo() {
        let mut config = AkConfig::default();
        assert!(config.set_field("topo", "distributed").is_err());
    }

    // ─── known_keys ─────────────────────────────────────

    #[test]
    fn known_keys_contains_expected() {
        let keys = AkConfig::known_keys();
        assert!(keys.contains(&"project.root"));
        assert!(keys.contains(&"defaults.mode"));
        assert!(keys.contains(&"defaults.topo"));
        assert!(keys.contains(&"defaults.port"));
        assert!(keys.contains(&"defaults.docker_image"));
    }

    // ─── 默认配置结构 ──────────────────────────────────────

    #[test]
    fn default_config_values() {
        let config = AkConfig::default();
        assert_eq!(config.schema_version, SCHEMA_VERSION);
        assert!(config.project.root.is_none());
        assert_eq!(config.defaults.mode, RunMode::Bin);
        assert_eq!(config.defaults.topo, Topology::Single);
        assert_eq!(config.defaults.port, crate::constants::DEFAULT_PORT);
        assert_eq!(
            config.defaults.docker_image,
            crate::constants::DEFAULT_DOCKER_IMAGE
        );
        assert!(config.source.is_none());
    }

    // ─── detect_project_root ────────────────────────────

    #[test]
    fn detect_project_root_not_configured() {
        let config = AkConfig::default();
        let result = config.detect_project_root();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("project root not set"));
    }

    #[test]
    fn detect_project_root_path_not_exists() {
        let mut config = AkConfig::default();
        config.project.root = Some(PathBuf::from("/nonexistent/path/that/does/not/exist"));
        let result = config.detect_project_root();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("project root path does not exist"));
    }

    #[test]
    fn detect_project_root_exists() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let mut config = AkConfig::default();
        config.project.root = Some(tmp_dir.path().to_path_buf());
        let result = config.detect_project_root();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), tmp_dir.path());
    }

    // ─── 文件 I/O ──────────────────────────────────────────

    #[test]
    fn load_from_valid_toml_file() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config_path = tmp_dir.path().join("ak.toml");
        std::fs::write(
            &config_path,
            r#"
schema_version = 1

[project]
root = "/tmp/aikv"

[defaults]
mode = "docker"
topo = "cluster"
port = 8080
docker_image = "aikv:dev"
"#,
        )
        .unwrap();

        let config = AkConfig::load_from_file(&config_path).unwrap();
        assert_eq!(config.schema_version, 1);
        assert_eq!(config.defaults.mode, RunMode::Docker);
        assert_eq!(config.defaults.topo, Topology::Cluster);
        assert_eq!(config.defaults.port, 8080);
        assert_eq!(config.defaults.docker_image, "aikv:dev");
        assert!(config.source.is_some());
    }

    #[test]
    fn load_from_partial_toml_fills_defaults() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config_path = tmp_dir.path().join("ak.toml");
        // 只指定 mode, 其他用默认值
        std::fs::write(&config_path, "[defaults]\nmode = \"docker\"\n").unwrap();

        let config = AkConfig::load_from_file(&config_path).unwrap();
        assert_eq!(config.defaults.mode, RunMode::Docker);
        assert_eq!(config.defaults.topo, Topology::Single); // 默认值
        assert_eq!(config.defaults.port, crate::constants::DEFAULT_PORT); // 默认值
    }

    #[test]
    fn load_from_empty_toml_uses_all_defaults() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config_path = tmp_dir.path().join("ak.toml");
        std::fs::write(&config_path, "").unwrap();

        let config = AkConfig::load_from_file(&config_path).unwrap();
        assert_eq!(config.defaults.mode, RunMode::Bin);
        assert_eq!(config.defaults.topo, Topology::Single);
        assert_eq!(config.defaults.port, crate::constants::DEFAULT_PORT);
    }

    #[test]
    fn load_from_invalid_toml_returns_error() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config_path = tmp_dir.path().join("ak.toml");
        std::fs::write(&config_path, "this is { not valid toml !!!").unwrap();

        let result = AkConfig::load_from_file(&config_path);
        assert!(result.is_err());
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config_path = tmp_dir.path().join("ak.toml");

        let mut config = AkConfig::default();
        config.source = Some(config_path.clone());
        config.defaults.mode = RunMode::Docker;
        config.defaults.topo = Topology::Cluster;
        config.defaults.port = 7777;
        config.defaults.docker_image = "aikv:test".to_string();

        config.save().unwrap();

        // 重新加载
        let reloaded = AkConfig::load_from_file(&config_path).unwrap();
        assert_eq!(reloaded.defaults.mode, RunMode::Docker);
        assert_eq!(reloaded.defaults.topo, Topology::Cluster);
        assert_eq!(reloaded.defaults.port, 7777);
        assert_eq!(reloaded.defaults.docker_image, "aikv:test");
        assert_eq!(reloaded.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn save_creates_parent_directories() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let nested_path = tmp_dir.path().join("a").join("b").join("c").join("ak.toml");

        let mut config = AkConfig::default();
        config.source = Some(nested_path.clone());
        config.save().unwrap();

        assert!(nested_path.exists());
    }

    // ─── TOML 序列化格式 ──────────────────────────────────

    #[test]
    fn toml_serialization_uses_lowercase_enums() {
        let config = AkConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("mode = \"bin\""));
        assert!(toml_str.contains("topo = \"single\""));
    }

    #[test]
    fn toml_deserialization_from_full_config() {
        let toml_str = r#"
schema_version = 1

[project]

[defaults]
mode = "bin"
topo = "single"
port = 6379
docker_image = "aikv:latest"
"#;
        let config: AkConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.schema_version, 1);
        assert_eq!(config.defaults.mode, RunMode::Bin);
    }

    // ─── 相对路径解析 ──────────────────────────────────────

    #[test]
    fn load_resolves_relative_project_root() {
        let tmp_dir = tempfile::tempdir().unwrap();
        // 创建子目录, 使相对路径 "." 可以 canonicalize
        let config_path = tmp_dir.path().join("ak.toml");
        std::fs::write(
            &config_path,
            "[project]\nroot = \".\"\n",
        )
        .unwrap();

        let config = AkConfig::load_from_file(&config_path).unwrap();
        // 相对路径应被解析为绝对路径
        if let Some(root) = &config.project.root {
            assert!(root.is_absolute(), "相对路径应被解析为绝对路径");
        }
    }
}
