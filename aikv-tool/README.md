# ak - AiKv 现代分布式 KV 存储管理工具

旨在提供类 Docker/Kubectl 的极致操作体验, 支持从本地开发, 集群部署到生产运维的全生命周期管理. 严格遵循 XDG 规范. 

## 特性

- 类 Docker/Kubectl 的命令行体验
- 全局 `-m/--mode` 选项, 类似 kubectl 的 `-n/--namespace`
- 支持本地二进制和 Docker 容器两种运行模式
- 支持单节点和集群部署
- 配置驱动, 默认值来自配置文件
- 严格遵循 XDG Base Directory 规范

## 安装

```bash
# 从源码编译安装
cd aikv-tool
cargo build --release

# 将二进制文件添加到 PATH
cp target/release/ak ~/.local/bin/
```

## 快速开始

```bash
# 构建 AiKv 二进制
ak build

# 启动服务(使用配置文件默认模式)
ak up

# 查看服务状态
ak get services

# 查看日志
ak logs -f

# 停止服务
ak down
```

## 全局选项

| 选项 | 说明 |
|------|------|
| `-m, --mode <MODE>` | 运行模式: `bin` 或 `docker`(默认读取配置文件) |
| `-v, --version` | 显示版本信息 |
| `-h, --help` | 显示帮助信息 |

### 模式选项说明

`-m/--mode` 是全局选项, 类似 kubectl 的 `-n/--namespace`, 用于指定运行模式: 

```bash
# 使用配置文件默认模式
ak up

# 显式指定 docker 模式
ak up -m docker

# 显式指定 bin 模式
ak logs -m bin -f
```

## XDG 目录规范

ak 严格遵循 [XDG Base Directory](https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html) 规范: 

| 用途 | 路径 | 说明 |
|------|------|------|
| 配置文件 | `~/.config/ak/ak.toml` | 全局配置 |
| 运行状态 | `~/.local/state/ak/run/` | PID 文件, 动态生成的 Compose 文件 |
| 缓存/日志 | `~/.cache/ak/logs/` | 工具日志和服务日志 |

## 配置文件

ak 按以下优先级加载配置: 

1. 当前目录向上查找的 `ak.toml`(项目级配置)
2. `~/.config/ak/ak.toml`(全局配置)

### 配置文件示例

```toml
# ak.toml([package]=配置元数据, [project]=AiKv 项目路径, [server]=端口与 aikv 对齐)

[package]
version = 1

[project]
root = "/path/to/aikv"

[server]
port = 6379

[build]
release = false
cluster = false

[docker]
image = "aikv:latest"

[run]
mode = "bin"
topo = "single"
```

---

## 命令参考

### `ak build` - 编译源代码或构建 Docker 镜像

**别名:** `ak b`

```bash
ak build [OPTIONS]
```

构建目标由全局 `-m/--mode` 或配置 `run.mode` 决定, 与 up/down 等命令一致. 

#### 选项

| 选项 | 说明 |
|------|------|
| `-r, --release` | Release 模式构建 |
| `-c, --cluster` | 启用集群特性 |
| `-s, --single` | 单节点模式 |
| `-t, --tag <TAG>` | 镜像标签(仅 -m docker) | 默认 latest |
| `-i, --image <IMAGE>` | 镜像完整名称(仅 -m docker) | aikv:\<tag\> |
| `-f, --force` | 强制重建: bin 时先 clean 再 build; docker 时覆盖已存在镜像 | - |

#### 示例

```bash
# 编译二进制(默认或 -m bin)
ak build
ak build -m bin -r

# 构建 Docker 镜像(-m docker 或配置 run.mode=docker)
ak build -m docker
ak build -m docker -t dev -f
ak build -m docker -i myreg/aikv:v1.0.0

# 集群构建
ak build -m docker -c
```

---

### `ak up` - 启动 AiKv 服务

```bash
ak up [OPTIONS]
```

#### 选项

| 选项 | 说明 |
|------|------|
| `-m, --mode <MODE>` | 运行模式: `bin` 或 `docker` |
| `-f, --foreground` | 前台模式运行 |
| `-c, --cluster` | 集群模式(Docker) |
| `-s, --single` | 单节点模式(Docker) |
| `-n, --nodes <N>` | 节点总数(纯节点模式) |
| `--shards <N>` | 集群分片数 |
| `--replicas <N>` | 每分片副本数 |
| `-i, --image <IMAGE>` | Docker 镜像 |

#### 示例

```bash
# 使用配置文件默认模式启动
ak up

# 启动本地二进制(后台)
ak up -m bin

# 前台模式启动
ak up -m bin -f

# 启动单节点 Docker 容器
ak up -m docker -s

# 启动 3 分片 1 副本的集群(6 节点)
ak up -m docker -c --shards 3 --replicas 1

# 启动 5 节点的纯节点集群
ak up -m docker -c -n 5

# 使用指定镜像
ak up -m docker -i myregistry/aikv:v2.0
```

---

### `ak down` - 停止并移除服务

```bash
ak down [OPTIONS]
```

#### 选项

| 选项 | 说明 |
|------|------|
| `-m, --mode <MODE>` | 运行模式 |
| `-v, --remove-volumes` | 同时删除数据卷 |
| `-c, --cluster` | 集群模式(Docker) |
| `-s, --single` | 单节点模式(Docker) |

#### 示例

```bash
# 停止当前服务
ak down

# 停止本地二进制
ak down -m bin

# 停止 Docker 集群并删除数据
ak down -m docker -c -v
```

---

### `ak restart` - 重启服务

```bash
ak restart [OPTIONS]
```

#### 选项

| 选项 | 说明 |
|------|------|
| `-m, --mode <MODE>` | 运行模式 |
| `-i, --init` | 深度重置(清理数据后重新启动) |
| `-c, --cluster` | 集群模式(Docker) |
| `-s, --single` | 单节点模式(Docker) |

#### 示例

```bash
# 原地重启
ak restart

# 深度重置(清空数据重新初始化)
ak restart -i

# 重启 Docker 集群
ak restart -m docker -c
```

---

### `ak logs` - 查看服务日志

**别名:** `ak l`

```bash
ak logs [OPTIONS]
```

#### 选项

| 选项 | 说明 | 默认值 |
|------|------|--------|
| `-m, --mode <MODE>` | 运行模式 | - |
| `-f, --follow` | 持续跟踪日志 | - |
| `-n, --lines <N>` | 显示最近 N 行 | `100` |
| `-c, --cluster` | 集群模式(Docker) | - |
| `-s, --single` | 单节点模式(Docker) | - |

#### 示例

```bash
# 查看日志
ak logs

# 实时跟踪
ak logs -f

# 查看最近 50 行并持续跟踪
ak logs -n 50 -f

# 查看 Docker 集群日志
ak logs -m docker -c -f
```

---

### `ak get` - 获取资源状态

**别名:** `ak g`

```bash
ak get <RESOURCE> [OPTIONS]
```

#### 资源类型

| 资源 | 说明 |
|------|------|
| `services` | 服务运行状态 |
| `config` | 当前生效的配置 |

#### 选项

| 选项 | 说明 | 默认值 |
|------|------|--------|
| `-o, --output <FORMAT>` | 输出格式: `json`, `yaml`, `table` | `table` |
| `-c, --cluster` | 集群范围 | - |
| `-s, --single` | 单节点范围 | - |

#### 示例

```bash
# 查看服务状态
ak get services

# JSON 格式输出
ak get services -o json

# 查看配置
ak get config

# YAML 格式输出配置
ak get config -o yaml
```

---

### `ak set` - 设置配置项

```bash
ak set config <KEY>=<VALUE>
```

#### 支持的配置项

| 配置项 | 说明 | 示例 |
|--------|------|------|
| `project.root` / `root` / `project_root` | AiKv 项目根目录 | `/home/user/aikv` |
| `server.port` / `port` | 服务端口(与 aikv 对齐) | `6379` |
| `build.release` | 默认 release 模式 | `true` |
| `docker.image` / `image` | Docker 默认镜像(build/up -m docker 未指定 -i/--image 时使用) | `aikv:v2.0` |
| `run.mode` | 默认运行目标 | `bin` / `docker` |
| `run.topo` | 默认部署拓扑 | `cluster` / `single` |

#### 示例

```bash
# 设置 AiKv 项目根目录
ak set config project.root=/home/user/aikv

# 设置默认运行模式为 docker
ak set config run.mode=docker

# 设置默认部署拓扑为集群
ak set config run.topo=cluster

# 设置 Docker 镜像
ak set config image=aikv:v2.0
```

---

### `ak clean` - 清理环境

```bash
ak clean [OPTIONS]
```

#### 选项

| 选项 | 说明 |
|------|------|
| `-a, --all` | 清理所有模式的状态 |
| `-f, --force` | 强制清理, 忽略运行状态检查 |
| `--logs` | 同时清理日志目录 |

#### 示例

```bash
# 清理当前模式的状态
ak clean

# 强制清理所有
ak clean -a -f

# 清理所有状态和日志
ak clean -a -f --logs
```

---

## 典型工作流

### 本地开发

```bash
# 编译
ak build

# 启动(前台调试)
ak up -m bin -f

# 停止
ak down
```

### Docker 单节点

```bash
# 构建镜像
ak build -m docker

# 启动
ak up -m docker -s

# 查看状态和日志
ak get services
ak logs -f

# 停止
ak down -v
```

### Docker 集群

```bash
# 构建集群镜像
ak build -m docker -c

# 启动 3 分片 1 副本集群
ak up -m docker -c --shards 3 --replicas 1

# 查看状态
ak get services -c

# 深度重置
ak restart -i

# 停止
ak down -c -v
```

### 设置默认模式(避免每次输入 -m)

```bash
# 设置默认为 docker 模式
ak set config target=docker

# 之后直接使用, 无需 -m
ak up -c --shards 3
ak logs -f
ak down -v
```

## License

MIT
