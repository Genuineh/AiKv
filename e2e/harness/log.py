"""统一端到端日志 (stderr)."""

from __future__ import annotations

import logging
import os

_LOG: logging.Logger | None = None


def get_logger() -> logging.Logger:
    """返回名为 `aikv.e2e` 的 logger; 级别由 `WIKV_E2E_LOG` 控制."""
    global _LOG
    if _LOG is not None:
        return _LOG
    level = os.environ.get("WIKV_E2E_LOG", "INFO").upper()
    logger = logging.getLogger("aikv.e2e")
    if not logger.handlers:
        handler = logging.StreamHandler()
        handler.setFormatter(
            logging.Formatter("%(asctime)s %(levelname)s %(name)s: %(message)s")
        )
        logger.addHandler(handler)
    logger.setLevel(getattr(logging, level, logging.INFO))
    _LOG = logger
    return logger
