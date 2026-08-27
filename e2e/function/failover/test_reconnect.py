# @component aikv-server
# @title Smart Client 路由感知与自动重定向功能测试
"""覆盖 Smart Client (redis-py / cluster) 在收到 MOVED 重定向错误时自动感知并重新路由到正确节点的特性."""

from __future__ import annotations

import pytest
import redis

_PREFIX = "{e2e:func:04:smart}:"


# @title Smart Client 自动重定向与路由表更新
def test_client_smart_routing(svc):
    """Smart Client 在跨节点请求时自动处理 MOVED/ASK 重定向并完成读写.

    1. 使用 6379 非集群直连客户端找到第二分片 Key | 成功
    2. 直连 GET 明确收到 MOVED | 抛出 ResponseError
    3. 使用集群模式 Smart Client 写入带 Hash Tag 的 Key | 自动感知分片节点并写入成功
    4. 使用 Smart Client 读取 Key 内容 | 自动重定向并返回准确值 "smart_val"
    5. 清理测试 Key smart_k | 成功
    """
    c = svc.client()
    if not c.cluster:
        pytest.skip("被测服务非集群模式")

    direct = redis.Redis(host=svc.host, port=6379, decode_responses=True)
    k = None
    try:
        for i in range(2000):
            candidate = f"{{moved{i}}}:{_PREFIX}smart_k"
            slot = int(direct.execute_command("CLUSTER", "KEYSLOT", candidate))
            if 8192 <= slot <= 16383:
                k = candidate
                break
        else:
            pytest.fail("无法找到第二分片测试 key")

        with pytest.raises(redis.exceptions.ResponseError, match=r"^MOVED \d+ \S+"):
            direct.get(k)
    finally:
        direct.close()

    try:
        c.delete(k)
        assert c.set(k, "smart_val") is True
        assert c.get(k) == "smart_val"
    finally:
        c.delete(k)
