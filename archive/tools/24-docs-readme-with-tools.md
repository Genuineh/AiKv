# Cursor transcript

---

## 1. User

## 文档整理 — 新会话

工作目录: `/root/code/database`

请先阅读 (不要跳过):
1. `AiKv-Workflow/backup/design.md` — 总规程
2. `AiKv-Workflow/backup/PROGRESS.md` — 当前进度
3. `AiKv-Workflow/backup/INDEX.md` — 本章索引 (若已有行则更新)

---

### 本次任务

| 项 | 值 |
|----|-----|
| PROGRESS 总步 | 24 (阶段 2b 第 6 篇, **aikv 汇总最后一篇**) |
| 目标文件 | `aikv/docs/README.md` |
| 本次子步 | 从步 0 开始 |
| 对比深度 | **模块级** (汇总文档; 纯导航链接, 不加深章逐段) |

### 必须遵守

- **确认门控**: 步 0→1→2→3→4, 每步先讨论, 我确认后再下一步
- **步 4**: 先出 **正文草稿**, 我确认后再写入 `aikv/`、更新 INDEX/PROGRESS
- **汇总文档**: **不必** Module Skill frontmatter
- **过程只进 backup/**: INDEX; aikv 不写迁移历史
- **旧文档顺序**: `backup/aikv/` → `aikv-oldmain` 根目录/`docs/` 类 → WiQunTools 查漏 → `wiqun-kv`
- **待核实**: 根文档至多简要提及; 详情仍引用 `aikv/ISSUES.md` (不展开排查过程)
- **步 4 落盘前**: 对照 design.md 验收项 — 链到现有根文档与 7 篇 modules 均有效; 与 [`README.md`](aikv/README.md) 分工不重复长文

### 上下文 (上一篇已完成)

- 步 23 ✅: [`aikv/CONTRIBUTING.md`](aikv/CONTRIBUTING.md) + [`aikv/CHANGELOG.md`](aikv/CHANGELOG.md)
- 步 19–22 ✅: ARCHITECTURE / DESIGN / DEPLOYMENT / README
- 阶段 1 modules 7 篇全部 ✅ (`docs/modules/*.md`)
- 当前 `aikv/docs/` **仅有** `modules/` — **尚无** `docs/README.md`
- `docs/development.md` **尚未创建** (design 目标结构有占位; 本步步 0 可讨论: 建议与 aidb 步 18 一致 — **A1 不建**, hub 链 DEPLOYMENT + CONTRIBUTING)
- 子目录说明 (`examples/README.md`, `tests/README.md`, `e2e/README.md`, `.github/README.md`) — design 称 **不在主文档体系**; 导航页是否一行链过去, 步 0 定
- 范本 (同类步 18): [`aidb/docs/README.md`](aidb/docs/README.md)
- 本会话归档参考: [`aikv/archive/23-contributing-changelog.md`](aikv/archive/23-contributing-changelog.md); aidb 同类: [`aidb/archive/18-docs-readme.md`](aidb/archive/18-docs-readme.md)

### 边界提示 (步 0 可细化)

**docs/README.md** — 回答「开发文档从哪读、链到哪」:

- 宜含: **纯链接** 导航 — 根文档表 (ARCHITECTURE/DESIGN/DEPLOYMENT/CONTRIBUTING/CHANGELOG/AGENTS/ISSUES); `docs/modules/` 七篇 (WHEN 一句话 + 链); 阅读路径 (首次了解 / 改某域 / 构建测试); 与 AiDb 文档互链一句 (可选)
- 不重复: README 的 30 秒入门/feature 表; CONTRIBUTING 测试矩阵; module 正文; ARCHITECTURE 架构图
- 旧稿: backup 侧 **无** 独立 `docs/README.md` — 从根 README §文档 + oldmain `docs/index.md` 查漏

**与相邻文档分工**

| 文档 | 回答 |
|------|------|
| [`README.md`](aikv/README.md) | 项目入口; 已有 §文档 扁平短表 |
| **docs/README.md (本篇)** | `docs/` 域导航 hub; modules 索引 |
| 根 ARCHITECTURE/DESIGN/… | 汇总正文 |
| `docs/modules/*.md` | 域级 Skill 正文 |

### 本次不要

- 不要跳过未确认的步骤
- 不要一步写 `docs/development.md` 正文 (除非步 0 你我确认本步顺带占位且我确认)
- 不要开始 Step 3 全仓验收或 Step 4 删旧仓库 (除非我明确要求)
- 不要修改 `design.md` / 流程, 除非我明确要求
- 不要 commit, 除非我明确要求

---

请从步 0 开始, 给出本步交付物, 等我确认。

---

## 2. Assistant

先阅读规程、进度与索引，并对照 aidb 范本和 aikv 现有文档结构，再给出步 0 交付物。

**Tool `Read`**

```json
{
  "path": "/root/code/database/AiKv-Workflow/backup/design.md"
}
```


**Tool `Read`**

```json
{
  "path": "/root/code/database/AiKv-Workflow/backup/PROGRESS.md"
}
```


**Tool `Read`**

```json
{
  "path": "/root/code/database/AiKv-Workflow/backup/INDEX.md"
}
```


**Tool `Read`**

```json
{
  "path": "/root/code/database/aidb/docs/README.md"
}
```


**Tool `Read`**

```json
{
  "path": "/root/code/database/aikv/README.md"
}
```


---

## 3. Assistant

继续查看 aikv 现有文档结构、aidb 步 18 归档与旧稿参考。

**Tool `Glob`**

```json
{
  "glob_pattern": "aikv/docs/**/*"
}
```


**Tool `Read`**

```json
{
  "path": "/root/code/database/aidb/archive/18-docs-readme.md",
  "limit": 120
}
```


**Tool `Read`**

```json
{
  "path": "/root/code/database/aikv/AGENTS.md",
  "limit": 80
}
```


**Tool `Grep`**

```json
{
  "pattern": "docs/README|index\\.md|文档",
  "path": "/root/code/database/backup/aikv"
}
```


**Tool `Glob`**

```json
{
  "glob_pattern": "**/aikv-oldmain/**/index.md"
}
```


---

## 4. Assistant

**Tool `Read`**

```json
{
  "path": "/root/code/database/aikv-oldmain/docs/index.md"
}
```


**Tool `Grep`**

```json
{
  "path": "/root/code/database/AiKv-Workflow/backup/aikv/README.md",
  "pattern": "文档|docs/"
}
```


**Tool `Read`**

```json
{
  "offset": 115,
  "limit": 150,
  "path": "/root/code/database/aidb/archive/18-docs-readme.md"
}
```


**Tool `Grep`**

```json
{
  "path": "/root/code/database/aikv/docs/modules",
  "pattern": "^description:"
}
```


**Tool `Read`**

```json
{
  "offset": 76,
  "path": "/root/code/database/aikv/AGENTS.md"
}
```


---

## 5. Assistant

**Tool `Read`**

```json
{
  "offset": 125,
  "limit": 40,
  "path": "/root/code/database/AiKv-Workflow/backup/aikv/README.md"
}
```


**Tool `Grep`**

```json
{
  "path": "/root/code/database/aikv/archive/23-contributing-changelog.md",
  "pattern": "development|docs/README|A1|C1|B1"
}
```


**Tool `Read`**

```json
{
  "path": "/root/code/database/aidb/README.md",
  "offset": 70,
  "limit": 35
}
```


**Tool `Grep`**

```json
{
  "path": "/root/code/database/aikv",
  "pattern": "docs/README"
}
```


---

## 6. Assistant

**Tool `Grep`**

```json
{
  "path": "/root/code/database/aidb/README.md",
  "pattern": "docs/README"
}
```


**Tool `Grep`**

```json
{
  "path": "/root/code/database/aikv/docs/modules/observability.md",
  "pattern": "observability-reference"
}
```


---

## 7. Assistant

## 步 0 — 定范围

**PROGRESS 步 24** · 目标 `aikv/docs/README.md` · **阶段 2b 最后一篇 (aikv 汇总完结)**

---

### 1. 本章定位

| 项 | 内容 |
|----|------|
| **回答的问题** | 「开发文档从哪读、按什么路径跳转?」 — 尤其是 `docs/` 域与 **7 篇 module** 的 WHEN 路由 |
| **文档类型** | 纯链接导航 hub; **无** Module Skill frontmatter; **无** 正文深度 |
| **对比深度** | **模块级** — 旧稿只查漏结构/遗漏链接, 不做逐段对照 |
| **不覆盖** | 源码实现、架构图、设计 rationale、测试矩阵、module 正文、迁移历史 |

**与相邻文档分工** (对齐 design + 步 22/23 结论):

| 文档 | 职责 | 本篇关系 |
|------|------|----------|
| [`aikv/README.md`](aikv/README.md) | 项目入口; §文档 **扁平短表** (含 7 modules) | 不重复 quick start / feature / 示例 / e2e |
| **本篇 `docs/README.md`** | `docs/` 域 hub; modules **WHEN 一句话** + 根文档分组 | 主交付 |
| 根 ARCHITECTURE / DESIGN / … | 汇总正文 | 只链, 不摘抄 |
| `docs/modules/*.md` | 域级 Skill 正文 | 只链 + WHEN, 不展开 |
| [`CONTRIBUTING.md`](aikv/CONTRIBUTING.md) | hooks / CI / **完整测试矩阵** | 只链 (步 23 已写) |
| [`ISSUES.md`](aikv/ISSUES.md) | 待核实详情 | 至多一行入口 |

---

### 2. 本章边界 (含 / 不含)

**宜写入 `docs/README.md`:**

1. 开篇 1–2 句: 本文是 `docs/` 导航; 项目入口见 [`../README.md`](../README.md)
2. **阅读路径** — 首次了解 / 改某域 / 构建测试 (对齐 aidb 步 18 范本)
3. **根文档表** — ARCHITECTURE, DESIGN, DEPLOYMENT, CONTRIBUTING, CHANGELOG, AGENTS, ISSUES (各一行说明 + 链)
4. **`docs/modules/` 七篇** — 文件名 + **WHEN 一句话** (取自各 module frontmatter `description` 的 Use when 部分)
5. **构建与测试** — 一行链 DEPLOYMENT + CONTRIBUTING (不搬矩阵)
6. **待核实** — 一行链 ISSUES
7. **可选** — 与 AiDb 文档互链一句 (见下文 §4d)

**明确不写:**

- README 的 30 秒入门、特性/feature 表、示例/e2e 表
- CONTRIBUTING 测试矩阵、hooks 细节
- ARCHITECTURE mermaid / 数据流
- module 内代码地图、invariant、任务步骤
- ISSUES 排查过程
- oldmain `docs/index.md` 文档树、API 命令大全、归档结构

**源码关联:** 本篇 **不绑定** 单一 `src/` 路径; 步 1 以 **现有 aikv 文档树 + 链接有效性** 为准 (design §验收标准).

**当前 `aikv/docs/` 现状:**

```shell
aikv/docs/
└── modules/                    # 7 篇 Skill + observability-reference.md
    ├── cluster.md
    ├── commands-core.md
    ├── commands-extended.md
    ├── observability.md
    ├── observability-reference.md  # 附属参考, 非独立 module 步
    ├── protocol.md
    ├── server.md
    └── storage.md
```

`docs/README.md` **尚不存在**; `docs/development.md` **尚未创建**.

---

### 3. 拟读文件列表 (步 1–4 用)

**当前 aikv (步 1 主依据 — 链目标必须存在):**

| 类别 | 路径 |
|------|------|
| 邻接入口 | `aikv/README.md`, `aikv/AGENTS.md`, `aikv/CLAUDE.md` |
| 根汇总 | `ARCHITECTURE.md`, `DESIGN.md`, `DEPLOYMENT.md`, `CONTRIBUTING.md`, `CHANGELOG.md`, `ISSUES.md` |
| modules (7) | `docs/modules/{protocol,server,storage,commands-core,commands-extended,cluster,observability}.md` |
| 附属 | `docs/modules/observability-reference.md` (hub 是否单列, 见 §4e) |
| 范本 | [`aidb/docs/README.md`](aidb/docs/README.md) (步 18 落盘形态) |
| 缺失 | `docs/development.md` (**不存在**, 步 4 前决定是否建占位) |

**旧文档 (步 2 查漏, 模块级):**

| 顺序 | 路径 | 预期 |
|------|------|------|
| 1 | `AiKv-Workflow/backup/aikv/` | **无** 独立 `docs/README.md`; 根 README §设计文档 仅 3 篇 (ARCHITECTURE/DESIGN/DEPLOYMENT), **无 modules** |
| 2 | `aikv-oldmain/docs/index.md` | 旧 docs 树 + 快速导航; **大量过时** (guide/API/architecture 已拆入新根文档 + modules) |
| 2 | `aikv-oldmain/docs/guide/`, `development/` | 步 19–22 已消化; 本篇仅查漏 **hub 应链哪些、不应恢复哪些** |
| 3 | WiQunTools `wiqun-kv-inventory/` | 步 22 已判无入口专章 — **跳过** |
| 4 | `wiqun-kv/` | 与 backup 重复 — **跳过** |

**过程参考 (不写进 aikv):**

- [`aikv/archive/22-readme.md`](aikv/archive/22-readme.md) — README §文档 与 hub 分工
- [`aikv/archive/23-contributing-changelog.md`](aikv/archive/23-contributing-changelog.md) — development 留步 24
- [`aidb/archive/18-docs-readme.md`](aidb/archive/18-docs-readme.md) — A1/B1/C1 决策与落盘草稿

---

### 4. 步 0 待你拍板的五项

#### 4a. `docs/development.md` (步 23 已建议与 aidb 步 17/18 一致)

| 选项 | 做法 | 建议 |
|------|------|------|
| **A1 (推荐)** | 本篇 **「构建与测试」** 小节只链 DEPLOYMENT + CONTRIBUTING; **不创建** `development.md` | ✅ 与 aidb 步 18、步 23 归档一致 |
| A2 | 步 4 **同时** 创建 `development.md` 占位 (标题 + 链) | design 目标树有该文件, 正文另开 |
| A3 | 步 4 写 `development.md` 短导航正文 | 与 A1 差异小, 多维护一篇 |

**默认建议 A1**.

#### 4b. 子目录 README (`examples/`, `tests/`, `e2e/`, `.github/`)

design: **不在主文档体系内**; 根 README 已链 `examples/`、`e2e/`; AGENTS/CONTRIBUTING 已链 `.github/`.

| 选项 | 做法 | 建议 |
|------|------|------|
| **B1 (推荐)** | 本篇 **不写** 子目录 README | 导航页更纯, 与 aidb B1 一致 |
| B2 | 文末 **「其它」** 小节各一行: examples / e2e / tests / `.github/README.md` | 从 docs 域一键跳转 |

**默认建议 B1**.

#### 4c. 根 `README.md` §文档 是否改链

当前 README **直接列** 7 篇 modules + 根文档, **未** 指向 `docs/README.md` (步 22 刻意留步 24).

| 选项 | 做法 | 建议 |
|------|------|------|
| **C1 (推荐)** | 步 4 顺带: §文档 **首行** 加 hub 链 (对齐 aidb: `开发文档 hub: [docs/README.md](docs/README.md) (…)`); **保留** 现有扁平表 | 双入口: 入口快跳 + docs 域 hub |
| C2 | **不改** README; 仅新建 `docs/README.md` | 最小 diff; 与 README 略重复 |
| C3 | §文档 **收敛**: 根文档保留, modules 改为「详见 docs/README §按域阅读」 | 更短, 但改动 README 较大 |

**默认建议 C1 (轻量)** — 与 aidb 步 18 一致.

#### 4d. 与 AiDb 文档互链

| 选项 | 做法 | 建议 |
|------|------|------|
| **D1 (推荐)** | 文末或阅读路径加 **一句**: 存储/共识底层见 [`../aidb/docs/README.md`](../aidb/docs/README.md) | 与 ARCHITECTURE/DEPLOYMENT 已有 AiDb 边界呼应 |
| D2 | 不写 | hub 更自闭 |

**默认建议 D1 (一句)**.

#### 4e. `observability-reference.md`

| 选项 | 做法 | 建议 |
|------|------|------|
| **E1 (推荐)** | **不** 单独占 modules 表一行; 在 observability 的 WHEN 单元格末尾加「指标表 → observability-reference.md」 | 与 progressive disclosure 一致 |
| E2 | modules 表 **8 行**, reference 独立一行 | 表更长, 易与 7 域 module 混淆 |

**默认建议 E1**.

---

### 5. 拟 INDEX 行 (确认后于步 4 写入 `backup/INDEX.md`)

| 新文档章节 | 状态 | 参考的旧文档 | 处理 | 备注 |
|------------|------|--------------|------|------|
| `docs/README.md` (导航) | 🔄 步 0 | `backup/aikv/README.md` §设计文档; `oldmain/docs/index.md` (结构查漏); aidb 步 18 范本; WiQunTools/wiqun-kv **跳过** | 待步 4 落盘 | 纯链接 hub; 7 modules WHEN; development 按 §4a; observability-reference 按 §4e |

---

### 6. 步 4 验收清单 (design 对齐, 步 0 先锁定)

- [ ] 链到现有 **6 篇根文档** + AGENTS + ISSUES 均有效
- [ ] 链到 **7 篇** `docs/modules/*.md` 均有效
- [ ] (E1) `observability-reference.md` 可从 hub 到达
- [ ] 与 [`README.md`](aikv/README.md) **不重复长文** (无 feature 表、无架构图、无测试矩阵)
- [ ] 无 Module Skill frontmatter
- [ ] 过程/旧文档对照 **只进** `backup/INDEX.md`, aikv 无迁移史
- [ ] (若 C1) 根 README 与 hub **互相可发现**
- [ ] (若 D1) AiDb docs hub 链有效
- [ ] 完成后 PROGRESS 步 24 ✅ → **阶段 2b 完结**, 可进入 Step 3 全仓验收

---

### 7. 步 0 交付物小结

| 交付物 | 状态 |
|--------|------|
| 本章边界 (含/不含) | ✅ 上文 §1–§2 |
| 拟读文件列表 | ✅ 上文 §3 |
| INDEX 拟增行 | ✅ 上文 §5 |
| 五项决策 (development / 子目录 / 根 README / AiDb 互链 / reference) | ⏳ 待你确认 |
| 写入 aikv / 更新 PROGRESS | ❌ 步 4 确认后再做 |

---

请确认或调整:

1. **边界** §1–§2 是否 OK?
2. **`docs/development.md`**: **A1 / A2 / A3**? (默认 **A1**)
3. **子目录 README**: **B1 / B2**? (默认 **B1**)
4. **根 README 改链**: **C1 / C2 / C3**? (默认 **C1**)
5. **AiDb 互链**: **D1 / D2**? (默认 **D1**)
6. **observability-reference**: **E1 / E2**? (默认 **E1**)

回复「可以」或具体修改意见后, 我再进入 **步 1** (梳理当前文档树、链接目标与各 module WHEN 要点).

---

## 8. User

可以

---

## 9. Assistant

步 0 已确认。开始步 1：梳理当前文档树、链接目标与各 module 的 WHEN 要点。

**Tool `Read`**

```json
{
  "path": "/root/code/database/aidb/docs/README.md"
}
```


**Tool `Grep`**

```json
{
  "path": "/root/code/database/aikv/docs/modules",
  "pattern": "^## 何时读本文",
  "-A": 8
}
```


**Tool `Glob`**

```json
{
  "glob_pattern": "aikv/*.md"
}
```


**Tool `Read`**

```json
{
  "limit": 30,
  "path": "/root/code/database/aikv/docs/modules/protocol.md"
}
```


**Tool `Read`**

```json
{
  "limit": 30,
  "path": "/root/code/database/aikv/docs/modules/observability.md"
}
```


---

## 10. Assistant

**Tool `Read`**

```json
{
  "limit": 25,
  "path": "/root/code/database/aikv/ARCHITECTURE.md"
}
```


**Tool `Read`**

```json
{
  "limit": 15,
  "path": "/root/code/database/aikv/README.md",
  "offset": 78
}
```


**Tool `Shell`**

```bash
cd /root/code/database && for f in aikv/ARCHITECTURE.md aikv/DESIGN.md aikv/DEPLOYMENT.md aikv/CONTRIBUTING.md aikv/CHANGELOG.md aikv/AGENTS.md aikv/ISSUES.md aikv/docs/modules/protocol.md aikv/docs/modules/server.md aikv/docs/modules/storage.md aikv/docs/modules/commands-core.md aikv/docs/modules/commands-extended.md aikv/docs/modules/cluster.md aikv/docs/modules/observability.md aikv/docs/modules/observability-reference.md aidb/docs/README.md; do test -f "$f" && echo "OK $f" || echo "MISSING $f"; done
```


---

## 11. Assistant

## 步 1 — 读当前文档 (以 aikv 现有文档树为准)

步 0 五项决策 (**A1 / B1 / C1 / D1 / E1**) 已锁定. 本篇不读 `src/`, 以 **链目标存在性 + 各 module WHEN** 为步 1 依据.

---

### 1. 当前文档树 (hub 将服务的域)

```shell
aikv/
├── AGENTS.md                   # AI 助手; 链 README/ARCHITECTURE/.github (尚无 modules 表)
├── ARCHITECTURE.md             # 分层/数据流; 已指 docs/modules/
├── CHANGELOG.md
├── CLAUDE.md                   # → AGENTS.md
├── CONTRIBUTING.md             # hooks/CI/测试矩阵
├── DEPLOYMENT.md               # 构建/feature/CLI/集群/监控
├── DESIGN.md
├── ISSUES.md
├── README.md                   # 入口; §文档 扁平表 (无 hub 链, 步 4 C1 补)
└── docs/
    ├── README.md               # ❌ 待建 (本篇)
    └── modules/
        ├── protocol.md         # ✅
        ├── server.md           # ✅
        ├── storage.md          # ✅
        ├── commands-core.md    # ✅
        ├── commands-extended.md# ✅
        ├── cluster.md          # ✅
        ├── observability.md    # ✅
        └── observability-reference.md  # 附属; E1 挂在 observability 行
```

**链接自检**: 根文档 7 篇 + modules 7+1 + 跨仓 `aidb/docs/README.md` — **全部存在**.

---

### 2. 阅读路径 (步 4 草稿骨架)

| 场景 | 路径 |
|------|------|
| **首次了解** | [README.md](../README.md) → [ARCHITECTURE.md](../ARCHITECTURE.md) → 按需打开下方 modules |
| **改某域代码** | [按域阅读](#按域阅读-modules) WHEN → 对应 module; 跨域见 module 内「不覆盖」 |
| **构建 / 测试 / PR** | [DEPLOYMENT.md](../DEPLOYMENT.md) + [CONTRIBUTING.md](../CONTRIBUTING.md); AI 速览 [AGENTS.md](../AGENTS.md) |
| **底层 LSM/Raft** | 一句链 [../aidb/docs/README.md](../aidb/docs/README.md) (D1) |

与 aidb 步 18 范本 **同结构**, 仅 modules 7 行 + AiDb 互链.

---

### 3. 汇总文档表 (一行说明, 取自 README §文档 / 各文定位)

| 文档 | hub 表「内容」列 (拟) |
|------|----------------------|
| ARCHITECTURE.md | 分层、数据流、与 AiDb 边界 |
| DESIGN.md | 跨模块设计决策与已知限制 |
| DEPLOYMENT.md | 构建、feature、CLI、集群部署、监控 |
| CONTRIBUTING.md | hooks、CI、测试矩阵、提交/PR 规范 |
| CHANGELOG.md | 版本变更记录 |
| AGENTS.md | AI 助手与 CI 入口 |
| ISSUES.md | 待核实与已知疑点 |

与 [`aikv/README.md`](aikv/README.md) §文档 **同文案**, hub 侧 **分组呈现**, 不扩写.

---

### 4. 按域阅读 — WHEN 一句话 (步 4 modules 表草稿)

取自各 module frontmatter `Use when` + 「何时读本文」首条, 压成 hub 一行 (与 aidb 步 18 风格一致):

| Module | 何时读 (拟) |
|--------|-------------|
| [protocol.md](modules/protocol.md) | 改 `protocol/*`; RESP2/3 parse/encode、buffer/深度限制、ProtocolVersion |
| [server.md](modules/server.md) | 改 `server/{listener,connection,config}`; TCP 读写/pipeline、HELLO、ATOM 事务、`max_clients` |
| [storage.md](modules/storage.md) | 改 `storage/*`; KvStorage、MemoryEngine/AiDbEngine、TTL/StoredValue、集群数据面 Raft 写 |
| [commands-core.md](modules/commands-core.md) | 改核心数据结构命令与 Router; String~ZSet/Key/DB、WRONGTYPE、KeyLock、CROSSSLOT 前路由 |
| [commands-extended.md](modules/commands-extended.md) | 改 JSON/Lua/阻塞/MIGRATE/SAVE/INFO/CONFIG 等扩展命令与 router extended dispatch |
| [cluster.md](modules/cluster.md) | 改 `cluster/*`、`init_cluster`; MOVED/ASK、CLUSTER 子命令、slot 迁移/failover (`cluster` feature) |
| [observability.md](modules/observability.md) | 改 slowlog/latency/info/metrics; INFO/SLOWLOG/LATENCY、`/metrics` (`monitoring`); 指标表 → [observability-reference.md](modules/observability-reference.md) |

**依赖顺序** (步 4 表后一行, 对齐 design + ARCHITECTURE):

`protocol → server → storage → commands-core → commands-extended`; `cluster` 依赖 storage + aidb cluster; `observability` 横切 (改 INFO 命令 dispatch 仍见 commands-extended).

---

### 5. 与相邻文档 — 重复风险检查

| 内容 | README | 本篇 hub | 结论 |
|------|--------|----------|------|
| 特性/feature 表 | ✅ | ❌ 不写 | OK |
| 快速开始 / 示例 / e2e | ✅ | ❌ (B1) | OK |
| §文档 扁平表 | ✅ 7 modules + 根文档 | hub 分组 + WHEN | C1 加 hub 链, 表保留 |
| ARCHITECTURE 分层图 | ✅ | ❌ | OK |
| CONTRIBUTING 测试矩阵 | ✅ | 只链 | OK |
| AGENTS 按域表 | ❌ 尚无 | hub 7 行 WHEN | 互补, 步 4 不改 AGENTS |

---

### 6. 步 1 要点小结

| 项 | 结论 |
|----|------|
| 链目标 | 全部有效; 无占位文件需链 |
| `development.md` | A1: hub「构建与测试」小节双链 DEPLOYMENT + CONTRIBUTING |
| 结构范本 | 对齐 [`aidb/docs/README.md`](aidb/docs/README.md) (~40 行纯导航) |
| 步 4 附带 | C1: README §文档 首行加 hub 链 |
| 跨仓 | D1: 文末或阅读路径一句链 aidb docs hub |
| reference | E1: observability 行内链, 不增第 8 行 |

---

### 7. 步 1 交付物状态

| 交付物 | 状态 |
|--------|------|
| 文档树 + 链接有效性 | ✅ §1 |
| 阅读路径骨架 | ✅ §2 |
| 根文档表文案 | ✅ §3 |
| modules WHEN 七行草稿 | ✅ §4 |
| 重复边界检查 | ✅ §5 |
| 写入 aikv | ❌ 步 4 后再做 |

---

请确认步 1 (或指出 WHEN 措辞 / 依赖顺序 / AiDb 互链位置要改). 确认后进入 **步 2** (按序查旧文档: `backup/aikv/` → oldmain `docs/index.md` → WiQunTools/wiqun-kv 查漏).

---

## 12. User

可以

---

## 13. Assistant

步 1 已确认。开始步 2：按规程顺序查阅旧文档并分类。

**Tool `Grep`**

```json
{
  "path": "/root/code/database/AiKv-Workflow/backup/aikv",
  "pattern": "docs/|文档|README|index",
  "glob": "*.md"
}
```


**Tool `Glob`**

```json
{
  "glob_pattern": "aikv-oldmain/docs/**/*.md"
}
```


**Tool `Grep`**

```json
{
  "path": "/root/code/database/WiQunTools",
  "pattern": "docs/README|文档索引|development\\.md",
  "glob": "**/wiqun-kv*"
}
```


**Tool `Grep`**

```json
{
  "path": "/root/code/database/wiqun-kv",
  "pattern": "docs/README|index\\.md"
}
```


---

## 14. Assistant

**Tool `Read`**

```json
{
  "limit": 50,
  "path": "/root/code/database/aikv-oldmain/README.md"
}
```


**Tool `Read`**

```json
{
  "limit": 40,
  "path": "/root/code/database/aikv-oldmain/docs/development/workflow.md"
}
```


**Tool `Glob`**

```json
{
  "glob_pattern": "wiqun-kv/**/*.md"
}
```


**Tool `Grep`**

```json
{
  "path": "/root/code/database/aikv/archive/22-readme.md",
  "pattern": "oldmain|index\\.md|docs/README"
}
```


---

## 15. Assistant

**Tool `Grep`**

```json
{
  "path": "/root/code/database/aikv-oldmain/README.md",
  "pattern": "文档|docs/"
}
```


**Tool `Read`**

```json
{
  "limit": 80,
  "offset": 400,
  "path": "/root/code/database/aidb/archive/18-docs-readme.md"
}
```


**Tool `Grep`**

```json
{
  "path": "/root/code/database/WiQunTools",
  "pattern": "docs/README|文档导航|index\\.md",
  "glob": "**/wiqun-kv*"
}
```


---

## 16. Assistant

## 步 2 — 查旧文档

按 design 顺序查阅; 本篇为 **纯链接 hub**, 旧稿几乎无同名文件, 结论以 **结构/分组/遗漏链** 为主.

---

### 1. 旧文档清单与分类

| 序 | 路径 | 分类 | 处理建议 |
|----|------|------|----------|
| 1a | `AiKv-Workflow/backup/aikv/README.md` §设计文档 | **部分仍有效** | 仅 3 链: ARCHITECTURE / DESIGN / DEPLOYMENT; **无** `docs/` hub、**无** modules — 步 19–23 已扩展, 本篇补全 |
| 1b | `backup/aikv/docs/` | **无有效内容 — 跳过** | 仅有 superpowers plans/specs; **无** `docs/README.md` |
| 2a | `aikv-oldmain/docs/index.md` | **已过时** | 旧 docs 树 (guide / development/api / architecture / archive); 分组思路可借鉴, 链接 **不可回迁** |
| 2b | `aikv-oldmain/README.md` §文档 | **已过时** | 链 `docs/DEVELOPMENT_PLAN.md`、`docs/API.md` 等 **当前不存在** 的路径; 步 22 已判不迁 |
| 2c | `aikv-oldmain/docs/guide/*` | **已过时/已链出** | quick start / 部署 / 最佳实践 → 步 22 `README.md` + 步 21 `DEPLOYMENT.md`; hub **不重复** |
| 2d | `aikv-oldmain/docs/development/api/01-commands.md` | **已过时** | 100+ 命令大全; 新结构由 `commands-core` + `commands-extended` modules 承担 — hub 只链 module |
| 2e | `aikv-oldmain/docs/development/api/02-cluster-api.md` | **已过时** | CLUSTER 26+ 命令详表 → `cluster.md` module; hub 只链 |
| 2f | `aikv-oldmain/docs/development/architecture/*` | **已过时/已消化** | 01-overview → 步 19 ARCHITECTURE; 03-cluster → cluster module; 04-storage 已归档; 05-lua → commands-extended |
| 2g | `aikv-oldmain/docs/development/workflow.md` | **已过时** | WiQunTools 式 22 步流水线 + `scripts/test_*.sh` — 现仓无; 步 23 已链 CONTRIBUTING |
| 2h | `aikv-oldmain/docs/guide/03-troubleshooting.md` | **已过时** | 故障排查旧叙事; 新体系无专章 — **不建** troubleshooting.md; 必要时用户走 ISSUES + DEPLOYMENT |
| 3 | WiQunTools `wiqun-kv-inventory/` | **无有效内容 — 跳过** | 模块 inventory, 无文档导航专章 (与步 22 一致) |
| 4 | `wiqun-kv/` | **无有效内容 — 跳过** | 无 `docs/README.md`; 根文档与 backup 重叠 |

**结论:** 无旧稿可「回迁」; 本篇需 **从零写**, 参考物仅为 oldmain **分组思路** (入门 → 汇总 → 域文档 → 贡献), 不复制链接列表.

---

### 2. oldmain 导航 vs 新结构 (可借鉴 / 丢弃)

| oldmain 分组 | 代表链 | 新文档归属 | 本篇 hub |
|--------------|--------|------------|----------|
| 新手入门 | guide/01-quick-start, 02-deployment | [`README.md`](../README.md) + [`DEPLOYMENT.md`](../DEPLOYMENT.md) | 阅读路径链回, 不重复 quick start |
| API 参考 | development/api/01-commands, 02-cluster-api | `commands-core` + `commands-extended` + `cluster` modules | **modules WHEN 表** |
| 架构设计 | development/architecture/* | 根 ARCHITECTURE / DESIGN + 7 modules | 根文档表 + modules 表 |
| 故障排除 | guide/03-troubleshooting | 无专章 | **不写** troubleshooting 链 (B1 精神: 不扩主体系) |
| 最佳实践 | guide/04-best-practices | DESIGN / DEPLOYMENT 已吸收要点 | 不单独链 |
| 开发工作流 | development/workflow.md | CONTRIBUTING (步 23) | A1: 链 CONTRIBUTING + DEPLOYMENT |
| 归档 / 计划 | docs/archive/*, plan.md | **丢弃** | 不写 |
| index 文档统计 / GitHub badge | index.md 底部 | **丢弃** | 不写 |
| 底层 LSM/Raft | oldmain 混写在 architecture | [`../aidb/docs/README.md`](../aidb/docs/README.md) | D1 一句互链 |

**不迁入:** v0.1.0/90% 进度、100+ 命令枚举表、`aikv-tool`、`--config config.toml`、性能 benchmark 表、docs/archive 树、独立 API 大全页.

---

### 3. backup 旧 README §设计文档 (对照)

```131:135:AiKv-Workflow/backup/aikv/README.md
## 设计文档

- [ARCHITECTURE.md](ARCHITECTURE.md)
- [DESIGN.md](DESIGN.md)
- [DEPLOYMENT.md](DEPLOYMENT.md)
```

| 项 | 步 23 后现状 | 本篇需补 |
|----|--------------|----------|
| 根汇总 3 篇 | ✅ 已存在且更完整 | + CONTRIBUTING, CHANGELOG, AGENTS, ISSUES |
| modules 7 篇 | ❌ 旧稿无 | ✅ WHEN 表 (步 1 草稿) |
| 测试/构建 | 旧 CONTRIBUTING 内嵌 (已过时) | A1: 链 DEPLOYMENT + CONTRIBUTING |
| docs 域入口 | 无 | ✅ 本篇即 hub |
| observability-reference | 旧稿无 | E1: 挂 observability 行 |

---

### 4. 步 2 消化 / INDEX 处理 (步 4 末执行)

| 旧文档 | 步 4 后处理 |
|--------|-------------|
| `backup/aikv/README.md` §设计文档 | **不删** (backup 归档); INDEX 记「结构参考, 已由 hub supersede」 |
| `aikv-oldmain/docs/index.md` 及 guide/development/* | **不删** (只读源); 无对应 **aikv 仓内** 源文件需删 |
| WiQunTools / wiqun-kv | 无动作 |

本篇 **不触发** oldmain 源侧删除 (design: 汇总章不对应单一旧文件迁移).

---

### 5. ISSUES

本篇为导航, 步 2 **无新 ISSUE**. 已有 23 条 (ISSUE-001~023) 在 hub **不展开**, 仅保留 ISSUES 一行链 (与步 1 一致).

---

### 6. 拟 INDEX 更新 (步 4 一并写入)

在步 0 拟行基础上补充步 2 结论:

| 新文档章节 | 状态 | 参考的旧文档 | 处理 | 备注 |
|------------|------|--------------|------|------|
| `docs/README.md` (导航) | 🔄 步 2 | `backup/aikv/README.md` §设计文档; `oldmain/docs/index.md` + `README.md` §文档 (分组查漏); WiQunTools/wiqun-kv **跳过** | 待步 4 落盘 | 无旧稿回迁; 7 modules WHEN; A1/B1/C1/D1/E1 |

---

### 7. 步 2 交付物状态

| 交付物 | 状态 |
|--------|------|
| 旧文档清单 + 分类 | ✅ §1 |
| oldmain → 新结构映射 | ✅ §2 |
| backup 对照 + 本篇需补项 | ✅ §3 |
| 消化/INDEX 处理 | ✅ §4 |
| ISSUES | ✅ 无新增 §5 |
| 写入 aikv | ❌ 步 4 后再做 |

---

请确认步 2 (或指出 oldmain 某分组是否应在 hub 保留一行, 例如 troubleshooting). 确认后进入 **步 3** (模块级: 与 aidb 步 18 类似, 对比 oldmain 导航形态 vs 新 hub 结构, 确认表述建议).

---

## 17. User

可以

---

## 18. Assistant

步 2 已确认。开始步 3：对照 oldmain 导航形态与新 hub 结构，给出模块级差异与表述建议。

**Tool `Read`**

```json
{
  "limit": 120,
  "offset": 478,
  "path": "/root/code/database/aidb/archive/18-docs-readme.md"
}
```


**Tool `Read`**

```json
{
  "limit": 30,
  "path": "/root/code/database/aikv/ARCHITECTURE.md",
  "offset": 24
}
```


---

## 19. Assistant

## 步 3 — 对比旧结构 (模块级)

旧侧 **无** `aikv/docs/README.md`; 对照对象为 **oldmain `docs/index.md` + `README.md` §文档**, 以及已落盘的 **aidb `docs/README.md`** (2a 范本, 非旧稿回迁).

---

### 1. 导航形态差异表

| 维度 | oldmain (`docs/index.md` + guide/development) | 当前 aikv (步 19–23 后) | 本篇 `docs/README.md` 应如何写 |
|------|-----------------------------------------------|-------------------------|--------------------------------|
| **Hub 位置** | `docs/index.md` (docs 子树内 mega 索引) | 根 `README.md` §文档 扁平表 (7 modules + 根文档) | **`docs/` 域 hub**; 根 README 加 hub 链 (C1) |
| **汇总文档** | 分散在 guide + development/architecture | 根 ARCHITECTURE / DESIGN / DEPLOYMENT / CONTRIBUTING / CHANGELOG | 相对链 `../*.md`; **分组表**, 不嵌正文 |
| **API / 命令** | `development/api/01-commands` + `02-cluster-api` (100+ 命令) | `commands-core` + `commands-extended` + `cluster` modules | **modules WHEN 表**; 不链 API 大全 |
| **实现细节** | architecture/01~05 单篇 | 7 篇 `docs/modules/*.md` (Skill) | WHEN 一行; 不摘 module 正文 |
| **开发/测试** | `development/workflow.md` (22 步 + scripts) | `CONTRIBUTING.md` (+ DEPLOYMENT 构建) | A1: 一行链两篇; **不建** `development.md` |
| **运维/部署** | guide/02-deployment, 03-troubleshooting | `DEPLOYMENT.md` | hub 链 DEPLOYMENT; **不建** troubleshooting 专章 |
| **底层引擎** | architecture 混写 LSM/Raft | AiDb 独立仓库 + modules 内「不覆盖」链 | D1: 一句链 `../aidb/docs/README.md` |
| **规划/归档** | docs/archive/*, plan.md | **已废弃** | **不写** |
| **体量** | index ~70 行 + tree + 统计 + 外链 | README ~110 行 | 目标 **~45–55 行**, 纯表 + 短引言 (略长于 aidb, 因 7 modules) |
| **AI 路由** | 无 | modules frontmatter + AGENTS (尚无 WHEN 表) | hub 提供 **WHEN 一行**; AGENTS 仍链 ARCHITECTURE, 不强制改 |

---

### 2. 路径映射 (oldmain → 新文档, hub 正文不写对照表)

| oldmain 典型链 | 新归属 | hub 动作 |
|----------------|--------|----------|
| `docs/index.md` | **`docs/README.md`** + 根 `README.md` | 双入口 (C1); hub 不复制 tree/统计 |
| `guide/01-quick-start` | 根 `README.md` §快速开始 | 阅读路径链回 README |
| `guide/02-deployment` | `DEPLOYMENT.md` | 根文档表 1 行 |
| `guide/03-troubleshooting` | — (无专章) | **不写**; 用户走 ISSUES / DEPLOYMENT |
| `guide/04-best-practices` | `DESIGN.md` / `DEPLOYMENT.md` | 不单独链 |
| `development/api/01-commands` | `commands-core` + `commands-extended` | modules 表 2 行 |
| `development/api/02-cluster-api` | `cluster.md` | modules 表 1 行 |
| `development/architecture/01-overview` | `ARCHITECTURE.md` | 根文档表 1 行 |
| `development/architecture/03-cluster` | `cluster.md` + ARCHITECTURE §集群 | module 链 |
| `development/architecture/04-storage` | `storage.md` (旧稿已归档) | module 链 |
| `development/architecture/05-lua` | `commands-extended.md` | module 链 |
| `development/workflow.md` | `CONTRIBUTING.md` | A1 构建/测试行 |
| README §文档 (DEVELOPMENT_PLAN 等) | 不存在路径 | **忽略**; 不恢复旧链 |
| Prometheus 指标详表 | `observability-reference.md` | E1: observability 行内链 |

**表述建议:** 开篇写「汇总在仓库根目录, 域实现见 `modules/`」; **不** 提 oldmain、`index.md` 或迁移.

---

### 3. 与 aidb `docs/README.md` 的形态对比 (2a 范本)

| 项 | aidb 步 18 落盘 | aikv 本篇 (已定) |
|----|-----------------|------------------|
| 结构 | 引言 + 阅读路径 + 汇总表 + modules + 构建 + 待核实 | **同结构** |
| modules 行数 | 5 | **7** (+ observability-reference 行内) |
| 跨产品链 | ARCHITECTURE 表内「与 AiKv 边界」 | D1: 独立一句链 **aidb docs hub** |
| 依赖顺序 | engine → engine-storage; 其余独立 | protocol → server → storage → commands-*; cluster / observability 见步 1 |
| 子目录 | B1 不写 | B1 不写 (examples/e2e 留根 README) |
| `development.md` | A1 不建 | A1 不建 |

aikv hub 为 aidb 范本的 **同构扩展**, 非 second INDEX.

---

### 4. 步 3 结论 (文档应如何表述)

1. **角色:** `docs/` 的 **开发文档入口** — 不是第二份 README, 不是 oldmain 式 API 索引.
2. **结构 (步 4 草稿骨架):**
   - `# AiKv 开发文档` + 1–2 句 (入口见 `../README.md`)
   - `## 阅读路径` — 3 bullet (步 1 §2); 可选第 4 条「底层 LSM/Raft → aidb docs」
   - `## 汇总文档` — 7 行表 (步 1 §3)
   - `## 按域阅读 (modules)` — 7 行 WHEN (步 1 §4) + 依赖顺序一行
   - `## 构建与测试` — A1 双链 DEPLOYMENT + CONTRIBUTING
   - `## 待核实` — 一行链 ISSUES
   - (D1) 可选独立 `## 与 AiDb` 一句, 或并入阅读路径 — **建议并入阅读路径**, 保持与 aidb 同节数
3. **不写:** mermaid、feature 表、测试矩阵、命令枚举、troubleshooting、子目录 README (B1)、`development.md` (A1).
4. **C1 (步 4 顺带):** `README.md` §文档 **表前** 加 hub 链 (对齐 aidb 措辞).
5. **ISSUES:** 无新条目; 步 3 无代码/行为差异.

---

### 5. 拟 INDEX 更新 (步 4 写入)

| 新文档章节 | 状态 | 参考的旧文档 | 处理 | 备注 |
|------------|------|--------------|------|------|
| `docs/README.md` (导航) | 🔄 步 3 | oldmain `docs/index.md` + README §文档 (形态); backup README §设计文档 (3 链); aidb 步 18 范本; WiQunTools/wiqun-kv 跳过 | 待步 4 落盘 | 模块级; oldmain docs/index → docs/README + 根 README 双入口; A1/B1/C1/D1/E1 |

---

### 6. 步 3 交付物小结

| 交付物 | 状态 |
|--------|------|
| 模块级差异表 | ✅ §1 |
| oldmain 路径映射 (内部用, 不进 aikv) | ✅ §2 |
| 与 aidb 范本对照 | ✅ §3 |
| 文档表述建议 | ✅ §4 |
| 新 ISSUES | 无 |
| 写入 aikv | ❌ 步 4 草稿待确认 |

---

请确认步 3. 确认后进入 **步 4 — 正文草稿** (含 `aikv/docs/README.md` 全文 + C1 README 改动一行; **不落盘**, 等你确认后再写文件并更新 INDEX/PROGRESS).

---

## 20. User

可以

---

## 21. Assistant

**Tool `Read`**

```json
{
  "limit": 15,
  "offset": 62,
  "path": "/root/code/database/aidb/README.md"
}
```


**Tool `Read`**

```json
{
  "limit": 30,
  "offset": 78,
  "path": "/root/code/database/aikv/README.md"
}
```


---

## 22. Assistant

## 步 4 — 正文草稿 (待确认, 未落盘)

以下为 **拟写入** 的全文 + **C1** README 一行改动. 确认后再落盘并更新 INDEX/PROGRESS.

---

### A. `aikv/docs/README.md` (新建)

```markdown
# AiKv 开发文档

`docs/` 域的开发文档导航. 项目介绍与快速开始见 [README.md](../README.md).

## 阅读路径

- **首次了解** — [README.md](../README.md) → [ARCHITECTURE.md](../ARCHITECTURE.md) → 按需打开下方 modules
- **改某域代码** — 查 [按域阅读](#按域阅读-modules) WHEN → 对应 module; 跨域边界见 module 内「不覆盖」
- **构建 / 测试 / PR** — [DEPLOYMENT.md](../DEPLOYMENT.md) + [CONTRIBUTING.md](../CONTRIBUTING.md); AI 助手速览见 [AGENTS.md](../AGENTS.md)
- **底层 LSM / Raft** — 存储与共识在 sibling [AiDb](../aidb/docs/README.md); AiKv 侧重 RESP 与命令层

## 汇总文档

| 文档 | 内容 |
|------|------|
| [ARCHITECTURE.md](../ARCHITECTURE.md) | 分层、数据流、与 AiDb 边界 |
| [DESIGN.md](../DESIGN.md) | 跨模块设计决策与已知限制 |
| [DEPLOYMENT.md](../DEPLOYMENT.md) | 构建、feature、CLI、集群部署、监控 |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | hooks、CI、测试矩阵、提交/PR 规范 |
| [CHANGELOG.md](../CHANGELOG.md) | 版本变更记录 |
| [AGENTS.md](../AGENTS.md) | AI 助手与 CI 入口 |
| [ISSUES.md](../ISSUES.md) | 待核实与已知疑点 |

## 按域阅读 (modules)

| Module | 何时读 |
|--------|--------|
| [protocol.md](modules/protocol.md) | 改 `protocol/*`; RESP2/3 parse/encode、buffer/深度限制、ProtocolVersion |
| [server.md](modules/server.md) | 改 `server/{listener,connection,config}`; TCP 读写/pipeline、HELLO、ATOM 事务、`max_clients` |
| [storage.md](modules/storage.md) | 改 `storage/*`; KvStorage、MemoryEngine/AiDbEngine、TTL/StoredValue、集群数据面 Raft 写 |
| [commands-core.md](modules/commands-core.md) | 改核心数据结构命令与 Router; String~ZSet/Key/DB、WRONGTYPE、KeyLock、CROSSSLOT 前路由 |
| [commands-extended.md](modules/commands-extended.md) | 改 JSON/Lua/阻塞/MIGRATE/SAVE/INFO/CONFIG 等扩展命令与 router extended dispatch |
| [cluster.md](modules/cluster.md) | 改 `cluster/*`、`init_cluster`; MOVED/ASK、CLUSTER 子命令、slot 迁移/failover (`cluster` feature) |
| [observability.md](modules/observability.md) | 改 slowlog/latency/info/metrics; INFO/SLOWLOG/LATENCY、`/metrics` (`monitoring`); 指标表 → [observability-reference.md](modules/observability-reference.md) |

依赖顺序: protocol → server → storage → commands-core → commands-extended; cluster 依赖 storage + aidb cluster; observability 横切 (INFO 命令 dispatch 仍见 commands-extended).

## 构建与测试

构建、Cargo feature 与完整测试矩阵见 [DEPLOYMENT.md](../DEPLOYMENT.md) 与 [CONTRIBUTING.md](../CONTRIBUTING.md).

## 待核实

详情见 [ISSUES.md](../ISSUES.md) (module 内一行引用, 不在此展开).
```

**篇幅:** 约 42 行 (含空行), 符合 ~45–55 行目标.

---

### B. C1 — `aikv/README.md` §文档 (表前加一行)

**现:**

```markdown
## 文档

| 文档 | 内容 |
```

**拟改为:**

```markdown
## 文档

开发文档 hub: [docs/README.md](docs/README.md) (汇总文档 + modules WHEN 路由).

| 文档 | 内容 |
```

其余扁平表 **保留不动**.

---

### C. 步 4 验收自检 (对照 design)

| 项 | 状态 |
|----|------|
| 链到 6 篇根文档 + AGENTS + ISSUES | ✅ 草稿已含 |
| 链到 7 篇 modules | ✅ |
| observability-reference 可从 hub 到达 | ✅ (E1, observability 行内) |
| 与 README 不重复长文 | ✅ 无 feature/架构/测试矩阵 |
| 无 Module Skill frontmatter | ✅ |
| AiDb docs hub 链有效 | ✅ `../aidb/docs/README.md` |
| A1 不建 development.md | ✅ |
| B1 不写子目录 README | ✅ |
| 过程不进 aikv | ✅ 仅 backup INDEX/PROGRESS 待更新 |

---

### D. 确认后行政更新 (预览, 尚未执行)

**INDEX** — aikv 表新增/更新行:

| 新文档章节 | 状态 | 参考的旧文档 | 处理 | 备注 |
|------------|------|--------------|------|------|
| `docs/README.md` (导航) | ✅ | backup README §设计文档; oldmain `docs/index.md` + README §文档; aidb 步 18 范本; WiQunTools/wiqun-kv 跳过 | 已写 `aikv/docs/README.md` | 纯链接 hub; 7 modules WHEN; A1/B1/C1/D1/E1 |

**PROGRESS** — 更新:

- 步 24 `docs/README.md` → ✅ 完成, 日期 2026-06-18
- Step 0 骨架: aikv `docs/README.md` 勾选; `docs/development.md` 注明 A1 未建 (hub 链 DEPLOYMENT + CONTRIBUTING)
- 阶段 2b 全部 ✅
- Step 3: 阶段 2b aikv 根文档与 docs/README ✅

---

请确认步 4 草稿 (或指出 WHEN 措辞 / 阅读路径 / C1 文案要改). 回复「可以」或修改意见后, 我再 **落盘** `aikv/docs/README.md`、改 README 一行、更新 INDEX/PROGRESS.

---

## 23. User

可以

---

## 24. Assistant

正在落盘步 4 草稿并更新 INDEX/PROGRESS。

**Tool `Write`**

```json
{
  "path": "/root/code/database/aikv/docs/README.md",
  "contents": "# AiKv 开发文档\n\n`docs/` 域的开发文档导航. 项目介绍与快速开始见 [README.md](../README.md).\n\n## 阅读路径\n\n- **首次了解** — [README.md](../README.md) → [ARCHITECTURE.md](../ARCHITECTURE.md) → 按需打开下方 modules\n- **改某域代码** — 查 [按域阅读](#按域阅读-modules) WHEN → 对应 module; 跨域边界见 module 内「不覆盖」\n- **构建 / 测试 / PR** — [DEPLOYMENT.md](../DEPLOYMENT.md) + [CONTRIBUTING.md](../CONTRIBUTING.md); AI 助手速览见 [AGENTS.md](../AGENTS.md)\n- **底层 LSM / Raft** — 存储与共识在 sibling [AiDb](../aidb/docs/README.md); AiKv 侧重 RESP 与命令层\n\n## 汇总文档\n\n| 文档 | 内容 |\n|------|------|\n| [ARCHITECTURE.md](../ARCHITECTURE.md) | 分层、数据流、与 AiDb 边界 |\n| [DESIGN.md](../DESIGN.md) | 跨模块设计决策与已知限制 |\n| [DEPLOYMENT.md](../DEPLOYMENT.md) | 构建、feature、CLI、集群部署、监控 |\n| [CONTRIBUTING.md](../CONTRIBUTING.md) | hooks、CI、测试矩阵、提交/PR 规范 |\n| [CHANGELOG.md](../CHANGELOG.md) | 版本变更记录 |\n| [AGENTS.md](../AGENTS.md) | AI 助手与 CI 入口 |\n| [ISSUES.md](../ISSUES.md) | 待核实与已知疑点 |\n\n## 按域阅读 (modules)\n\n| Module | 何时读 |\n|--------|--------|\n| [protocol.md](modules/protocol.md) | 改 `protocol/*`; RESP2/3 parse/encode、buffer/深度限制、ProtocolVersion |\n| [server.md](modules/server.md) | 改 `server/{listener,connection,config}`; TCP 读写/pipeline、HELLO、ATOM 事务、`max_clients` |\n| [storage.md](modules/storage.md) | 改 `storage/*`; KvStorage、MemoryEngine/AiDbEngine、TTL/StoredValue、集群数据面 Raft 写 |\n| [commands-core.md](modules/commands-core.md) | 改核心数据结构命令与 Router; String~ZSet/Key/DB、WRONGTYPE、KeyLock、CROSSSLOT 前路由 |\n| [commands-extended.md](modules/commands-extended.md) | 改 JSON/Lua/阻塞/MIGRATE/SAVE/INFO/CONFIG 等扩展命令与 router extended dispatch |\n| [cluster.md](modules/cluster.md) | 改 `cluster/*`、`init_cluster`; MOVED/ASK、CLUSTER 子命令、slot 迁移/failover (`cluster` feature) |\n| [observability.md](modules/observability.md) | 改 slowlog/latency/info/metrics; INFO/SLOWLOG/LATENCY、`/metrics` (`monitoring`); 指标表 → [observability-reference.md](modules/observability-reference.md) |\n\n依赖顺序: protocol → server → storage → commands-core → commands-extended; cluster 依赖 storage + aidb cluster; observability 横切 (INFO 命令 dispatch 仍见 commands-extended).\n\n## 构建与测试\n\n构建、Cargo feature 与完整测试矩阵见 [DEPLOYMENT.md](../DEPLOYMENT.md) 与 [CONTRIBUTING.md](../CONTRIBUTING.md).\n\n## 待核实\n\n详情见 [ISSUES.md](../ISSUES.md) (module 内一行引用, 不在此展开).\n"
}
```


**Tool `StrReplace`**

```json
{
  "path": "/root/code/database/aikv/README.md",
  "old_string": "## 文档\n\n| 文档 | 内容 |",
  "new_string": "## 文档\n\n开发文档 hub: [docs/README.md](docs/README.md) (汇总文档 + modules WHEN 路由).\n\n| 文档 | 内容 |"
}
```


**Tool `StrReplace`**

```json
{
  "path": "/root/code/database/AiKv-Workflow/backup/INDEX.md",
  "old_string": "| `CHANGELOG.md` | ✅ | `backup/aikv/CHANGELOG.md`; wiqun-kv (重复跳过) | 已写 `aikv/CHANGELOG.md` | 保留 0.0.1–0.10.5; [Unreleased] 空 |\n\n### aikv 旧文档来源速查",
  "new_string": "| `CHANGELOG.md` | ✅ | `backup/aikv/CHANGELOG.md`; wiqun-kv (重复跳过) | 已写 `aikv/CHANGELOG.md` | 保留 0.0.1–0.10.5; [Unreleased] 空 |\n| `docs/README.md` (导航) | ✅ | `backup/aikv/README.md` §设计文档; `oldmain/docs/index.md` + README §文档 (分组查漏); aidb 步 18 范本; WiQunTools/wiqun-kv **跳过** | 已写 `aikv/docs/README.md` | 纯链接 hub; 7 modules WHEN; A1/B1/C1/D1/E1 |\n\n### aikv 旧文档来源速查"
}
```


**Tool `StrReplace`**

```json
{
  "path": "/root/code/database/AiKv-Workflow/backup/PROGRESS.md",
  "old_string": "- [ ] aikv: 新建根目录文档 (同上) — 可先占位\n- [ ] aikv: `docs/README.md`, `docs/development.md`\n- [ ] aikv: `docs/modules/` — protocol, server, storage, commands-core, commands-extended, cluster, observability (7 篇)",
  "new_string": "- [x] aikv: 新建根目录文档 (README, ARCHITECTURE, DESIGN, DEPLOYMENT, CHANGELOG, CONTRIBUTING)\n- [x] aikv: `docs/README.md` (hub); `docs/development.md` 未建 (hub 链 DEPLOYMENT + CONTRIBUTING)\n- [x] aikv: `docs/modules/` — protocol, server, storage, commands-core, commands-extended, cluster, observability (7 篇)"
}
```


**Tool `StrReplace`**

```json
{
  "path": "/root/code/database/AiKv-Workflow/backup/PROGRESS.md",
  "old_string": "| 24 | `docs/README.md` (导航) | ⬜ 待开始 | |",
  "new_string": "| 24 | `docs/README.md` (导航) | ✅ 完成 | 2026-06-18 |"
}
```


**Tool `StrReplace`**

```json
{
  "path": "/root/code/database/AiKv-Workflow/backup/PROGRESS.md",
  "old_string": "- [ ] 阶段 2b: aikv 根文档与 `docs/README.md` 全部 ✅",
  "new_string": "- [x] 阶段 2b: aikv 根文档与 `docs/README.md` 全部 ✅"
}
```


---

## 25. Assistant

**Tool `StrReplace`**

```json
{
  "new_string": "**当前阶段**: 阶段 2b ✅ 完成 — 可进入 Step 3 全仓验收",
  "old_string": "**当前阶段**: 阶段 2a ✅ 完成 — 可进入阶段 2b (aikv 汇总, 步 19+)",
  "path": "/root/code/database/AiKv-Workflow/backup/PROGRESS.md"
}
```


**Tool `StrReplace`**

```json
{
  "new_string": "| 23 | `CONTRIBUTING.md` / `CHANGELOG.md` | ✅ 完成 | 2026-06-18 |\n| 24 | `docs/README.md` (导航) | ✅ 完成 | 2026-06-18 |\n\n`docs/development.md`: 未单独创建; hub 链 DEPLOYMENT + CONTRIBUTING (步 24 A1).\n\n---",
  "old_string": "| 23 | `CONTRIBUTING.md` / `CHANGELOG.md` | ✅ 完成 | 2026-06-18 |\n| 24 | `docs/README.md` (导航) | ✅ 完成 | 2026-06-18 |\n\n---",
  "path": "/root/code/database/AiKv-Workflow/backup/PROGRESS.md"
}
```


**Tool `Shell`**

```bash
cd /root/code/database && for f in aikv/docs/README.md aikv/ARCHITECTURE.md aikv/DESIGN.md aikv/DEPLOYMENT.md aikv/CONTRIBUTING.md aikv/CHANGELOG.md aikv/AGENTS.md aikv/ISSUES.md aikv/docs/modules/protocol.md aikv/docs/modules/server.md aikv/docs/modules/storage.md aikv/docs/modules/commands-core.md aikv/docs/modules/commands-extended.md aikv/docs/modules/cluster.md aikv/docs/modules/observability.md aikv/docs/modules/observability-reference.md aidb/docs/README.md; do test -f "$f" && echo "OK $f" || echo "MISSING $f"; done
```


---

## 26. Assistant

## 步 4 已落盘

**aikv 汇总 (步 19–24) 全部完成**; 阶段 2b 完结.

### 写入 / 更新

| 文件 | 动作 |
|------|------|
| [`aikv/docs/README.md`](aikv/docs/README.md) | **新建** — 阅读路径、汇总表、7 篇 modules WHEN、构建/测试、待核实 |
| [`aikv/README.md`](aikv/README.md) | **C1** — §文档 表前加 hub 链 |
| [`AiKv-Workflow/backup/INDEX.md`](AiKv-Workflow/backup/INDEX.md) | aikv `docs/README.md` 行 ✅ |
| [`AiKv-Workflow/backup/PROGRESS.md`](AiKv-Workflow/backup/PROGRESS.md) | 步 24 ✅; 阶段 2b ✅; Step 0 骨架 aikv 项勾选; `development.md` A1 说明 |

### 验收

- hub 内 **15 个相对链** + 跨仓 `../aidb/docs/README.md` — 目标文件均存在
- 无 frontmatter; 未建 `docs/development.md` (A1)
- 未写 examples/e2e/tests (B1)

### 进度

```
阶段 1 (modules 12)     ✅
阶段 2a (aidb 汇总 6)   ✅
阶段 2b (aikv 汇总 6)   ✅  ← 本篇完结
Step 3 全仓验收         ⬜
Step 4 删旧仓库         ⬜
```

下一步若要继续, 可开 **Step 3 验收** (对照 design.md 验收项逐条核对). 需要我接着做 Step 3 时说一声即可.

---

## 27. User

使用 vibe-coding/scripts/archive-cursor-session.sh  帮我保存当前对话到 archive 里面 带 tool 和不带 tool 的都保存

---

## 28. Assistant

**Tool `Read`**

```json
{
  "path": "/root/code/database/vibe-coding/scripts/archive-cursor-session.sh"
}
```


**Tool `Glob`**

```json
{
  "glob_pattern": "**/archive-cursor-session*"
}
```

