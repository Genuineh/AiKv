//! @component aikv-config

use std::net::TcpListener;
use std::process::Command;

#[test]
fn config_priority_print_config_stdout_before_bind() {
    // --print-config 后继续启动; 占住 bind 端口使子进程 bind 失败后退出, .output() 才能返回.
    let guard = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = guard.local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("aikv.toml");
    std::fs::write(&cfg, format!("[server]\nbind = \"127.0.0.1:{port}\"\n")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_aikv"))
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--print-config",
            "--engine",
            "memory",
        ])
        .output()
        .unwrap();
    // print_config 在 init_logging/bind 之前; 进程可能随后 bind 失败, 但 stdout 必须先有 TOML
    let stdout = String::from_utf8(out.stdout).unwrap();
    let parsed: toml::Value =
        toml::from_str(&stdout).expect("stdout must contain only valid config TOML");
    assert!(parsed.get("server").is_some());
    assert_eq!(
        parsed
            .get("server")
            .and_then(|server| server.get("bind"))
            .and_then(toml::Value::as_str),
        Some(format!("127.0.0.1:{port}").as_str())
    );
    for marker in [
        "config file:",
        "aikv starting",
        "\"timestamp\"",
        cfg.to_str().unwrap(),
    ] {
        assert!(
            !stdout.contains(marker),
            "stdout must not contain log marker {marker:?}: {stdout}"
        );
    }

    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains(&format!("config file: {}", cfg.display())),
        "stderr must contain config file log: {stderr}"
    );
}

/// 回归测试: `--sync-wal` 裸 flag 必须覆盖 TOML 与 env 的 `false`.
#[test]
fn cli_sync_wal_bare_flag_overrides_lower_layers() {
    let guard = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = guard.local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("aikv.toml");
    std::fs::write(
        &cfg,
        format!("[server]\nbind = \"127.0.0.1:{port}\"\n[engine]\nsync_wal = false\n"),
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_aikv"))
        .env("AIKV_SYNC_WAL", "false")
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--print-config",
            "--engine",
            "memory",
            "--sync-wal",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(out.stdout).unwrap();
    let parsed: toml::Value =
        toml::from_str(&stdout).expect("stdout must contain only valid config TOML");
    assert_eq!(
        parsed
            .get("engine")
            .and_then(|engine| engine.get("sync_wal"))
            .and_then(toml::Value::as_bool),
        Some(true)
    );
}
