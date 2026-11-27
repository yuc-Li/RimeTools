#!/usr/bin/env python3
"""生成 RimeTools 图标（可复现，无外部依赖，仅需 Pillow）。

设计意图：
- 背景：青绿渐变圆角方块，取自 Rime（雾凇/霜）的冷暖对比里的青色调；
- 主体：一张白色的「候选窗/词条卡片」，内部三行条目，首行用琥珀色高亮 ——
  直接对应「输入法候选列表 + 词库条目」，比抽象的字母缩略更容易在 32px 下辨认；
- 全部几何按 1024 基准绘制，再按 4 倍超采样缩放，保证小尺寸边缘干净。

用法：python3 scripts/gen-icons.py
产物：src-tauri/icons/{32x32,48x48,64x64,128x128,256x256,512x512,icon}.png
"""

from __future__ import annotations

import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

BASE = 1024          # 设计基准尺寸
SS = 4               # 超采样倍数
OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"

# ── 配色 ──
BG_TOP = (26, 168, 148)      # 青绿
BG_BOTTOM = (10, 92, 86)     # 深青
CARD = (255, 255, 255)
ROW_ACTIVE = (245, 166, 61)  # 琥珀：选中态
ROW_IDLE = (186, 203, 201)   # 灰绿：未选中
SHADOW = (6, 52, 50, 90)

# ── 几何（1024 基准，见下方 draw_icon 的注释说明）──
CARD_X0, CARD_Y0, CARD_X1, CARD_Y1 = 176, 288, 848, 736
CARD_R = 60
ROW_X0 = 232
ROW_H = 72
# (上边, 下边, 右边, 颜色)：三行等高、间距一致，距卡片上下各 80
ROWS = [
    (368, 368 + ROW_H, 792, ROW_ACTIVE),
    (476, 476 + ROW_H, 660, ROW_IDLE),
    (584, 584 + ROW_H, 540, ROW_IDLE),
]


def rounded_mask(size: int, radius: int) -> Image.Image:
    m = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(m)
    d.rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    return m


def vertical_gradient(size: int, top: tuple[int, int, int], bottom: tuple[int, int, int]) -> Image.Image:
    grad = Image.new("RGB", (1, size))
    for y in range(size):
        t = y / max(1, size - 1)
        # 轻微缓动，避免中段过亮
        t = t ** 0.92
        grad.putpixel(
            (0, y),
            tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(3)),
        )
    return grad.resize((size, size), Image.Resampling.NEAREST)


def draw_icon(size: int = BASE * SS) -> Image.Image:
    s = size
    k = s / BASE  # 缩放系数

    def sc(*vals: float) -> tuple[float, ...]:
        return tuple(v * k for v in vals)

    # 1) 背景：圆角 + 竖直渐变
    canvas = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    bg = vertical_gradient(s, BG_TOP, BG_BOTTOM).convert("RGBA")
    bg.putalpha(rounded_mask(s, int(224 * k)))
    canvas.alpha_composite(bg)

    # 2) 卡片投影（几何与卡片一致，保证四边留白均匀）
    shadow = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        sc(CARD_X0, CARD_Y0 + 22, CARD_X1, CARD_Y1 + 22), radius=CARD_R * k, fill=SHADOW
    )
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(28 * k)))

    d = ImageDraw.Draw(canvas)

    # 3) 白色卡片
    d.rounded_rectangle(
        sc(CARD_X0, CARD_Y0, CARD_X1, CARD_Y1), radius=CARD_R * k, fill=CARD
    )

    # 4) 三行条目：等高、等间距、上下留白对称。
    #    首行高亮并用琥珀色，直接对应「输入法候选窗里的选中项」。
    for (y0, y1, x1, color) in ROWS:
        d.rounded_rectangle(
            sc(ROW_X0, y0, x1, y1), radius=(y1 - y0) / 2 * k, fill=color
        )

    # 选中行右端的「光标」小方块，暗示正在输入
    d.rounded_rectangle(sc(702, 396, 742, 436), radius=11 * k, fill=(255, 255, 255))

    return canvas


def draw_tray(size: int = 32 * SS) -> Image.Image:
    """托盘专用图标：透明底 + 青绿卡片。

    为什么单独做一颗：系统托盘常驻，按 32px 显示；把带外层圆角底方块的应用图标
    直接缩到 32px，会显得拥挤、边框吃掉细节。这里去掉外层底，卡片占满画布，
    主体（琥珀高亮行 + 两行灰色）在小尺寸下更清楚；透明底也能同时适配浅色/深色面板。
    """
    s_size = size
    k = s_size / 1024

    canvas = Image.new("RGBA", (s_size, s_size), (0, 0, 0, 0))
    d = ImageDraw.Draw(canvas)

    # 卡片：几乎占满，留一点点边距避免贴边被裁
    x0, y0, x1, y1, r = 40, 132, 984, 892, 96
    d.rounded_rectangle(
        (x0 * k, y0 * k, x1 * k, y1 * k), radius=r * k, fill=BG_BOTTOM
    )

    # 三行：等高、等距
    rx0 = 150
    for (ry0, ry1, rx1, color) in [
        (292, 400, 874, ROW_ACTIVE),
        (456, 564, 700, (236, 244, 242)),
        (620, 728, 560, (236, 244, 242)),
    ]:
        d.rounded_rectangle(
            (rx0 * k, ry0 * k, rx1 * k, ry1 * k), radius=(ry1 - ry0) / 2 * k, fill=color
        )

    return canvas


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    big = draw_icon()

    targets = {
        "32x32.png": 32,
        "48x48.png": 48,
        "64x64.png": 64,
        "128x128.png": 128,
        "256x256.png": 256,
        "512x512.png": 512,
        "icon.png": 512,  # Tauri 默认应用图标
    }
    for name, px in targets.items():
        img = big.resize((px, px), Image.Resampling.LANCZOS)
        path = OUT / name
        img.save(path, "PNG", optimize=True)
        print(f"  {name:14} {px}x{px}  {path.stat().st_size} B")

    # 托盘专用（透明底）：
    #  - Tauri 的 Image 只接受 RGBA8，Pillow 的 RGBA 正好匹配
    tray = draw_tray()
    for name, px in {"tray-32.png": 32, "tray-64.png": 64, "tray-128.png": 128}.items():
        tray.resize((px, px), Image.Resampling.LANCZOS).save(OUT / name, "PNG", optimize=True)
        print(f"  {name:14} {px}x{px}（托盘）")

    # Windows 打包需要 .ico（多尺寸内嵌，Tauri 与资源管理器都用得上）
    ico_sizes = [16, 24, 32, 48, 64, 128, 256]
    ico_images = [big.resize((n, n), Image.Resampling.LANCZOS) for n in ico_sizes]
    ico_path = OUT / "icon.ico"
    ico_images[-1].save(ico_path, format="ICO", sizes=[(n, n) for n in ico_sizes])
    print(f"  icon.ico        {', '.join(str(n) for n in ico_sizes)}（Windows）")

    # 参考大图（便于肉眼校对，不参与打包）
    big.resize((1024, 1024), Image.Resampling.LANCZOS).save(OUT / "icon-1024.png", "PNG")
    print("  icon-1024.png  1024x1024（预览用）")


if __name__ == "__main__":
    main()