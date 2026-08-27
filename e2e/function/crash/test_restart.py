# @component aikv-server
# @title 节点优雅重启与持久化完整性功能测试
"""通过控管句柄优雅停止 (SIGTERM) 节点进程并保留数据目录重新拉起, 校验已持久化数据完好性."""

from __future__ import annotations

from harness.binary import ensure_release_binary
from harness.node import start_node

_PREFIX = "{e2e:func:02:restart}:"


# @title 优雅重启与数据完整性校验
def test_graceful_restart_data_integrity():
    """验证服务优雅停止 (SIGTERM) 并重启后数据零丢失.

    本用例自建临时单机节点, 不依赖外部被测服务.

    1. 构建/确认 aikv 二进制可执行文件 | 成功
    2. 启动临时单机测试节点 | 节点就绪
    3. 写入测试数据 Key | 写入成功
    4. 获取节点端口，执行 stop(keep_data=True) 优雅停止节点 | 进程停止
    5. 复用相同数据目录在原端口拉起新节点 | 节点就绪
    6. 读取之前写入的 Key | 内容与重启前完全一致
    7. 优雅清理并关闭临时节点 | 清理成功
    """
    binary = ensure_release_binary()
    temp_node = start_node(binary=binary, engine="aidb")
    try:
        c = temp_node.client()
        k = _PREFIX + "k1"
        c.set(k, "persisted_val")

        port = temp_node.port
        temp_node.stop(keep_data=True)

        restarted_node = start_node(binary=binary, engine="aidb", port=port)
        try:
            c2 = restarted_node.client()
            assert c2.get(k) == "persisted_val"
            c2.delete(k)
        finally:
            restarted_node.stop(keep_data=False)
    finally:
        if temp_node.proc is not None:
            temp_node.stop(keep_data=False)
