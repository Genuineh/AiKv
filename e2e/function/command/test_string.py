# @component aikv-server
# @title String、Bitmaps 与 Key 空间管理功能测试
"""覆盖 String 全量读写 (SET NX/EX, MSET/MGET)、Bitmaps 位图、数值自增递减与 Key 空间过期管理 (EXPIRE/TTL/SCAN 等)."""

from __future__ import annotations

_PREFIX = "{tag0}:"


# @title String 基础读写与高级选项 (SET NX/EX/GET, MSET, MGET, STRLEN, APPEND)
def test_string_crud_and_opts(svc):
    """SET NX/EX 扩展选项与 MSET/MGET 批量读写.

    1. 清理测试 Key k1, k2 | 成功
    2. 设置 k1 为 "v1" 并指定 nx=True | 返回 True (成功写入)
    3. 再次设置 k1 为 "v2" 并指定 nx=True | 返回 None (Key 已存在拒绝写入)
    4. 读取 k1 | 返回 "v1"
    5. 使用 MSET 批量写入 k1="nv1", k2="nv2" | 返回 True
    6. 使用 MGET 批量读取 k1, k2 | 返回 ["nv1", "nv2"]
    7. 查询 k1 的字符串长度 STRLEN | 返回 3
    8. 对 k1 APPEND 追加 "_app" | 返回新长度 7
    9. 读取 k1 | 返回 "nv1_app"
    10. 清理测试 Key k1, k2 | 成功
    """
    c = svc.client()
    k1 = _PREFIX + "k1"
    k2 = _PREFIX + "k2"
    c.delete(k1, k2)

    # SET NX
    assert c._r.set(k1, "v1", nx=True) is True
    assert c._r.set(k1, "v2", nx=True) is None
    assert c.get(k1) == "v1"

    # MSET & MGET (同一 slot)
    assert c._r.mset({k1: "nv1", k2: "nv2"}) is True
    assert c._r.mget(k1, k2) == ["nv1", "nv2"]

    # STRLEN & APPEND
    assert c._r.strlen(k1) == 3
    assert c._r.append(k1, "_app") == 7
    assert c.get(k1) == "nv1_app"

    c.delete(k1, k2)


# @title 数值运算与 Bitmaps 位图 (INCR, INCRBY, DECR, INCRBYFLOAT, SETBIT, GETBIT)
def test_numeric_and_bitmaps(svc):
    """数值递增递减与 Bitmaps 按位读写.

    1. 清理测试 Key knum, kbit | 成功
    2. 对 knum 执行 INCR 自增 1 | 返回 1
    3. 对 knum 执行 INCRBY 增加 10 | 返回 11
    4. 对 knum 执行 DECR 减少 1 | 返回 10
    5. 对 knum 执行 INCRBYFLOAT 增加 2.5 | 返回 12.5
    6. 设置 kbit 第 7 位偏移为 1 | 返回原旧值 0
    7. 读取 kbit 第 7 位偏移 | 返回 1
    8. 读取 kbit 第 0 位偏移 | 返回 0
    9. 清理测试 Key knum, kbit | 成功
    """
    c = svc.client()
    knum = _PREFIX + "num"
    kbit = _PREFIX + "bit"
    c.delete(knum, kbit)

    assert c._r.incr(knum) == 1
    assert c._r.incrby(knum, 10) == 11
    assert c._r.decr(knum) == 10
    assert float(c._r.incrbyfloat(knum, 2.5)) == 12.5

    # Bitmaps
    assert c._r.setbit(kbit, 7, 1) == 0
    assert c._r.getbit(kbit, 7) == 1
    assert c._r.getbit(kbit, 0) == 0

    c.delete(knum, kbit)


# @title Key 空间过期与 SCAN 游标迭代 (EXPIRE, TTL, PERSIST, SCAN)
def test_key_expiration_and_scan(svc):
    """Key 过期生命周期与 SCAN 游标遍历.

    1. 清理测试 Key k | 成功
    2. 写入 Key k 内容 "temp" | 成功
    3. 设置 k 的过期时间 EXPIRE 为 10 秒 | 返回 True
    4. 查询 k 的剩余生存时间 TTL | 返回 0 到 10 之间
    5. 移除 k 的过期属性 PERSIST | 返回 True
    6. 查询 k 的剩余生存时间 TTL | 返回 -1 (永不过期)
    7. 使用 SCAN 游标遍历匹配前缀的 Key | 集合中包含 Key k
    8. 清理测试 Key k | 成功
    """
    c = svc.client()
    k = _PREFIX + "expire"
    c.delete(k)

    c.set(k, "temp")
    assert c._r.expire(k, 10) is True
    ttl = c._r.ttl(k)
    assert 0 < ttl <= 10

    assert c._r.persist(k) is True
    assert c._r.ttl(k) == -1

    # SCAN via cli
    res_scan = c.cli("SCAN", "0", "MATCH", _PREFIX + "*", "COUNT", "100")
    assert k in res_scan

    c.delete(k)
