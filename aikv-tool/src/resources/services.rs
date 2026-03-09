//! 服务资源
//!
//! 表示 AiKv 服务的运行状态, 可通过 `ak ps` 查询. 

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use sysinfo::{Pid, System};

use crate::constants::{files, DOCKER_PROJECT_NAME};
use crate::paths;
use crate::runtime::docker;
use crate::utils::helpers::get_state_subdir;

/// 服务状态汇总
#[derive(Debug, Serialize, Deserialize)]
pub struct ServicesStatus {
    /// 本地二进制进程状态
    pub bin: Option<BinServiceStatus>,
    /// Docker 容器服务状态列表
    pub docker: Vec<DockerServiceStatus>,
}

/// 本地二进制进程状态
#[derive(Debug, Serialize, Deserialize)]
pub struct BinServiceStatus {
    pub name: String,
    pub status: String,
    pub pid: u32,
    pub memory: String,
    pub uptime: String,
}

/// Docker 容器服务状态
#[derive(Debug, Serialize, Deserialize)]
pub struct DockerServiceStatus {
    pub name: String,
    pub status: String,
    pub ports: String,
}

impl ServicesStatus {
    /// 获取当前服务状态
    pub async fn get(is_cluster: bool) -> Result<Self> {
        let run_dir = paths::run_dir()?;
        let state_subdir = get_state_subdir(is_cluster);

        // 检查本地二进制进程
        let bin = Self::get_bin_status(&run_dir)?;

        // 获取 Docker 容器状态
        let docker = Self::get_docker_status(&run_dir, state_subdir).await?;

        Ok(Self { bin, docker })
    }

    fn get_bin_status(run_dir: &std::path::Path) -> Result<Option<BinServiceStatus>> {
        let pid_path = run_dir.join(files::PID_FILE);
        if !pid_path.exists() {
            return Ok(None);
        }

        let pid_str = fs::read_to_string(&pid_path)?;
        let raw_pid = match pid_str.trim().parse::<u32>() {
            Ok(pid) => pid,
            Err(_) => return Ok(None),
        };

        let mut sys = System::new_all();
        sys.refresh_processes(
            sysinfo::ProcessesToUpdate::Some(&[Pid::from(raw_pid as usize)]),
            true,
        );

        if let Some(process) = sys.process(Pid::from(raw_pid as usize)) {
            return Ok(Some(BinServiceStatus {
                name: "aikv-bin".to_string(),
                status: "Running".to_string(),
                pid: raw_pid,
                memory: format!("{} KB", process.memory() / 1024),
                uptime: format!("{}s", process.run_time()),
            }));
        }

        Ok(None)
    }

    async fn get_docker_status(
        run_dir: &std::path::Path,
        state_subdir: &str,
    ) -> Result<Vec<DockerServiceStatus>> {
        let staged_compose = run_dir
            .join(state_subdir)
            .join(files::DOCKER_COMPOSE);

        if !staged_compose.exists() {
            return Ok(Vec::new());
        }

        let mut cmd = docker::compose_command(DOCKER_PROJECT_NAME, &staged_compose).await?;
        cmd.current_dir(run_dir.join(state_subdir));
        cmd.arg("ps").arg("--format").arg("json");

        let output = match cmd.output().await {
            Ok(o) => o,
            Err(_) => return Ok(Vec::new()),
        };

        let out_str = String::from_utf8_lossy(&output.stdout);
        let mut services = Vec::new();

        for line in out_str.lines() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                services.push(DockerServiceStatus {
                    name: json["Name"].as_str().unwrap_or("-").to_string(),
                    status: json["Status"].as_str().unwrap_or("-").to_string(),
                    ports: json["Ports"].as_str().unwrap_or("-").to_string(),
                });
            }
        }

        Ok(services)
    }

    /// 检查是否有任何服务正在运行
    pub fn is_any_running(&self) -> bool {
        self.bin.is_some() || !self.docker.is_empty()
    }

    /// 检查指定模式下是否有 Docker 服务运行(预留 API)
    #[allow(dead_code)]
    pub async fn is_docker_running(is_cluster: bool) -> Result<bool> {
        let run_dir = paths::run_dir()?;
        let state_subdir = get_state_subdir(is_cluster);
        let compose_file = run_dir.join(state_subdir).join(files::DOCKER_COMPOSE);

        if !compose_file.exists() {
            return Ok(false);
        }

        let mut cmd = docker::compose_command(DOCKER_PROJECT_NAME, &compose_file).await?;
        cmd.arg("ps").arg("--format").arg("json");

        if let Ok(output) = cmd.output().await {
            let out_str = String::from_utf8_lossy(&output.stdout);
            return Ok(!out_str.trim().is_empty() && out_str.trim() != "[]");
        }

        Ok(false)
    }

    /// 检查本地二进制进程是否运行(预留 API)
    #[allow(dead_code)]
    pub fn is_bin_running() -> Result<bool> {
        let run_dir = paths::run_dir()?;
        let pid_path = run_dir.join(files::PID_FILE);

        if !pid_path.exists() {
            return Ok(false);
        }

        let pid_str = fs::read_to_string(&pid_path)?;
        let raw_pid = match pid_str.trim().parse::<u32>() {
            Ok(pid) => pid,
            Err(_) => return Ok(false),
        };

        let mut sys = System::new_all();
        sys.refresh_processes(
            sysinfo::ProcessesToUpdate::Some(&[Pid::from(raw_pid as usize)]),
            true,
        );

        Ok(sys.process(Pid::from(raw_pid as usize)).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_any_running_empty() {
        let status = ServicesStatus {
            bin: None,
            docker: Vec::new(),
        };
        assert!(!status.is_any_running());
    }

    #[test]
    fn is_any_running_with_bin() {
        let status = ServicesStatus {
            bin: Some(BinServiceStatus {
                name: "aikv-bin".to_string(),
                status: "Running".to_string(),
                pid: 12345,
                memory: "1024 KB".to_string(),
                uptime: "100s".to_string(),
            }),
            docker: Vec::new(),
        };
        assert!(status.is_any_running());
    }

    #[test]
    fn is_any_running_with_docker() {
        let status = ServicesStatus {
            bin: None,
            docker: vec![DockerServiceStatus {
                name: "aikv1".to_string(),
                status: "Up 5 minutes".to_string(),
                ports: "0.0.0.0:6379->6379/tcp".to_string(),
            }],
        };
        assert!(status.is_any_running());
    }

    #[test]
    fn is_any_running_with_both() {
        let status = ServicesStatus {
            bin: Some(BinServiceStatus {
                name: "aikv-bin".to_string(),
                status: "Running".to_string(),
                pid: 12345,
                memory: "1024 KB".to_string(),
                uptime: "100s".to_string(),
            }),
            docker: vec![DockerServiceStatus {
                name: "aikv1".to_string(),
                status: "Up 5 minutes".to_string(),
                ports: "0.0.0.0:6379->6379/tcp".to_string(),
            }],
        };
        assert!(status.is_any_running());
    }

    #[test]
    fn services_status_serializable() {
        let status = ServicesStatus {
            bin: Some(BinServiceStatus {
                name: "aikv-bin".to_string(),
                status: "Running".to_string(),
                pid: 42,
                memory: "2048 KB".to_string(),
                uptime: "60s".to_string(),
            }),
            docker: vec![],
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("aikv-bin"));
        assert!(json.contains("42"));

        // 反序列化
        let deserialized: ServicesStatus = serde_json::from_str(&json).unwrap();
        assert!(deserialized.bin.is_some());
        assert_eq!(deserialized.bin.unwrap().pid, 42);
    }

    #[test]
    fn docker_service_status_serializable() {
        let svc = DockerServiceStatus {
            name: "aikv1".to_string(),
            status: "Up 2 hours".to_string(),
            ports: "6379/tcp".to_string(),
        };
        let json = serde_json::to_string(&svc).unwrap();
        let deserialized: DockerServiceStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "aikv1");
        assert_eq!(deserialized.status, "Up 2 hours");
    }
}
