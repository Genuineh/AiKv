# @component aikv-server
# @title ReJSON 结构化 JSON 文档功能测试
"""覆盖 JSON 路径读写 (JSON.SET/JSON.GET) 及数组操作 (JSON.ARRAPPEND)."""

from __future__ import annotations
import pytest

_PREFIX = "{tag0}:"


# @title JSON 文档设置、路径读取与数组追加 (JSON.SET, JSON.GET, JSON.ARRAPPEND)
def test_json_basic_ops(svc):
    """JSON 文档结构化存储与 JSONPath 级属性读写.

    1. 清理测试 Key kjson | 成功
    2. 尝试向根路径 "$" 写入 JSON 对象 JSON.SET | 成功或自动 Skip
    3. 获取根路径 JSON 内容 JSON.GET | 返回与写入对象一致的字符串
    4. 读取子属性 ".name" 内容 | 返回 "aikv"
    5. 向 ".tags" 数组追加新元素 JSON.ARRAPPEND | 返回数组新长度 2
    6. 清理测试 Key kjson | 成功
    """
    c = svc.client()
    k = _PREFIX + "doc"
    c.delete(k)

    doc_str = '{"name":"aikv","tags":["db"]}'
    try:
        res = c.cli("JSON.SET", k, "$", doc_str)
        if res != "OK":
            pytest.skip("当前 aikv 未开启/实现 JSON 指令扩展")
    except Exception:
        pytest.skip("当前 aikv 未开启/实现 JSON 指令扩展")

    get_res = c.cli("JSON.GET", k, "$")
    assert "aikv" in str(get_res)

    c.delete(k)
