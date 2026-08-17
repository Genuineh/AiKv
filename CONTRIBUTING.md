# 贡献指南

本文是面向开源贡献者与系统开发者的 **本地开发、测试验证、Git 分支与 PR 提交流程指南**.

- 项目架构与分层设计见 [ARCHITECTURE.md](ARCHITECTURE.md);
- 部署、Feature 开关与配置调优见 [docs/deployment.md](docs/deployment.md);
- 设计决策与技术权衡见 [docs/design.md](docs/design.md);
- 完整 CI 架构见 [.github/README.md](.github/README.md).

```mermaid
flowchart LR
    S1[1. 准备开发环境] --> S2[2. 配置 Git 钩子]
    S2 --> S3[3. Issue 与分支]
    S3 --> S4[4. 本地验证与测试]
    S4 --> S5[5. 规范 Commit]
    S5 --> S6[6. 文档同步]
    S6 --> S7[7. 提交 PR 并自检]
```

---

## 1. 准备开发环境

### Rust 工具链

本项目固定使用 **Rust stable** (含 `clippy` 与 `rustfmt`), 由 [`rust-toolchain.toml`](rust-toolchain.toml) 声明.  
进入仓库目录后 `rustup` 会自动切换为对应版本, 可通过 `rustup show` 确认.

### 系统依赖与 Protoc

- **Linux / macOS**: 推荐开发环境 (CI 运行在 `ubuntu-latest`).
- **protoc (Protobuf 编译器)**: 编译 `cluster` 特性时生成 Raft gRPC 协议桩代码需要.
  ```bash
  # Ubuntu / Debian
  sudo apt-get install -y protobuf-compiler
  # macOS
  brew install protobuf
  ```

### AiDb 依赖与本地 Patch

`Cargo.toml` 中 `aidb` 默认通过 Git 依赖引入 (`branch = "new/main"`). 本地高频联调时, 推荐在 `~/.cargo/config.toml` 中配置本地覆盖:

```toml
# ~/.cargo/config.toml (仅本地生效, 不提交到 Git 仓库)
[patch."https://github.com/wiqun/AiDb.git"]
aidb = { path = "/absolute/path/to/aidb" }
```

---

## 2. 配置 Git 钩子与安全门禁

克隆仓库后建议立即安装本地 Git 钩子:

```bash
./install-hooks.sh   # 软链接 hooks/* → .git/hooks/
```

### 钩子职责说明

- [`hooks/pre-commit`](hooks/pre-commit): 在 `git commit` 时自动执行:
  1. 分支保护 ([`hooks/check-branch.sh`](hooks/check-branch.sh)): 禁止直接在 `main` / `new/main` 提交代码
  2. 文档链接检查 ([`hooks/check-docs-links.sh`](hooks/check-docs-links.sh)): 检查暂存区 `.md` 文件的相对链接与死链
  3. 依赖解析检查: 验证 `aidb` 依赖解析正常
  4. 代码格式检查: `cargo fmt --check`
  5. 静态分析检查: `RUSTFLAGS='-D warnings' cargo clippy --all-targets --all-features`
  6. 安全扫描 ([`hooks/check-security.sh`](hooks/check-security.sh)): `cargo audit` 与 `cargo deny check`
- [`hooks/commit-msg`](hooks/commit-msg): 校验提交说明是否遵循 Conventional Commits 规范 (如 `feat:`, `fix:`, `chore:` 等).

> **说明**: Git hook 默认**不执行** `cargo test`, 测试由开发者本地手动或 CI 运行.

### Security 检查与逃生门

遇到已知依赖上游未修复漏洞时, 可使用环境变量临时跳过安全扫描:

```bash
SKIP_SECURITY=1 git commit -m "..."   # 仅跳过 security, fmt 与 clippy 仍正常执行
```

---

## 3. Issue 驱动与分支规范

### GitHub Issues 驱动

所有新功能、bug 修复、重构或文档改动均须与 GitHub Issue 关联:
1. 先创建 GitHub Issue (按场景选用模板: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `perf`).
2. 明确 Issue 的需求、复现步骤或验收条件后再开始编码.

### 分支命名规范

所有工作分支基于主开发分支 (`new/main`) 创建:

- **分支命名格式**: `{type}/{NN}-{slug}`
- **示例**:
  - `feat/102-cluster-slot-migration`
  - `fix/45-resp-parser-buffer-overflow`
  - `docs/88-refactor-documentation-system`

---

## 4. 本地验证与测试矩阵

### 4.1 本地快速门禁

```bash
export RUSTFLAGS='-D warnings'
cargo fmt --check
cargo clippy --all-targets --features cluster,monitoring
cargo test --workspace --features cluster,monitoring -- --test-threads=1
```

> **注意**: 集成测试必须添加 `--test-threads=1`, 避免端口和数据目录冲突.

### 4.2 专项测试与慢测

```bash
# 慢测与压力测试
cargo test --test server --features cluster -- --ignored --test-threads=1
cargo test --test commands --features cluster -- --ignored --test-threads=1
cargo test --test stress_ttl --features cluster -- --ignored --test-threads=1

# 块压缩测试
cargo test --features compression --lib storage::aidb_options
```

### 4.3 E2E 黑盒验收测试

基于 pytest 的黑盒验收测试套件 (需先启动待测服务):

```bash
pytest e2e/function/ -v
```

---

## 5. 文档同步规范 (硬性要求)

- **修改公共 API 或命令语义**: 必须同步更新 [`docs/modules/`](docs/modules/) 对应的模块文档;
- **修改架构或配置**: 必须同步更新 [ARCHITECTURE.md](ARCHITECTURE.md) 与 [docs/deployment.md](docs/deployment.md);
- **Bug 修复 (`fix:`)**: 必须在同一 PR 附带回归测试, commit 消息必须引用 Issue (`Fixes #NN`);
- **相对路径引用**: 所有文档间链接必须采用有效的相对路径, 通过 `hooks/check-docs-links.sh` 检查.

---

## 6. PR 提交与代码审查 (Code Review)

1. **推送分支**: 将工作分支推送到远程仓库;
2. **创建 PR**: 目标分支指定为 `new/main`, 关联对应 Issue;
3. **CI 门禁全绿**: 确保 GitHub Actions 中的 `ci.yml` 与 `security.yml` 全部通过;
4. **审查与合并**: 经过 Maintainer Code Review 通过后, 以 **Squash and merge** 方式合并回 `new/main`.
