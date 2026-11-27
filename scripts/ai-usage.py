#!/usr/bin/env python3
"""统计本项目在 DSH 中消耗的模型与 Token（数据来自本地会话日志）。

用法：
    python3 scripts/ai-usage.py                  # 以当前仓库路径定位会话目录
    python3 scripts/ai-usage.py --dir <会话目录>  # 手动指定

说明：
- DSH 把每个会话存成 `<会话目录>/session.v4.jsonl.zstd`，按工作目录分目录，
  目录名是 cwd 的转义形式（如 `--home-<用户>-repos-RimeTools--`）。
- 每步请求的 `usage` 字段给出 inputTokens / outputTokens / cacheReadTokens / totalTokens。
  `cacheReadTokens` 是命中前缀缓存的部分，已经包含在 totalTokens 里。
- 主会话（delegationDepth = 0）与子代理会话（delegationDepth = 1）分开统计。
- 数字会随会话继续增长，需要更新时重跑本脚本即可。
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import json
import pathlib
import re
import subprocess
import sys

MODEL_RE = re.compile(r'"model"\s*:\s*"([^"]+)"')


def default_session_dir() -> pathlib.Path:
    """按当前仓库绝对路径推导 DSH 会话目录名。"""
    cwd = pathlib.Path.cwd().resolve()
    escaped = str(cwd).lstrip("/").replace("/", "-")
    return pathlib.Path.home() / ".dsh" / "sessions" / f"--{escaped}--"


def read_session(path: pathlib.Path) -> dict | None:
    try:
        raw = subprocess.run(
            ["zstd", "-dc", str(path)], capture_output=True, check=True
        ).stdout.decode("utf-8", "replace")
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None

    usage = dict(input=0, output=0, cache=0, total=0, steps=0)
    turns = 0
    depth = None
    tmin = tmax = None
    models: collections.Counter[str] = collections.Counter()
    # 按模型归集用量：一个会话中途可能切换模型
    per_model: dict[str, dict[str, int]] = collections.defaultdict(
        lambda: dict(input=0, output=0, cache=0, total=0, steps=0)
    )
    current_model: str | None = None

    for line in raw.splitlines():
        if not line.strip():
            continue
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue

        if depth is None and "delegationDepth" in rec:
            depth = rec.get("delegationDepth")

        t = rec.get("time")
        if isinstance(t, (int, float)) and t > 0:
            tmin = t if tmin is None else min(tmin, t)
            tmax = t if tmax is None else max(tmax, t)

        if rec.get("type") == "turn/start":
            turns += 1

        # 记录当前请求使用的模型
        if rec.get("type") in ("request/header", "request/context"):
            head = rec.get("data") or {}
            cfg = head.get("header") or head
            m = (cfg.get("config") or {}).get("model") or cfg.get("model")
            if isinstance(m, str):
                current_model = m
        elif rec.get("type") == "assistant/message":
            src = ((rec.get("data") or {}).get("message") or {}).get("source") or {}
            mm = src.get("model")
            if isinstance(mm, str):
                current_model = mm
            elif isinstance(mm, dict) and isinstance(mm.get("model"), str):
                current_model = mm["model"]

        for m in MODEL_RE.findall(line):
            models[m] += 1

        u = (rec.get("data") or {}).get("usage")
        if isinstance(u, dict) and "inputTokens" in u:
            usage["input"] += u.get("inputTokens", 0)
            usage["output"] += u.get("outputTokens", 0)
            usage["cache"] += u.get("cacheReadTokens", 0)
            usage["total"] += u.get("totalTokens", 0)
            usage["steps"] += 1
            if current_model:
                pm = per_model[current_model]
                pm["input"] += u.get("inputTokens", 0)
                pm["output"] += u.get("outputTokens", 0)
                pm["cache"] += u.get("cacheReadTokens", 0)
                pm["total"] += u.get("totalTokens", 0)
                pm["steps"] += 1

    return dict(
        usage=usage,
        turns=turns,
        depth=depth,
        tmin=tmin,
        tmax=tmax,
        models=models,
        per_model=per_model,
        path=path,
    )


def fmt_time(ms: float | None) -> str:
    if not ms:
        return "-"
    return dt.datetime.fromtimestamp(ms / 1000).strftime("%Y-%m-%d %H:%M")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", type=pathlib.Path, default=None, help="会话目录")
    ap.add_argument(
        "--update",
        type=pathlib.Path,
        default=None,
        metavar="FILE",
        help="把统计结果写回该文件的 <!-- ai-usage:start/end --> 标记块",
    )
    args = ap.parse_args()

    base = args.dir or default_session_dir()
    if not base.is_dir():
        sys.exit(f"找不到会话目录：{base}\n可用 --dir 指定")

    sessions = []
    for d in sorted(base.iterdir(), key=lambda p: p.stat().st_mtime):
        f = d / "session.v4.jsonl.zstd"
        if not f.exists():
            continue
        s = read_session(f)
        if s and s["usage"]["total"] > 0:
            sessions.append(s)

    if not sessions:
        sys.exit("没有找到含用量的会话记录")

    main_s = [s for s in sessions if (s["depth"] or 0) == 0]
    sub_s = [s for s in sessions if (s["depth"] or 0) > 0]

    def total(group, key):
        return sum(s["usage"][key] for s in group)

    if args.update:
        write_block(args.update, base, sessions)
        return

    print(f"会话目录：{base}")
    print(f"会话数量：主会话 {len(main_s)} 个，子代理 {len(sub_s)} 个\n")

    print("| 会话 | 类型 | 轮次 | 时间范围 | input | output | cacheRead | total |")
    print("| --- | --- | ---: | --- | ---: | ---: | ---: | ---: |")
    for s in sessions:
        kind = "主会话" if (s["depth"] or 0) == 0 else "子代理"
        name = s["path"].parent.name[:12]
        u = s["usage"]
        span = f"{fmt_time(s['tmin'])} ~ {fmt_time(s['tmax'])}"
        print(
            f"| `{name}` | {kind} | {s['turns']} | {span} | "
            f"{u['input']:,} | {u['output']:,} | {u['cache']:,} | {u['total']:,} |"
        )

    print()
    print("| 汇总 | input | output | cacheRead | total |")
    print("| --- | ---: | ---: | ---: | ---: |")
    for label, group in (("主会话", main_s), ("子代理", sub_s), ("合计", sessions)):
        print(
            f"| {label} | {total(group,'input'):,} | {total(group,'output'):,} | "
            f"{total(group,'cache'):,} | {total(group,'total'):,} |"
        )

    print()
    print("| 模型 | 步数 | input | output | cacheRead | total |")
    print("| --- | ---: | ---: | ---: | ---: | ---: |")
    merged: dict[str, dict[str, int]] = collections.defaultdict(
        lambda: dict(input=0, output=0, cache=0, total=0, steps=0)
    )
    for s in sessions:
        for m, v in s["per_model"].items():
            for k in ("input", "output", "cache", "total", "steps"):
                merged[m][k] += v[k]
    for m, v in sorted(merged.items(), key=lambda kv: -kv[1]["total"]):
        print(
            f"| `{m}` | {v['steps']} | {v['input']:,} | {v['output']:,} | "
            f"{v['cache']:,} | {v['total']:,} |"
        )

    all_t = [s["tmin"] for s in sessions if s["tmin"]] + [
        s["tmax"] for s in sessions if s["tmax"]
    ]
    if all_t:
        print(f"时间跨度：{fmt_time(min(all_t))} ~ {fmt_time(max(all_t))}")



def write_block(path: pathlib.Path, base: pathlib.Path, sessions: list[dict]) -> None:
    """把统计结果写回 Markdown 的标记块。"""
    import datetime as _dt

    main_s = [s for s in sessions if (s["depth"] or 0) == 0]
    sub_s = [s for s in sessions if (s["depth"] or 0) > 0]
    merged: dict[str, dict[str, int]] = collections.defaultdict(
        lambda: dict(input=0, output=0, cache=0, total=0, steps=0)
    )
    for s in sessions:
        for m, v in s["per_model"].items():
            for k in ("input", "output", "cache", "total", "steps"):
                merged[m][k] += v[k]

    def tot(group, key):
        return sum(s["usage"][key] for s in group)

    now = _dt.datetime.now().strftime("%Y-%m-%d %H:%M")
    lines = [f"统计时点：**{now}**（会话仍在继续，重跑本脚本可刷新）", ""]
    lines += ["**按模型**", "", "| 模型 | 步数 | input | output | cacheRead | total |",
              "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for m, v in sorted(merged.items(), key=lambda kv: -kv[1]["total"]):
        lines.append(
            f"| `{m}` | {v['steps']:,} | {v['input']:,} | {v['output']:,} | "
            f"{v['cache']:,} | {v['total']:,} |"
        )
    lines.append(
        f"| **合计** | **{sum(v['steps'] for v in merged.values()):,}** | "
        f"**{sum(v['input'] for v in merged.values()):,}** | "
        f"**{sum(v['output'] for v in merged.values()):,}** | "
        f"**{sum(v['cache'] for v in merged.values()):,}** | "
        f"**{sum(v['total'] for v in merged.values()):,}** |"
    )
    lines += ["", "**按角色**", "", "| 角色 | 会话数 | input | output | cacheRead | total |",
              "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for label, group in (("主会话", main_s), ("子代理", sub_s), ("合计", sessions)):
        n = len(group)
        lines.append(
            f"| {label} | {n} | {tot(group,'input'):,} | {tot(group,'output'):,} | "
            f"{tot(group,'cache'):,} | {tot(group,'total'):,} |"
        )

    text = path.read_text(encoding="utf-8")
    head = "<!-- ai-usage:start -->"
    tail = "<!-- ai-usage:end -->"
    if head not in text or tail not in text:
        sys.exit(f"{path} 中缺少 {head} / {tail} 标记")
    pre, rest = text.split(head, 1)
    _, post = rest.split(tail, 1)
    path.write_text(pre + head + "\n" + "\n".join(lines) + "\n" + tail + post, encoding="utf-8")
    print(f"已更新 {path}")


if __name__ == "__main__":
    main()
