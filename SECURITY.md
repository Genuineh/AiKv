---
name: security
description: AiKv 安全漏洞报告与支持范围
---

# AiKv 安全政策

## 支持范围

安全修复仅针对最新的 `1.x` 版本提供。其他版本不在持续支持范围内, 报告问题前请先升级到最新的 `1.x` 版本并确认问题仍然存在.

## 报告漏洞

请使用 GitHub Private Vulnerability Reporting 提交安全漏洞:

https://github.com/wiqun/AiKv/security/advisories/new

请勿在公开 Issue 或讨论区发布尚未修复的漏洞细节. 报告中请尽量包含受影响的 AiKv 版本, 运行平台, 部署方式, 复现步骤, 影响范围以及可用的缓解措施.

## 安全边界

AiKv v1 不内建 `AUTH`, `ACL` 或 `TLS`. `RESP`, `MetaRaft` 和 `MultiRaft` 端口不得暴露到不可信网络. 需要跨越信任边界时, 必须在 AiKv 前使用认证/TLS proxy 或 service mesh, 并通过网络访问控制限制端口来源.

## 响应说明

除非经用户与维护者明确确认, 本项目不承诺固定响应 SLA, 修复时间或公开披露时间. 是否受理, 风险分级, 修复计划与披露安排将根据报告内容和维护者可用性单独确认.
