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
