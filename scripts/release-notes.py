#!/usr/bin/env python3
"""从 CHANGELOG.md 提取指定版本的发布说明。

用法：python3 scripts/release-notes.py 0.1.0 > RELEASE_NOTES.md

找不到对应小节时以非零退出码失败——发布流程里宁可中断，也不要发一份空说明。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

CHANGELOG = Path(__file__).resolve().parent.parent / "CHANGELOG.md"


def extract(version: str) -> str:
    text = CHANGELOG.read_text(encoding="utf-8")
    v = re.escape(version.lstrip("v"))
    # 匹配 `## [x.y.z] - 日期` 到下一个同级标题之前
    m = re.search(rf"^## \[{v}\][^\n]*\n(.*?)(?=^## \[|\Z)", text, re.M | re.S)
    if not m:
        raise SystemExit(f"CHANGELOG.md 中没有 [{version}] 小节，请先补充")
    body = m.group(1).strip("\n")
    if not body.strip():
        raise SystemExit(f"CHANGELOG.md 中 [{version}] 小节为空")
    return body.rstrip() + "\n"


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("用法：release-notes.py <版本号>，如 release-notes.py 0.1.0")
    sys.stdout.write(extract(sys.argv[1]))


if __name__ == "__main__":
    main()