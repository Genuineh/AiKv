use std::path::{Path, PathBuf};

use super::settings::{ConfigError, Settings};

const CONFIG_FILENAME: &str = "aikv.toml";
const ETC_CONFIG_PATH: &str = "/etc/aikv/aikv.toml";

/// 在 `search_dir` 下发现配置: explicit > search_dir/aikv.toml (运行时 search_dir = cwd).
pub fn discover_config_path_in(
    search_dir: &Path,
    explicit: Option<&Path>,
) -> Result<Option<PathBuf>, ConfigError> {
    if let Some(path) = explicit {
        if path.is_file() {
            return Ok(Some(path.to_path_buf()));
        }
        return Err(ConfigError::File {
            path: path.to_path_buf(),
            message: "file not found".to_string(),
        });
    }

    let candidate = search_dir.join(CONFIG_FILENAME);
    if candidate.is_file() {
        Ok(Some(candidate))
    } else {
        Ok(None)
    }
}

/// 生产 wrapper: explicit > ./aikv.toml (cwd) > /etc/aikv/aikv.toml.
pub fn discover_config_path(explicit: Option<&Path>) -> Result<Option<PathBuf>, ConfigError> {
    let cwd = std::env::current_dir().map_err(|e| ConfigError::File {
        path: PathBuf::from("."),
        message: e.to_string(),
    })?;

    if explicit.is_some() {
        return discover_config_path_in(&cwd, explicit);
    }

    if let Some(found) = discover_config_path_in(&cwd, None)? {
        return Ok(Some(found));
    }

    let etc = PathBuf::from(ETC_CONFIG_PATH);
    if etc.is_file() {
        Ok(Some(etc))
    } else {
        Ok(None)
    }
}

pub fn load_settings_from_file(path: &Path) -> Result<Settings, ConfigError> {
    let contents = std::fs::read_to_string(path).map_err(|e| ConfigError::File {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    toml::from_str(&contents).map_err(|e| ConfigError::File {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use super::*;

    #[test]
    fn discover_config_path_in_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let found = discover_config_path_in(dir.path(), None).unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn discover_config_path_in_finds_aikv_toml() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(CONFIG_FILENAME);
        std::fs::write(&config_path, "[server]\n").unwrap();

        let found = discover_config_path_in(dir.path(), None).unwrap();
        assert_eq!(found, Some(config_path));
    }

    #[test]
    fn discover_config_path_in_explicit_missing_fails() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.toml");
        let err = discover_config_path_in(dir.path(), Some(&missing)).unwrap_err();
        assert!(matches!(err, ConfigError::File { .. }));
    }

    #[test]
    fn discover_config_path_in_explicit_takes_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let default_path = dir.path().join(CONFIG_FILENAME);
        let explicit_path = dir.path().join("custom.toml");
        std::fs::write(&default_path, "[server]\n").unwrap();
        std::fs::write(&explicit_path, "[server]\n").unwrap();

        let found = discover_config_path_in(dir.path(), Some(&explicit_path)).unwrap();
        assert_eq!(found, Some(explicit_path));
    }

    #[test]
    fn load_partial_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.toml");
        std::fs::write(&path, "[server]\nbind = \"192.168.1.1:6380\"\n").unwrap();

        let settings = load_settings_from_file(&path).unwrap();
        assert_eq!(
            settings.server.bind,
            Some(SocketAddr::from((Ipv4Addr::new(192, 168, 1, 1), 6380)))
        );
        assert!(settings.server.max_clients.is_none());
        assert!(settings.engine.kind.is_none());
        assert!(settings.observability.metrics_addr.is_none());
    }

    #[test]
    fn load_unknown_key_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "[server]\nunknown_key = 1\n").unwrap();
        assert!(load_settings_from_file(&path).is_err());
    }

    #[test]
    fn load_engine_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine.toml");
        std::fs::write(
            &path,
            "[engine]\nkind = \"aidb\"\ndata_dir = \"/var/lib/aikv\"\n",
        )
        .unwrap();

        let settings = load_settings_from_file(&path).unwrap();
        assert_eq!(
            settings.engine.kind,
            Some(super::super::engine::EngineKind::AiDb)
        );
        assert_eq!(
            settings.engine.data_dir,
            Some(PathBuf::from("/var/lib/aikv"))
        );
    }
}
