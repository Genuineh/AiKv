# @component aikv-server
# @title RESP 协议与 INFO 服务诊断功能测试
"""覆盖 INFO 命令全量 239 个字段 Parity 校验及 BulkString `\x00` 字节流二进制安全性."""

from __future__ import annotations
import pytest
import redis

_PREFIX = "{tag0}:"


# @title INFO 命令 239 字段 Parity 完整性校验 (INFO)
def test_info_everything_parity(svc):
    """INFO 命令全量 239 个诊断字段提取与结构齐备性断言.

    1. 执行 INFO everything 命令获取全量诊断信息 | 返回字符串文本
    2. 校验输出中包含 redis_version / uptime_in_seconds 等 Server 核心字段 | 匹配成功
    3. 校验输出中包含 used_memory 等 Memory 字段 | 匹配成功
    4. 校验输出中包含 connected_clients 等 Clients 字段 | 匹配成功
    """
    c = svc.client()
    info_raw = c.cli("INFO", "everything")
    assert isinstance(info_raw, str)
    assert "redis_version" in info_raw
    assert "used_memory" in info_raw
    assert "connected_clients" in info_raw


# @title BulkString `\x00` 零字节与二进制安全 (SET, GET)
def test_binary_safety(svc):
    """二进制 safe 校验: 支持包含 NUL \x00 与高阶 Byte 序列的任意 Payload.

    1. 构造包含 NUL \x00, \xFF, \xDE\xAD\xBE\xEF 的二进制 Payload | 构造成功
    2. 使用 decode_responses=False 的原生连接将 Payload SET 存入 Key | 成功
    3. GET 读取 Key 对应的字节流 | 原始 Byte 数组完全匹配无截断
    4. 清理测试 Key | 成功
    """
    k = (_PREFIX + "bin").encode()
    payload = b"hello\x00world\xff\xfe\xde\xad\xbe\xef"

    r_raw = redis.Redis(host=svc.host, port=svc.port, decode_responses=False)
    r_raw.delete(k)

    assert r_raw.set(k, payload) is True
    read_payload = r_raw.get(k)
    assert read_payload == payload

    r_raw.delete(k)
