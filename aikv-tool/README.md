## 现代分布式 AiKv 存储管理工具

本仓库为 **aikv-tool**, 在不破坏 [aikv-toolchain](https://github.com/AiKv/aikv-toolchain) 原工具的前提下, 对工具链进行重构。当前以 **CLI 为主**, 等功能完善后再实现 TUI。

旨在提供类 **Docker / kubectl** 的极致操作体验, 支持从本地开发、集群部署到生产运维的全生命周期管理, **严格遵循 XDG Base Directory 规范**。

## 快速开始

```bash
# 安装工具
cargo install --path aikv-tool

# 快速构建和部署服务
ak quick
```

```bash
Usage:  ak quick [OPTIONS]

Options:
  -m,  --mode <MODE>    运行目标: bin 或 docker
  -t,  --topo <TOPO>    部署模式: single 或 cluster
  -n,  --nodes <N>       节点总数(纯节点模式, 与 -s/-r 互斥)
  -s,  --shards <N>     分片数
  -r,  --replicas <N>   每分片副本数(需与 -s 同用)
  -i,  --image <IMAGE>  Docker 镜像(build + up 共用)
  -f,  --force          强制重新部署: down -v → clean → build(force) → up
       --release        Release 模式构建(仅 bin)
  -h,  --help           查看帮助
```

#### 示例

```bash
# 使用配置默认 mode/topo，构建并启动
ak quick

# 指定 docker 单节点
ak quick -m docker -t single

# 指定 docker 集群: 3 分片 1 副本
ak quick -m docker -t cluster -s 3 -r 1

# 纯节点模式 5 节点
ak quick -m docker -t cluster -n 5

# 强制重新部署(先停服务、清状态，再构建并启动)
ak quick -f
```

## 常用命令

```bash
# 构建 AiKv 二进制
ak build

# 启动服务(使用配置文件默认模式)
ak up

# 查看服务状态
ak ps

# 查看日志
ak logs -f

# 停止服务
ak down

# 启动 OTel 观测栈
ak otels up
```

## 配置文件

**为了方便使用和保持灵活，给命令提供了配置文件，用于设置一些默认值。**

>大部分命令支持 -m/--mode 和 -t/--topo 参数，类似 kubectl 中的 -n/--namespace。
>这两个参数主要用于选择 aikv 服务是以二进制程序还是容器运行，单机部署还是集群部署。
>在配置文件中设定后 quick,build,up,down,restart 等相关命令默认使用该参数模式。

ak 命令参数优先级说明:

1. 明文执行
2. 配置文件参数
3. 命令默认值

ak 按以下优先级加载配置:

4. 当前目录向上查找的 `ak.toml`(项目级配置)
5. `~/.config/ak/ak.toml`(全局配置, XDG)

```toml
schema_version = 1 # 配置格式版本

[project]
root = ".." # aikv 项目路径

[defaults]
mode = "docker" # 使用的服务模式
topo = "single" # 服务的部署模式
port = 6379     # 使用的默认端口
docker_image = "aikv:latest" # 使用的默认镜像
```

---

## 命令参考

```bash
Usage:  ak <COMMAND>

Commands:
  quick    构建并部署服务
  build    构建程序或镜像
  up       启动服务
  down     删除服务
  restart  重启服务
  logs     查看服务日志
  ps       查看服务
  config   设置配置
  otels    管理 OTel 可观测性栈
  clean    清除运行时状态和日志
```

### 编译源代码或构建镜像

```bash
Usage:  ak build [OPTIONS]

Options:
  -m,  --mode <MODE>    运行目标: bin 或 docker
  -t,  --topo <TOPO>    部署模式: single 或 cluster (cluster 仅 docker 模式下生效)
  -i,  --image <IMAGE>  指定镜像名(仅 docker 模式下生效)
  -f,  --force          强制构建, 会覆盖掉已有程序或镜像
  -r,  --release        构建程序为 Release 模式(仅 bin 模式下生效)
  -h,  --help           查看帮助
```

#### 示例

```bash
# 编译二进制(默认或 -m bin)
ak build
ak build -m bin -r

# 构建 Docker 镜像
ak build -m docker
ak build -m docker -f
ak build -m docker -i myreg/aikv:v1.0.0

# 集群拓扑镜像
ak build -m docker -t cluster
```

---

### 启动服务

```bash
Usage:  ak up [OPTIONS]

Options:
  -m,  --mode <MODE>          运行目标: bin 或 docker
  -t,  --topo <TOPO>          部署模式: single 或 cluster (cluster 仅 docker 模式下生效)
  -n,  --nodes <NODES>        启动的节点数 (仅启动集群节点不初始化,  与 --shards/--replicas 互斥)
  -s,  --shards <SHARDS>      集群分片数 (即 masters 数,  与 -n 互斥)
  -r,  --replicas <REPLICAS>  分配副本数 (即 slaves 数,  需与 -s 同用)
  -i,  --image <IMAGE>        执行使用的镜像 (默认为配置文件镜像 aikv: latest)
  -h,  --help                 查看帮助
```

#### 示例

```bash
# 使用配置文件默认模式启动
ak up

# 启动本地二进制(后台)
ak up -m bin

# 启动单节点 Docker 容器
ak up -m docker -t single

# 启动 3 分片 1 副本的集群
ak up -m docker -t cluster -s 3 -r 1

# 启动 4 节点的纯节点集群
ak up -m docker -t cluster -n 4

# 使用指定镜像
ak up -m docker -i myregistry/aikv:v2.0
```

---

### 停止并移除服务

```bash
Usage:  ak down [OPTIONS]

Options:
  -m,  --mode <MODE>     运行目标: bin 或 docker
  -t,  --topo <TOPO>     部署模式: single 或 cluster (cluster 仅 docker 模式下生效)
  -v,  --remove-volumes  同时删除数据卷(Docker)
  -h,  --help            查看帮助
```

#### 示例

```bash
# 停止当前服务
ak down

# 停止本地二进制
ak down -m bin

# 停止 Docker 集群并删除数据卷
ak down -m docker -t cluster -v
```

---

### 重启服务

```bash
Usage:  ak restart [OPTIONS]

Options:
  -m,  --mode <MODE>  运行目标: bin 或 docker
  -t,  --topo <TOPO> 部署模式: single 或 cluster (cluster 仅 docker 模式下生效)
  -i,  --init         深度重置(清理数据后重新启动)
  -h,  --help         查看帮助
```

#### 示例

```bash
# 原地重启
ak restart

# 深度重置(清空数据重新初始化)
ak restart --init

# 重启 Docker 集群
ak restart -m docker -t cluster
```

---

### 查看服务日志

```bash
Usage:  ak logs [OPTIONS]

Options:
  -m,  --mode <MODE>    运行目标: bin 或 docker
  -t,  --topo <TOPO>    部署模式: single 或 cluster (cluster 仅 docker 模式下生效)
  -f,  --follow         持续跟踪日志
  -n,  --lines <LINES>  显示最近 N 行
  -h,  --help           查看帮助
```

#### 示例

```bash
# 查看日志
ak logs

# 实时跟踪
ak logs -f

# 查看最近 50 行并持续跟踪
ak logs -n 50 -f

# 查看 Docker 集群日志
ak logs -m docker -t cluster -f
```

---

### 查看服务运行状态

```bash
Usage:  ak ps [OPTIONS]

Options:
  -m,  --mode <MODE>      运行目标: bin 或 docker
  -t,  --topo <TOPO>      部署模式: single 或 cluster (cluster 仅 docker 模式下生效)
  -o,  --output <OUTPUT>  输出格式: `table`、`json`、`yaml`
  -h,  --help             查看帮助
```

#### 示例

```bash
# 查看服务状态
ak ps

# JSON 格式输出
ak ps -o json

# 集群拓扑
ak ps -t cluster -o yaml
```

---

### 管理工具配置

```bash
Usage:  ak config <COMMAND>

Commands:
  get   查看当前生效的配置
  set   设置配置项(e.g. ak config set project.root=/path)
  sync  将配置文件同步到当前 schema 版本
  path  显示当前使用的配置文件路径

Options:
  -h,  --help  查看帮助
```

```bash
Usage:  ak config get [OPTIONS]

Options:
  -o,  --output <OUTPUT>  输出格式:  yaml, json, table,  默认为 yaml
```

```bash
ak config set <KEY>=<VALUE>
```

#### 示例

```bash
# 查看当前配置
ak config get
ak config get -o json

# 设置 AiKv 项目根目录
ak config set project.root=/home/user/aikv

# 设置默认运行模式为 docker
ak config set defaults.mode=docker

# 设置默认部署拓扑为集群
ak config set defaults.topo=cluster
ak config set topo=cluster

# 设置 Docker 镜像
ak config set image=aikv:v2.0

# 配置文件升级后同步 schema
ak config sync

# 查看配置文件路径
ak config path
```

---

### 清理环境

```bash
Usage:  ak clean [OPTIONS]

Options:
  -m,  --mode <MODE>  运行目标: bin 或 docker
  -t,  --topo <TOPO> 部署模式: single 或 cluster (cluster 仅 docker 模式下生效)
  -a,  --all          清理所有拓扑的状态(相当于"恢复出厂", 仅保留配置)
  -f,  --force        强制清理,  跳过运行状态检查
  -h,  --help         查看帮助
```

#### 示例

```bash
# 清理当前拓扑的状态
ak clean

# 强制清理所有状态
ak clean -a -f
```

---

### 管理 OTel 可观测性栈

管理 OTel 观测栈(Prometheus、Grafana、Jaeger、Loki、Tempo、Pyroscope 等).

```bash
Usage:  ak otels [OPTIONS] <COMMAND>

Commands:
  up       启动 OTel 观测栈
  down     停止 OTel 观测栈
  restart  重启 OTel 观测栈
  logs     查看 OTel 日志
  status   查看 OTel 状态

Options:
  -f, --follow          持续跟踪日志
  -n, --lines <LINES>   显示最近 N 行日志
  -v, --remove-volumes  删除数据卷(Down 时使用)
  -h, --help            查看帮助
```

#### 示例

```bash
# 启动 OTel 观测栈
ak otels up

# 查看 OTel 状态
ak otels status

# 查看 OTel 日志
ak otels logs -f

# 停止 OTel 栈（保留数据卷）
ak otels down

# 停止 OTel 栈并删除数据卷
ak otels down -v
```

> **注意**: OTel 观测栈与 AiKv 服务共享 `aikv` 网络，支持以下场景：
> - 先启动 OTel, 再启动 AiKv
> - 先启动 AiKv, 再启动 OTel
> - 网络不存在时会自动创建

---

## 运行测试

在仓库根目录(aikv-tool 下)执行：
CLI 测试主要验证子命令解析、帮助信息、非法参数与互斥选项等，无需启动 Docker 或 AiKv 服务。

```bash
# 运行全部测试（单元测试 + CLI 集成测试）
cargo test

# 仅运行 CLI 集成测试（不依赖 Docker/真实进程）
cargo test --test cli

# 仅运行单元测试
cargo test --lib
```

## XDG 规范

ak 严格遵循 [XDG Base Directory](https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html) 规范:

| 用途 | 路径 | 说明 |
|------|------|------|
| 配置文件 | `~/.config/ak/ak.toml` | 全局配置 |
| 运行状态 | `~/.local/state/ak/run/` | PID 文件,  动态生成的 Compose 文件 |
| 缓存/日志 | `~/.cache/ak/logs/` | 工具日志和服务日志 |

## License

MIT
