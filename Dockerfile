# ============================================================
# AiKv Dockerfile - 生产级多阶段构建
# ============================================================

# ------------------------------------------------------------
# Stage 1: Builder - 编译阶段
# ------------------------------------------------------------
FROM rust:1.92-bookworm AS builder

# 启用 BuildKit 缓存加速
ENV CARGO_HOME=/usr/local/cargo
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

# 安装编译依赖
RUN apt-get update && apt-get install -y \
    cmake \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 1. 预编译依赖 (利用 Docker 缓存)
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    mkdir -p benches && echo "fn main() {}" > benches/aikv_benchmark.rs && \
    echo "fn main() {}" > benches/comprehensive_benchmark.rs

# 接收功能开关参数 (如 FEATURES=cluster)
ARG FEATURES=""
RUN cargo build --release ${FEATURES:+--features $FEATURES}

# 2. 编译正式代码
COPY src ./src
# 确保源码更新后触发重新编译
RUN touch src/main.rs
RUN cargo build --release ${FEATURES:+--features $FEATURES} --bin aikv

# 3. 移除调试符号，极大幅度减小体积
RUN strip target/release/aikv

# ------------------------------------------------------------
# Stage 2: Runtime - 极简运行环境
# ------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# 安装运行时必要组件
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    redis-tools \
    && rm -rf /var/lib/apt/lists/*

# 创建非 root 安全用户
RUN groupadd --gid 1000 aikv && \
    useradd --uid 1000 --gid aikv --shell /bin/bash --create-home aikv

WORKDIR /app

# 从编译阶段拷贝产物
COPY --from=builder /app/target/release/aikv ./aikv
# 拷贝你刚移动到根目录的配置文件
COPY aikv.toml ./aikv.toml

# 权限设置
RUN chown -R aikv:aikv /app
USER aikv

# 端口与健康检查
EXPOSE 6379
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD redis-cli PING | grep -q "PONG" || exit 1

# 启动指令：默认读取同级目录下的 aikv.toml
ENTRYPOINT ["/app/aikv"]
CMD ["--config", "/app/aikv.toml"]

# 镜像元数据
LABEL org.opencontainers.image.title="AiKv" \
      org.opencontainers.image.source="https://github.com/Genuineh/AiKv"
