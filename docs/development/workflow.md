# AiKv 优化工作流

本文档定义了 AiKv 数据库优化工作流的标准化执行流程。

## 步骤

| Step | Name | Description | How to do |
|------|------|------|-----------|
| 1 | create-branch | 创建优化用分支 | `git checkout -b {new-branch}` 遵循 git 分支命名规范 |
| 2 | detect-env | 检测环境 | @.cursor/skills/detect-env/SKILL.md |
| 3 | static-analysis | 代码静态分析 | 依次检查：1. 语法/编译 `cargo check` 2. 格式 `cargo fmt --check` 3. Clippy 规范 `cargo clippy -- -D warnings` 4. 安全审计 `cargo audit` 5. 依赖过时检查 `cargo outdated` |
| 4 | unit-test | 单元测试 | `cargo test` |
| 5 | smoke-test | 冒烟测试 | `./scripts/test_smoke.sh` |
| 6 | build-project | 构建 | @.cursor/skills/build-deploy/SKILL.md |
| 7 | deploy-service | 部署 | @.cursor/skills/build-deploy/SKILL.md |
| 8 | functional-test | 功能测试 | `./scripts/test_functional.sh  |
| 9 | baseline-test | 基线性能测试（锚点） |  |
| 10 | generate-report | 生成测试报告 | 基于 step09-baseline-test.json 整理成可读报告（Markdown/文本） |
| 11 | analyze-report | 分析报告 → 优化方案 | 根据报告输出优化目标 + 阈值，供 Step 12 使用 |
| 12 | optimize-code | 按方案改代码 | 代码变更；失败可撤销 |
| 13 | static-analysis | 代码静态分析 | 同 Step 3 |
| 14 | unit-test | 单元测试 | 同 Step 4 |
| 15 | commit-changes | 本地提交 | `git add <files> && git commit -m "..."` → 本地 commit |
| 16 | rebuild | 重新构建 | @.cursor/skills/build-deploy/SKILL.md |
| 17 | redeploy | 重新部署 | @.cursor/skills/build-deploy/SKILL.md |
| 18 | functional-test-2 | 命令全量功能测试（回归前） | 同 Step 8：`./scripts/test_functional.sh [host] [port]`，确认优化后仍通过全量命令 |
| 19 | regression-test | 回归测试（浮标） |  |
| 20 | generate-report-2 | 生成回归报告 | 同 Step 10（基于 step19 数据） |
| 21 | compare-results | 对比基线 vs 回归 | 对比 step09 与 step19 的 JSON；达标 → Step 22，未达标 → 回 Step 12（最多 3 次） |
| 22 | finalize-docs | 完善文档 | 记录变更与优化结果 → 工作流结束 |
