# @component aikv-server
# @title 并发
"""多客户端并行读写不挂死、不错乱到不可用."""

from __future__ import annotations

import threading

_PREFIX = "e2e:func:conc:"
_N = 50


# @title 双客户端并行
def test_two_clients_parallel_set_get(svc):
    """双客户端并行 SET/GET 不挂死、不错乱.

    1. 两线程各自将本批 50 个 key SET 成功 | 均成功
    2. 各线程 GET 本批 key | 值与各自写入一致
    3. 两线程在超时内结束 | 无异常
    4. 客户端再发 PING | 成功 (True)
    """
    errors: list[BaseException] = []

    def worker(tag: str) -> None:
        try:
            c = svc.client()
            for i in range(_N):
                key = f"{_PREFIX}{tag}:{i}"
                assert c.set(key, f"{tag}-{i}") is True
            for i in range(_N):
                key = f"{_PREFIX}{tag}:{i}"
                assert c.get(key) == f"{tag}-{i}"
        except BaseException as exc:  # noqa: BLE001 — 汇总线程异常
            errors.append(exc)

    t1 = threading.Thread(target=worker, args=("a",))
    t2 = threading.Thread(target=worker, args=("b",))
    t1.start()
    t2.start()
    t1.join(timeout=60)
    t2.join(timeout=60)
    assert not t1.is_alive() and not t2.is_alive(), "并发线程超时未结束"
    assert not errors, f"并发出错: {errors!r}"

    c = svc.client()
    assert c.ping() is True
    for tag in ("a", "b"):
        for i in range(_N):
            c.delete(f"{_PREFIX}{tag}:{i}")
