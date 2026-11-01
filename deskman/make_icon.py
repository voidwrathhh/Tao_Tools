"""生成 deskman 应用图标 (.ico)

设计：深色圆角方底 + 三层错位玻璃面板（紫→蓝渐变）+ 白色高光边
寓意：多面板管理 / 桌面整理 / 科技感
"""
from PIL import Image, ImageDraw, ImageFilter
import math

SIZE = 1024  # 高分辨率底稿，缩小到各尺寸


def rounded_rect(draw, xy, radius, fill=None, outline=None, width=1):
    draw.rounded_rectangle(xy, radius=radius, fill=fill, outline=outline, width=width)


def lerp_color(c1, c2, t):
    return tuple(int(a + (b - a) * t) for a, b in zip(c1, c2))


def build_base():
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # ---- 1. 深色圆角方底 ----
    bg_color = (18, 18, 26, 255)
    bg_margin = int(SIZE * 0.02)
    bg_radius = int(SIZE * 0.22)
    rounded_rect(draw, (bg_margin, bg_margin, SIZE - bg_margin, SIZE - bg_margin),
                 bg_radius, fill=bg_color)

    # ---- 2. 底板微光（从上到下的微弱渐变） ----
    for y in range(bg_margin, SIZE - bg_margin, 4):
        t = (y - bg_margin) / (SIZE - bg_margin * 2)
        glow_alpha = int(20 * (1 - t))
        if glow_alpha > 0:
            c = (60, 40, 120, glow_alpha)
            draw.rectangle((bg_margin, y, SIZE - bg_margin, y + 4), fill=c)

    img = img.filter(ImageFilter.GaussianBlur(1))

    # ---- 3. 三层错位面板（紫→蓝渐变，从左下到右上递进） ----
    img2 = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    d2 = ImageDraw.Draw(img2)

    c1 = (138, 84, 255)  # 紫
    c2 = (72, 160, 255)  # 蓝

    panel_margin = int(SIZE * 0.16)
    panel_size = SIZE - panel_margin * 2
    offset_step = int(SIZE * 0.07)
    panel_radius = int(SIZE * 0.06)

    # 面板 1（最左下，最暗）
    p1_x = panel_margin
    p1_y = panel_margin + offset_step * 2
    p1_color = lerp_color(c1, c2, 0.0)
    p1_alpha = 100
    rounded_rect(d2, (p1_x, p1_y, p1_x + panel_size, p1_y + panel_size),
                 panel_radius, fill=(*p1_color, p1_alpha),
                 outline=(*lerp_color(p1_color, (255, 255, 255), 0.3), 120), width=4)

    # 面板 2（中间）
    p2_x = panel_margin + offset_step
    p2_y = panel_margin + offset_step
    p2_color = lerp_color(c1, c2, 0.5)
    p2_alpha = 160
    rounded_rect(d2, (p2_x, p2_y, p2_x + panel_size, p2_y + panel_size),
                 panel_radius, fill=(*p2_color, p2_alpha),
                 outline=(*lerp_color(p2_color, (255, 255, 255), 0.4), 160), width=4)

    # 面板 3（最右上，最亮）
    p3_x = panel_margin + offset_step * 2
    p3_y = panel_margin
    p3_color = lerp_color(c1, c2, 1.0)
    p3_alpha = 230
    rounded_rect(d2, (p3_x, p3_y, p3_x + panel_size, p3_y + panel_size),
                 panel_radius, fill=(*p3_color, p3_alpha),
                 outline=(*lerp_color(p3_color, (255, 255, 255), 0.5), 220), width=5)

    # 面板 3 内容线条（模拟 UI 元素）
    line_color = (255, 255, 255, 160)
    line_w = int(SIZE * 0.012)
    inner_pad = int(SIZE * 0.045)
    # 横线 1（标题栏）
    d2.line((p3_x + inner_pad, p3_y + inner_pad + line_w,
             p3_x + panel_size - inner_pad, p3_y + inner_pad + line_w),
            fill=line_color, width=line_w)
    # 横线 2（内容行）
    y2 = p3_y + inner_pad + line_w * 5
    d2.line((p3_x + inner_pad, y2, p3_x + panel_size * 0.6, y2),
            fill=(255, 255, 255, 100), width=line_w)
    # 横线 3
    y3 = y2 + line_w * 4
    d2.line((p3_x + inner_pad, y3, p3_x + panel_size * 0.45, y3),
            fill=(255, 255, 255, 70), width=line_w)

    # 小方块（模拟图标占位）
    sq_size = int(SIZE * 0.06)
    sq_x = p3_x + inner_pad
    sq_y = y3 + line_w * 3
    d2.rounded_rectangle((sq_x, sq_y, sq_x + sq_size, sq_y + sq_size),
                         radius=int(sq_size * 0.3), fill=(255, 255, 255, 80))

    # 合成
    img = Image.alpha_composite(img, img2)

    # ---- 4. 高光/光效 ----
    img3 = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    d3 = ImageDraw.Draw(img3)

    # 右上角光斑
    glow_r = int(SIZE * 0.18)
    gx, gy = int(SIZE * 0.72), int(SIZE * 0.22)
    for r in range(glow_r, 0, -3):
        t = 1 - r / glow_r
        a = int(30 * t * t)
        c = (180, 140, 255, a)
        d3.ellipse((gx - r, gy - r, gx + r, gy + r), fill=c)

    img3 = img3.filter(ImageFilter.GaussianBlur(int(SIZE * 0.02)))
    img = Image.alpha_composite(img, img3)

    return img


def build_icon_sizes(base):
    sizes = [16, 24, 32, 48, 64, 128, 256]
    icons = []
    for s in sizes:
        resized = base.resize((s, s), Image.LANCZOS)
        icons.append(resized)
    return icons


def main():
    base = build_base()

    # 保存预览
    preview = base.copy()
    preview.save("deskman/icon_preview.png")

    # 保存 .ico（多尺寸）
    icons = build_icon_sizes(base)
    icons[-1].save("deskman/deskman.ico",
                   format="ICO",
                   sizes=[(s, s) for s in [16, 24, 32, 48, 64, 128, 256]],
                   append_images=icons[:-1])

    print(f"图标已生成: deskman/deskman.ico ({len(icons)} sizes)")
    print(f"预览: deskman/icon_preview.png")


if __name__ == "__main__":
    main()
