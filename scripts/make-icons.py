"""Draws the app and tray icons (no dependencies). Run from the repo root:

    python3 scripts/make-icons.py && pnpm tauri icon app-icon.png

Writes app-icon.png (1024 px source for `tauri icon`) and the tray icons in src-tauri/icons/.
"""
import struct
import zlib

AMBER = (245, 158, 11)
RED = (239, 68, 68)
SLATE = (100, 116, 139)
WHITE = (255, 255, 255)
BLACK = (0, 0, 0)


def rrect(x, y, x0, y0, x1, y1, r):
    if not (x0 <= x <= x1 and y0 <= y <= y1):
        return False
    cx, cy = min(max(x, x0 + r), x1 - r), min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def poly(x, y, pts):
    inside, j = False, len(pts) - 1
    for i, (xi, yi) in enumerate(pts):
        xj, yj = pts[j]
        if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi:
            inside = not inside
        j = i
    return inside


# Glyph in a 36-unit box: laptop screen outline, base, and a bolt on the screen.
BOLT = [(19.5, 9), (12.5, 18.5), (17, 18.5), (15.5, 25), (23.5, 15), (18.8, 15), (20.8, 9)]


def laptop(x, y, stroke=2.6):
    screen = rrect(x, y, 6, 5, 30, 25, 3) and not rrect(x, y, 6 + stroke, 5 + stroke, 30 - stroke, 25 - stroke, 1.2)
    return screen or rrect(x, y, 2, 27, 34, 30.5, 1.5)


def render(path, size, pixel, ss=3):
    """pixel(u, v) -> (r, g, b, a) or None, with u, v in 0..1."""
    rows = bytearray()
    for py in range(size):
        rows.append(0)
        for px in range(size):
            acc = [0, 0, 0, 0]
            for i in range(ss):
                for j in range(ss):
                    c = pixel((px + (i + 0.5) / ss) / size, (py + (j + 0.5) / ss) / size)
                    if c:
                        a = c[3] / 255
                        acc[0] += c[0] * a
                        acc[1] += c[1] * a
                        acc[2] += c[2] * a
                        acc[3] += a
            n = ss * ss
            alpha = acc[3] / n
            rgb = [round(acc[k] / acc[3]) if acc[3] else 0 for k in range(3)]
            rows += bytes(rgb + [round(255 * alpha)])

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(bytes(rows), 9)))
        f.write(chunk(b"IEND", b""))


def slash(x, y, w=2.6):
    """Diagonal bar across the glyph, for 'off'."""
    return abs((x - 4) - (y - 2) * (28 / 30)) * 0.73 < w / 2 and 3 <= x <= 33 and 2 <= y <= 33


def badge(x, y, r=6.0):
    return (x - 29) ** 2 + (y - 7) ** 2 <= r * r


def glyph(x, y, look, stroke=2.6):
    """The laptop plus the state mark: bolt (awake), badge dot (attention), slash (off)."""
    if look == "attention":
        if badge(x, y):
            return True
        if badge(x, y, 8.4):  # clear ring so the dot reads on top of the frame
            return False
    if look == "off" and slash(x, y):
        return True
    if look == "off" and slash(x, y, 6.0):  # gap around the slash
        return False
    return laptop(x, y, stroke) or (look == "awake" and poly(x, y, BOLT))


def template(look):
    """macOS menu bar: black glyph on transparent; the system tints it."""
    def px(u, v):
        return (*BLACK, 255) if glyph(u * 36, v * 36, look) else None
    return px


def colored(look):
    """Windows/Linux trays: white glyph on a colored tile, readable on light and dark bars."""
    tile = {"awake": AMBER, "attention": RED}.get(look, SLATE)

    def px(u, v):
        x, y = u * 36, v * 36
        if not rrect(x, y, 0, 0, 36, 36, 7):
            return None
        # Glyph scaled into the tile's padding.
        gx, gy = 4 + (x - 4) * 36 / 28, 5 + (y - 5) * 36 / 28
        return (*WHITE, 255) if glyph(gx, gy, look, 3.2) else (*tile, 255)
    return px


def app_icon(u, v):
    # macOS grid: 824/1024 body, ~185 px corner radius; other OSes use the same art.
    m = 100 / 1024
    if not rrect(u, v, m, m, 1 - m, 1 - m, 185 / 1024):
        return None
    t = (v - m) / (1 - 2 * m)
    bg = tuple(round(a + (b - a) * t) for a, b in zip((30, 41, 59), (15, 23, 42)))
    # Glyph (x 2..34, y 5..30.5) centred in the middle half of the icon.
    x, y = 2 + (u - 0.25) * 64, 5 + (v - 0.301) * 64
    if poly(18 + (x - 18) / 0.8, 17 + (y - 15.5) / 0.8, BOLT):  # bolt at 80%, raised to clear the frame
        return (*AMBER, 255)
    if laptop(x, y, 2.4):
        return (*WHITE, 255)
    return (*bg, 255)


if __name__ == "__main__":
    for look in ("idle", "awake", "attention", "off"):
        render(f"src-tauri/icons/tray-{look}.png", 36, template(look))
        render(f"src-tauri/icons/tray-{look}-color.png", 32, colored(look))
    render("app-icon.png", 1024, app_icon, ss=2)
    print("icons written")
