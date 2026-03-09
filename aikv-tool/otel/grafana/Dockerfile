# Grafana 镜像：预装 Drilldown 等插件
# 构建: 在仓库根目录执行 docker build -f otel/grafana/Dockerfile -t aikv-grafana:12.3.0 .
# 或由 docker compose build grafana 自动构建
FROM grafana/grafana:12.3.0

USER root

# 构建时安装插件，运行时无需网络、无需等待（首次构建会拉取插件，约 1～2 分钟）
RUN grafana-cli plugins install grafana-metricsdrilldown-app && \
    grafana-cli plugins install grafana-exploretraces-app && \
    grafana-cli plugins install grafana-lokiexplore-app && \
    grafana-cli plugins install grafana-pyroscope-app && \
    grafana-cli plugins install pyroscope-panel

USER grafana
