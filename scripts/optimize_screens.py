#!/usr/bin/env python3
"""把截图脚本产出的 2x PNG 降采样并转成 WebP（README 用）。

为什么要转：2x PNG 单张 300KB+，三张就接近 1MB；WebP 同等观感通常只有 1/4。
README 里引用 .webp，GitHub 正常渲染。
"""

from pathlib import Path

from PIL import Image

OUT = Path("docs/screenshots")
TARGET_WIDTH = 1400
QUALITY = 88


def main() -> None:
    pngs = sorted(OUT.glob("*.png"))
    if not pngs:
        print("没有找到 PNG，跳过")
        return
    for png in pngs:
        im = Image.open(png).convert("RGB")
        if im.width > TARGET_WIDTH:
            h = round(im.height * TARGET_WIDTH / im.width)
            im = im.resize((TARGET_WIDTH, h), Image.Resampling.LANCZOS)
        webp = png.with_suffix(".webp")
        im.save(webp, "WEBP", quality=QUALITY, method=6)
        png.unlink()  # 只保留 WebP，避免仓库里留两份
        print(f"  {webp.name}  {im.width}x{im.height}  {webp.stat().st_size // 1024} KB")


if __name__ == "__main__":
    main()