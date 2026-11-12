"""Generate the Syncora .ico used by the Windows executable.

Run from the project root:
    python tools/make_icon.py

The design mirrors the procedural icon in src/platform.rs: a rounded blue-violet
tile with two white arrows. Keeping this script in the repo makes the asset
reproducible without binary editing tools.
"""

from PIL import Image, ImageDraw, ImageFilter

SIZE = 1024
RADIUS = int(SIZE * 0.22)


def lerp(a, b, t):
    return int(round(a + (b - a) * t))


def make_gradient():
    small = 256
    image = Image.new("RGB", (small, small))
    pixels = image.load()
    for y in range(small):
        for x in range(small):
            t = (x + y) / (2 * (small - 1))
            pixels[x, y] = (
                lerp(86, 128, t),
                lerp(132, 92, t),
                lerp(255, 245, t),
            )
    return image.resize((SIZE, SIZE), Image.Resampling.BICUBIC)


def main():
    canvas = make_gradient().convert("RGBA")

    # Soft highlight in the top-left corner.
    glow = Image.new("L", (SIZE, SIZE), 0)
    glow_draw = ImageDraw.Draw(glow)
    glow_draw.ellipse(
        (-SIZE * 0.18, -SIZE * 0.22, SIZE * 0.62, SIZE * 0.58),
        fill=70,
    )
    glow = glow.filter(ImageFilter.GaussianBlur(SIZE * 0.12))
    white = Image.new("RGBA", (SIZE, SIZE), (255, 255, 255, 255))
    canvas = Image.composite(white, canvas, glow.point(lambda v: int(v * 0.22)))

    # Rounded mask.
    mask = Image.new("L", (SIZE, SIZE), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (0, 0, SIZE - 1, SIZE - 1), radius=RADIUS, fill=255
    )
    canvas.putalpha(mask)

    draw = ImageDraw.Draw(canvas)
    stroke = int(SIZE * 0.086)
    white = (255, 255, 255, 255)

    def p(x, y):
        return (x * SIZE, y * SIZE)

    # Top arrow pointing right.
    draw.line([p(0.27, 0.365), p(0.64, 0.365)], fill=white, width=stroke)
    draw.polygon(
        [p(0.615, 0.272), p(0.615, 0.458), p(0.775, 0.365)],
        fill=white,
    )

    # Bottom arrow pointing left.
    draw.line([p(0.73, 0.635), p(0.36, 0.635)], fill=white, width=stroke)
    draw.polygon(
        [p(0.385, 0.542), p(0.385, 0.728), p(0.225, 0.635)],
        fill=white,
    )

    canvas = canvas.resize((256, 256), Image.Resampling.LANCZOS)
    canvas.save(
        "assets/icon.ico",
        sizes=[(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (24, 24), (16, 16)],
    )
    canvas.save("assets/icon.png")
    print("wrote assets/icon.ico and assets/icon.png")


if __name__ == "__main__":
    main()
