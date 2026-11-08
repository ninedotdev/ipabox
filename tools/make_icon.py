from PIL import Image, ImageDraw, ImageFilter

SRC = "assets/ipabox-art.png"
DST = "assets/ipabox-icon-1024.png"

CANVAS, BODY, INSET = 1024, 824, 100
RADIUS = int(BODY * 0.2246)

art = Image.open(SRC).convert("RGB").resize((BODY, BODY), Image.LANCZOS)

mask = Image.new("L", (BODY, BODY), 0)
ImageDraw.Draw(mask).rounded_rectangle([0, 0, BODY - 1, BODY - 1], radius=RADIUS, fill=255)

canvas = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))

shadow = Image.new("L", (CANVAS, CANVAS), 0)
ImageDraw.Draw(shadow).rounded_rectangle(
    [INSET, INSET + 12, INSET + BODY - 1, INSET + BODY - 1 + 12], radius=RADIUS, fill=90
)
shadow = shadow.filter(ImageFilter.GaussianBlur(14))
canvas.paste((0, 0, 0), (0, 0), shadow)

body = Image.new("RGBA", (BODY, BODY))
body.paste(art, (0, 0), mask)
canvas.alpha_composite(body, (INSET, INSET))

canvas.save(DST)
print("wrote", DST, canvas.size)
