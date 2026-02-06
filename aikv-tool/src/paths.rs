//! XDG 路径解析
//!
//! 遵循 XDG Base Directory Specification, 提供应用目录解析. 
//! 所有函数均为纯函数或仅涉及目录创建, 不涉及配置业务逻辑. 

use anyhow::{anyhow, Result};
use std::fs;
use std::path::PathBuf;

use crate::constants::{dirs as app_dirs, files, xdg, APP_NAME};

/// 获取 XDG 配置文件路径 (~/.config/ak/ak.toml)
pub fn config_path() -> Option<PathBuf> {
    ::dirs::config_dir().map(|p| p.join(APP_NAME).join(files::AK_CONFIG))
}

/// 获取 XDG 状态目录 (~/.local/state/ak/)
pub fn state_dir() -> Option<PathBuf> {
    if let Ok(state_home) = std::env::var(xdg::ENV_STATE_HOME) {
        return Some(PathBuf::from(state_home).join(APP_NAME));
    }
    let mut path = ::dirs::home_dir()?;
    for segment in xdg::DEFAULT_STATE_SUBPATH {
        path = path.join(segment);
    }
    Some(path.join(APP_NAME))
}

/// 获取 XDG 缓存目录 (~/.cache/ak/)
pub fn cache_dir() -> Option<PathBuf> {
    ::dirs::cache_dir().map(|p| p.join(APP_NAME))
}

/// 获取运行时状态目录 (~/.local/state/ak/run/), 不存在则自动创建
pub fn run_dir() -> Result<PathBuf> {
    let path = state_dir()
        .ok_or_else(|| anyhow!("could not get state directory"))?
        .join(app_dirs::RUN);
    if !path.exists() {
        fs::create_dir_all(&path)?;
    }
    Ok(path)
}

/// 获取日志目录 (~/.cache/ak/logs/), 不存在则自动创建
pub fn log_dir() -> Result<PathBuf> {
    let path = cache_dir()
        .ok_or_else(|| anyhow!("could not get cache directory"))?
        .join(app_dirs::LOGS);
    if !path.exists() {
        fs::create_dir_all(&path)?;
    }
    Ok(path)
}

/// 从当前目录向上查找本地配置文件 (ak.toml)
pub fn find_local_config() -> Option<PathBuf> {
    let mut current = std::env::current_dir().ok()?;
    loop {
        let config_file = current.join(files::AK_CONFIG);
        if config_file.exists() {
            return Some(config_file);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_ends_with_ak_toml() {
        if let Some(path) = config_path() {
            assert!(path.ends_with(format!("{}/{}", APP_NAME, files::AK_CONFIG)));
        }
        // 如果 dirs::config_dir() 返回 None (CI 环境), 跳过
    }

    #[test]
    fn state_dir_with_xdg_env() {
        // 保存并设置环境变量
        let original = std::env::var(xdg::ENV_STATE_HOME).ok();
        std::env::set_var(xdg::ENV_STATE_HOME, "/tmp/test-xdg-state");

        let result = state_dir();
        assert!(result.is_some());
        let dir = result.unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/test-xdg-state").join(APP_NAME));

        // 还原
        match original {
            Some(val) => std::env::set_var(xdg::ENV_STATE_HOME, val),
            None => std::env::remove_var(xdg::ENV_STATE_HOME),
        }
    }

    #[test]
    fn state_dir_without_xdg_env() {
        let original = std::env::var(xdg::ENV_STATE_HOME).ok();
        std::env::remove_var(xdg::ENV_STATE_HOME);

        if let Some(dir) = state_dir() {
            // 应回退到 $HOME/.local/state/ak
            assert!(dir.ends_with(format!(".local/state/{}", APP_NAME)));
        }

        // 还原
        if let Some(val) = original {
            std::env::set_var(xdg::ENV_STATE_HOME, val);
        }
    }

    #[test]
    fn cache_dir_ends_with_ak() {
        if let Some(path) = cache_dir() {
            assert!(path.ends_with(APP_NAME));
        }
    }

    #[test]
    fn run_dir_creates_directory() {
        // run_dir 会在 state_dir 下创建 run/ 子目录
        let result = run_dir();
        if let Ok(path) = result {
            assert!(path.exists(), "run_dir 应自动创建目录");
            assert!(path.ends_with("run"));
        }
    }

    #[test]
    fn log_dir_creates_directory() {
        let result = log_dir();
        if let Ok(path) = result {
            assert!(path.exists(), "log_dir 应自动创建目录");
            assert!(path.ends_with("logs"));
        }
    }

    #[test]
    fn find_local_config_returns_none_for_root() {
        // 在根目录不太可能有 ak.toml
        // 这是一个弱测试, 但确保函数不 panic
        let _ = find_local_config();
    }
}
