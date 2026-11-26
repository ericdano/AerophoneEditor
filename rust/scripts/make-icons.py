#!/usr/bin/env python3
"""Make the app icons from the artwork in ae30-app/assets/icons/icon-source.png.

    python3 rust/scripts/make-icons.py [source.png]

The source is a square PNG with a transparent background (the artwork has
its own rounded frame and shadow). Writes, next to it:
  logo.png   512 px, the window / Dock icon built into the app
  logo.icns  macOS app icon (16-1024 px), artwork inside Apple's icon grid
             (824 of 1024 px) so it lines up with other apps in the Dock
  logo.ico   Windows icon (16-256 px), artwork filling more of the square

Needs Pillow (pip install pillow) and macOS's iconutil for the .icns.
"""
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

ICONS = Path(__file__).resolve().parent.parent / "ae30-app" / "assets" / "icons"
CANVAS = 1024


def fitted(source: Image.Image, body: int) -> Image.Image:
    """The artwork, trimmed to what's visible, scaled into `body` px and
    centred on a transparent CANVAS-sized square."""
    art = source.convert("RGBA")
    art = art.crop(art.getchannel("A").getbbox())
    scale = body / max(art.size)
    art = art.resize((round(art.width * scale), round(art.height * scale)), Image.LANCZOS)
    out = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    out.alpha_composite(art, ((CANVAS - art.width) // 2, (CANVAS - art.height) // 2))
    return out


def main() -> None:
    source = Image.open(Path(sys.argv[1]) if len(sys.argv) > 1 else ICONS / "icon-source.png")
    mac = fitted(source, 848)   # the frame plus its shadow ~ Apple's 824 px body
    win = fitted(source, 1000)

    mac.resize((512, 512), Image.LANCZOS).save(ICONS / "logo.png")
    win.save(ICONS / "logo.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])

    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "logo.iconset"
        iconset.mkdir()
        for n in (16, 32, 128, 256, 512):
            mac.resize((n, n), Image.LANCZOS).save(iconset / f"icon_{n}x{n}.png")
            mac.resize((n * 2, n * 2), Image.LANCZOS).save(iconset / f"icon_{n}x{n}@2x.png")
        out = Path(tmp) / "logo.icns"
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(out)], check=True)
        shutil.copy(out, ICONS / "logo.icns")
    print(f"Wrote logo.png, logo.ico and logo.icns in {ICONS}")


if __name__ == "__main__":
    main()
