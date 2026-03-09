//! CLI 集成测试
//!
//! 使用 assert_cmd 测试 ak 命令行工具的基本行为. 
//! 这些测试验证 CLI 解析层, 不依赖外部服务(Docker, 进程等). 

use assert_cmd::Command;
use predicates::prelude::*;

fn ak_cmd() -> Command {
    #[allow(deprecated)]
    Command::cargo_bin("ak").unwrap()
}

// ─── 帮助信息 ──────────────────────────────────────────────

#[test]
fn help_flag() {
    ak_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("AiKv"))
        .stdout(predicate::str::contains("build"))
        .stdout(predicate::str::contains("up"))
        .stdout(predicate::str::contains("down"))
        .stdout(predicate::str::contains("restart"))
        .stdout(predicate::str::contains("logs"))
        .stdout(predicate::str::contains("ps"))
        .stdout(predicate::str::contains("config"))
        .stdout(predicate::str::contains("clean"))
        .stdout(predicate::str::contains("quick"));
}

#[test]
fn version_flag() {
    ak_cmd()
        .arg("-v")
        .assert()
        .success()
        .stdout(predicate::str::contains("ak"));
}

// ─── 子命令帮助 ────────────────────────────────────────────

#[test]
fn build_help() {
    ak_cmd()
        .args(["build", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("--topo"))
        .stdout(predicate::str::contains("--force"))
        .stdout(predicate::str::contains("--release"));
}

#[test]
fn up_help() {
    ak_cmd()
        .args(["up", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("--topo"))
        .stdout(predicate::str::contains("--nodes"))
        .stdout(predicate::str::contains("--shards"))
        .stdout(predicate::str::contains("--replicas"));
}

#[test]
fn down_help() {
    ak_cmd()
        .args(["down", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("--remove-volumes"));
}

#[test]
fn restart_help() {
    ak_cmd()
        .args(["restart", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("--init"))
        .stdout(predicate::str::contains("-i")); // short for --init
}

#[test]
fn logs_help() {
    ak_cmd()
        .args(["logs", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--follow"))
        .stdout(predicate::str::contains("--lines"));
}

#[test]
fn ps_help() {
    ak_cmd()
        .args(["ps", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("--topo"))
        .stdout(predicate::str::contains("--output"));
}

#[test]
fn config_help() {
    ak_cmd()
        .args(["config", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("get"))
        .stdout(predicate::str::contains("set"))
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("path"));
}

#[test]
fn clean_help() {
    ak_cmd()
        .args(["clean", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("--all"))
        .stdout(predicate::str::contains("--force"));
}

#[test]
fn quick_help() {
    ak_cmd()
        .args(["quick", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("--topo"))
        .stdout(predicate::str::contains("--nodes"))
        .stdout(predicate::str::contains("--shards"))
        .stdout(predicate::str::contains("--replicas"))
        .stdout(predicate::str::contains("--image"))
        .stdout(predicate::str::contains("--force"))
        .stdout(predicate::str::contains("--release"));
}

// ─── 错误处理 ──────────────────────────────────────────────

#[test]
fn no_subcommand_shows_help_or_error() {
    // 不带子命令时应报错
    ak_cmd().assert().failure();
}

#[test]
fn unknown_subcommand() {
    ak_cmd()
        .arg("nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}

#[test]
fn build_invalid_mode() {
    ak_cmd()
        .args(["build", "--mode", "invalid"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid"));
}

#[test]
fn up_invalid_topo() {
    ak_cmd()
        .args(["up", "--topo", "distributed"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("distributed"));
}

#[test]
fn quick_invalid_mode() {
    ak_cmd()
        .args(["quick", "--mode", "invalid"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid"));
}

#[test]
fn quick_invalid_topo() {
    ak_cmd()
        .args(["quick", "--topo", "distributed"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("distributed"));
}

#[test]
fn build_invalid_topo() {
    ak_cmd()
        .args(["build", "--topo", "distributed"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("distributed"));
}

// ─── 命令别名 ──────────────────────────────────────────────

#[test]
fn build_alias_b() {
    // 'b' 是 'build' 的别名
    ak_cmd()
        .args(["b", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--mode"));
}

#[test]
fn logs_alias_l() {
    // 'l' 是 'logs' 的别名
    ak_cmd()
        .args(["l", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--follow"));
}

// ─── config 子命令 ────────────────────────────────────────

#[test]
fn config_path_runs() {
    // config path 应该能成功运行(只是输出路径)
    ak_cmd().args(["config", "path"]).assert().success();
}

#[test]
fn config_get_default_format() {
    // config get 默认以 YAML 格式输出当前配置
    ak_cmd()
        .args(["config", "get"])
        .assert()
        .success()
        .stdout(predicate::str::contains("mode"))
        .stdout(predicate::str::contains("topo"));
}

#[test]
fn config_get_json() {
    ak_cmd()
        .args(["config", "get", "-o", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"))
        .stdout(predicate::str::contains("schema_version"));
}

#[test]
fn config_get_table() {
    ak_cmd()
        .args(["config", "get", "-o", "table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Property"))
        .stdout(predicate::str::contains("Value"));
}

#[test]
fn config_set_invalid_format() {
    // 没有 '=' 分隔符应报错
    ak_cmd()
        .args(["config", "set", "no_equals_sign"])
        .assert()
        .failure();
}

// ─── 互斥参数 ──────────────────────────────────────────────

#[test]
fn up_nodes_conflicts_with_shards() {
    ak_cmd()
        .args(["up", "--nodes", "3", "--shards", "3"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn up_replicas_requires_shards() {
    ak_cmd()
        .args(["up", "--replicas", "1"])
        .assert()
        .failure();
}

#[test]
fn quick_nodes_conflicts_with_shards() {
    ak_cmd()
        .args(["quick", "--nodes", "3", "--shards", "3"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn quick_replicas_requires_shards() {
    ak_cmd()
        .args(["quick", "--replicas", "1"])
        .assert()
        .failure();
}
