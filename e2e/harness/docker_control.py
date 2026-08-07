"""Docker 容器节点控制辅助库 (专用于黑盒 Docker 集群环境下的故障注入测试)."""

from __future__ import annotations

import subprocess

PORT_TO_CONTAINER: dict[int, str] = {
    6379: "aikv-1",
    6380: "aikv-2",
    6381: "aikv-3",
    7379: "aikv-4",
    7380: "aikv-5",
    7381: "aikv-6",
}


def is_docker_available() -> bool:
    """检查当前环境是否可与 docker daemon 交互."""
    try:
        proc = subprocess.run(["docker", "ps"], capture_output=True, check=False)
        return proc.returncode == 0
    except Exception:
        return False


def get_container_name_by_port(port: int) -> str | None:
    """按映射端口查找 aikv Docker 容器名."""
    if port in PORT_TO_CONTAINER:
        return PORT_TO_CONTAINER[port]
    try:
        proc = subprocess.run(
            ["docker", "ps", "-a", "--format", "{{.Names}} {{.Ports}}"],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            return None
        for line in proc.stdout.splitlines():
            if f":{port}->" in line or f"0.0.0.0:{port}->" in line:
                return line.split()[0]
    except Exception:
        pass
    return None


def kill_container(name_or_port: str | int) -> bool:
    """强杀容器 (SIGKILL)."""
    name = (
        get_container_name_by_port(name_or_port)
        if isinstance(name_or_port, int)
        else name_or_port
    )
    if not name:
        return False
    proc = subprocess.run(["docker", "kill", name], capture_output=True, check=False)
    return proc.returncode == 0


def stop_container(name_or_port: str | int) -> bool:
    """停止容器."""
    name = (
        get_container_name_by_port(name_or_port)
        if isinstance(name_or_port, int)
        else name_or_port
    )
    if not name:
        return False
    proc = subprocess.run(["docker", "stop", name], capture_output=True, check=False)
    return proc.returncode == 0


def start_container(name_or_port: str | int) -> bool:
    """启动已有容器."""
    name = (
        get_container_name_by_port(name_or_port)
        if isinstance(name_or_port, int)
        else name_or_port
    )
    if not name:
        return False
    proc = subprocess.run(["docker", "start", name], capture_output=True, check=False)
    return proc.returncode == 0


def container_networks(name: str) -> list[str]:
    """列出容器连接的全部网络名 (断网前快照, 供断网后重连)."""
    proc = subprocess.run(
        [
            "docker",
            "inspect",
            "-f",
            "{{range $k,$v := .NetworkSettings.Networks}}{{$k}} {{end}}",
            name,
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        return []
    return proc.stdout.split()


def disconnect_network(name_or_port: str | int) -> list[str]:
    """断开容器全部网络 (网络分区注入).

    返回断开前的网络名列表, 供 `connect_network` 恢复. 节点可能挂多个网络
    (compose 自定义网 + 外部网), 逐个断开才能实现真正的孤立.
    """
    name = (
        get_container_name_by_port(name_or_port)
        if isinstance(name_or_port, int)
        else name_or_port
    )
    if not name:
        return []
    nets = container_networks(name)
    for net in nets:
        subprocess.run(
            ["docker", "network", "disconnect", net, name],
            capture_output=True,
            check=False,
        )
    return nets


def connect_network(name_or_port: str | int, networks: list[str] | None = None) -> bool:
    """重连容器网络; `networks` 为空时用 `container_networks` 重新发现.

    返回是否全部网络重连成功 (任一失败 → False, 便于测试在恢复阶段定位).
    """
    name = (
        get_container_name_by_port(name_or_port)
        if isinstance(name_or_port, int)
        else name_or_port
    )
    if not name:
        return False
    nets = networks if networks is not None else container_networks(name)
    ok = True
    for net in nets:
        proc = subprocess.run(
            ["docker", "network", "connect", net, name],
            capture_output=True,
            check=False,
        )
        if proc.returncode != 0:
            ok = False
    return ok


def resp_encode(*args: str) -> str:
    """RESP array 编码 (命令参数)."""
    parts = [f"*{len(args)}\r\n"]
    for a in args:
        raw = a.encode("utf-8")
        parts.append(f"${len(raw)}\r\n")
        parts.append(a)
        parts.append("\r\n")
    return "".join(parts)


def parse_resp(data: str):
    """小 RESP 解码器 (供 CLUSTER SLOTS 等嵌套结构使用).

    `-ERR ...` 错误行返回该文本 (调用方以 `startswith("-")` 判别).
    """
    pos = 0
    text = data

    def _parse():
        nonlocal pos
        while pos < len(text) and text[pos] in "\r\n":
            pos += 1
        if pos >= len(text):
            return None
        t = text[pos]
        line_end = text.index("\r\n", pos)
        line = text[pos + 1 : line_end]
        pos = line_end + 2
        if t == "+":
            return line
        if t == "-":
            return line
        if t == ":":
            return int(line)
        if t == "$":
            n = int(line)
            if n == -1:
                return None
            val = text[pos : pos + n]
            pos += n + 2
            return val
        if t == "*":
            n = int(line)
            if n == -1:
                return None
            return [_parse() for _ in range(n)]
        raise ValueError(f"未知 RESP 类型 {t!r}")

    return _parse()


def exec_resp(name: str, *args: str, port: int = 6379) -> str:
    """docker exec 内执行 RESP 命令, 返回原始 RESP 文本."""
    return exec_resp_raw(name, resp_encode(*args), port=port)


def exec_resp_raw(name: str, payload: str, port: int = 6379) -> str:
    """docker exec 内执行预编码的 RESP 载荷 (支持管道化多命令).

    镜像内无 redis-cli 但有 netcat-openbsd (entrypoint healthcheck 同款),
    故用 `printf '<RESP>' | nc -w 5 127.0.0.1 <port>` 走回环完成请求.
    `-w 5` 提供大响应 (CLUSTER SLOTS / CLUSTER INFO / READONLY+GET 管道) 的完整
    读取窗口, 分区测试期间宿主机负载波动时避免短窗口截断大响应导致 flake;
    断网后 host 端口映射失效, 容器内操作一律走此入口.
    """
    escaped = payload.replace("'", "'\\''")
    script = f"printf '%s' '{escaped}' | nc -w 5 127.0.0.1 {port}"
    # stdout 必须二进制读取 (text=True 的 universal newlines 会把 `\r\n` 归一化掉 `\r`,
    # 破坏 RESP 解析); 本命令不读 stdin, 故无 `-i`.
    proc = subprocess.run(
        ["docker", "exec", name, "sh", "-c", script],
        capture_output=True,
        check=False,
    )
    return proc.stdout.decode("utf-8", errors="replace")
