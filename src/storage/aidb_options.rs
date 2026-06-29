//! AiDb `Options` 构建: CLI / 生产 vs 单测 preset.

use aidb::config::Options;

/// CLI / 生产路径: 对齐 aidb DEPLOYMENT 的 `Options::default()`.
pub fn server_db_options(sync_wal: bool) -> Options {
    Options {
        create_if_missing: true,
        sync_wal,
        ..Options::default()
    }
}

/// 单测 / 快速回归: 小 memtable、关 background_compaction.
pub fn testing_db_options() -> Options {
    Options {
        create_if_missing: true,
        ..Options::for_testing()
    }
}
