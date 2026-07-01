//! AiDb `Options` 构建: CLI / 生产 vs 单测 preset.

use aidb::config::Options;

/// 生产 preset 名称 (CLI `--aidb-preset` / 环境变量 `AIKV_AIDB_PRESET`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DbPreset {
    #[default]
    Default,
    HighWrite,
    HighRead,
}

impl DbPreset {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "default" => Some(Self::Default),
            "high-write" | "high_write" => Some(Self::HighWrite),
            "high-read" | "high_read" => Some(Self::HighRead),
            _ => None,
        }
    }
}

/// CLI / 生产路径: 对齐 aidb DEPLOYMENT preset.
pub fn server_db_options(sync_wal: bool) -> Options {
    server_db_options_with_preset(sync_wal, DbPreset::Default)
}

pub fn server_db_options_with_preset(sync_wal: bool, preset: DbPreset) -> Options {
    let base = match preset {
        DbPreset::Default => Options::default(),
        DbPreset::HighWrite => Options::for_high_write_throughput(),
        DbPreset::HighRead => Options::for_high_read_throughput(),
    };
    Options {
        create_if_missing: true,
        sync_wal,
        ..base
    }
}

/// 单测 / 快速回归: 小 memtable、关 background_compaction.
pub fn testing_db_options() -> Options {
    Options {
        create_if_missing: true,
        ..Options::for_testing()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_preset_parse_aliases() {
        assert_eq!(DbPreset::parse("default"), Some(DbPreset::Default));
        assert_eq!(DbPreset::parse("high-write"), Some(DbPreset::HighWrite));
        assert_eq!(DbPreset::parse("high_write"), Some(DbPreset::HighWrite));
        assert_eq!(DbPreset::parse("HIGH-READ"), Some(DbPreset::HighRead));
        assert!(DbPreset::parse("unknown").is_none());
    }

    #[test]
    fn high_write_preset_enlarges_memtable() {
        let opts = server_db_options_with_preset(false, DbPreset::HighWrite);
        assert_eq!(opts.memtable_size, 256 * 1024 * 1024);
    }
}
