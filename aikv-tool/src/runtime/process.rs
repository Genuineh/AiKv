//! 本地进程管理

use anyhow::Result;
use std::fs;
use std::net::TcpListener;
use sysinfo::{Pid, System};

use crate::paths;
use crate::constants::files;

/// 校验物理端口是否可用
pub fn check_port_availability(port: u16) -> Result<()> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(_) => Ok(()),
        Err(e) => anyhow::bail!("port {} already in use: {}", port, e),
    }
}

/// 读取 PID 文件并检查进程是否存在(预留 API)
#[allow(dead_code)]
pub fn get_running_process_info() -> Result<Option<ProcessInfo>> {
    let pid_path = paths::run_dir()?.join(files::PID_FILE);
    
    if !pid_path.exists() {
        return Ok(None);
    }

    let pid_str = fs::read_to_string(&pid_path)?;
    let raw_pid = pid_str.trim().parse::<u32>()?;
    let pid = Pid::from(raw_pid as usize);

    let mut sys = System::new_all();
    sys.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
    );

    if let Some(process) = sys.process(pid) {
        let name = process.name().to_string_lossy().to_string();
        if name.to_lowercase().contains("aikv") {
            return Ok(Some(ProcessInfo {
                pid: raw_pid,
                name,
                memory_kb: process.memory() / 1024,
                uptime_secs: process.run_time(),
            }));
        }
    }

    Ok(None)
}

/// 进程信息(预留结构体)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub memory_kb: u64,
    pub uptime_secs: u64,
}

/// 停止本地进程
pub fn stop_process(pid: u32) -> Result<bool> {
    #[cfg(unix)]
    {
        use libc::{kill, SIGTERM};
        unsafe {
            if kill(pid as i32, SIGTERM) == 0 {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// 清理 PID 文件
pub fn cleanup_pid_file() -> Result<()> {
    let pid_path = paths::run_dir()?.join(files::PID_FILE);
    if pid_path.exists() {
        fs::remove_file(pid_path)?;
    }
    Ok(())
}

/// 写入 PID 文件
pub fn write_pid_file(pid: u32) -> Result<()> {
    let pid_path = paths::run_dir()?.join(files::PID_FILE);
    fs::write(&pid_path, pid.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn check_port_available() {
        // 端口 0 让 OS 分配, 然后检查该端口是否被报告为占用
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let bound_port = listener.local_addr().unwrap().port();

        // 端口已被绑定, 应返回错误
        let result = check_port_availability(bound_port);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains(&bound_port.to_string()));

        // 释放端口
        drop(listener);

        // 端口释放后应可用
        let result = check_port_availability(bound_port);
        assert!(result.is_ok());
    }

    #[test]
    fn write_and_cleanup_pid_file() {
        // 写入 PID 文件
        write_pid_file(99999).unwrap();

        // 验证文件存在且内容正确
        let pid_path = paths::run_dir().unwrap().join(files::PID_FILE);
        assert!(pid_path.exists());
        let content = fs::read_to_string(&pid_path).unwrap();
        assert_eq!(content, "99999");

        // 清理
        cleanup_pid_file().unwrap();
        assert!(!pid_path.exists());
    }

    #[test]
    fn cleanup_pid_file_when_not_exists() {
        // 确保文件不存在时 cleanup 不 panic
        let pid_path = paths::run_dir().unwrap().join(files::PID_FILE);
        if pid_path.exists() {
            fs::remove_file(&pid_path).unwrap();
        }
        assert!(cleanup_pid_file().is_ok());
    }

    #[test]
    fn process_info_struct() {
        let info = ProcessInfo {
            pid: 1234,
            name: "aikv".to_string(),
            memory_kb: 2048,
            uptime_secs: 300,
        };
        assert_eq!(info.pid, 1234);
        assert_eq!(info.name, "aikv");
        assert_eq!(info.memory_kb, 2048);
        assert_eq!(info.uptime_secs, 300);
    }

    #[test]
    fn stop_process_nonexistent() {
        // 对一个不太可能存在的 PID 调用 stop_process
        // 返回 false 表示信号发送失败(进程不存在)
        let result = stop_process(4_000_000_000);
        assert!(result.is_ok());
        // 在 Unix 系统上, PID 不存在时 kill 返回 -1, 所以 result 为 false
        assert!(!result.unwrap());
    }
}
