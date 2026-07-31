# @component aikv-server
# @title 强杀崩溃与 WAL 重放功能测试
"""通过 SIGKILL 信号强杀节点进程模拟突发断电/崩溃, 重启后校验崩溃前已落盘 WAL 日志重放准确性."""

from __future__ import annotations
import os
import signal
import pytest
from harness.binary import ensure_release_binary
from harness.node import start_node

_PREFIX = "{e2e:func:02:crash}:"


# @title 强杀崩溃恢复后数据一致性
def test_crash_recovery_wal_replay(svc):
    """验证 SIGKILL 强杀后重启 WAL 日志重放无损坏 (非伪通过).

    1. 构建/确认 aikv 二进制可执行文件 | 成功
    2. 启动临时单机测试节点 | 节点就绪
    3. 写入崩溃前测试 Key | 写入成功
    4. 对节点进程发送 SIGKILL (kill -9) 强杀信号 | 进程异常退出
    5. 复用原数据目录启动新节点，触发 WAL 重发恢复 | 节点就绪
    6. 读取之前写入的 Key | WAL 日志重放完毕且内容一致
    7. 清理临时节点与数据目录 | 清理成功
    """
    try:
        binary = ensure_release_binary()
    except Exception:
        pytest.skip("无法构建/获取 aikv 可执行二进制文件，跳过 SIGKILL 崩溃恢复测试")

    temp_node = start_node(binary=binary, engine="aidb")
    try:
        c = temp_node.client()
        k = _PREFIX + "crash_k"
        c.set(k, "persisted_before_crash")

        port = temp_node.port
        if temp_node.proc is not None:
            os.kill(temp_node.proc.pid, signal.SIGKILL)
            temp_node.proc.wait()
            temp_node.proc = None

        restarted_node = start_node(binary=binary, engine="aidb", port=port)
        try:
            c2 = restarted_node.client()
            assert c2.get(k) == "persisted_before_crash"
            c2.delete(k)
        finally:
            restarted_node.stop(keep_data=False)
    finally:
        if temp_node.proc is not None:
            temp_node.stop(keep_data=False)
