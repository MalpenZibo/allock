#!/usr/bin/env bash
# Records the README's animation of the lock in action: allock's headless
# renderer draws it frame by frame over the test wallpaper, nothing of the
# session. Needs a GPU adapter, and Python with Pillow.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo test --release demo -- --ignored --nocapture
# An animated WebP: frame times in milliseconds, so it plays at 60 fps where a
# GIF cannot, and in full colour. Through libwebp's own animation encoder:
# ffmpeg's piles its lossy partial frames up into a smear.
python3 - <<'PY'
import os
from PIL import Image

W, H = 1280, 800
SIZE = W * H * 4
path = "target/snapshots/demo.rgba"
count = os.path.getsize(path) // SIZE
with open(path, "rb") as raw:
    def frame():
        return Image.frombytes("RGBA", (W, H), raw.read(SIZE)).convert("RGB")
    first = frame()
    rest = (frame() for _ in range(count - 1))
    # 60 fps in whole milliseconds.
    durations = [16 if i % 3 == 2 else 17 for i in range(count)]
    first.save("assets/screenshots/demo.webp", save_all=True, append_images=rest,
               duration=durations, loop=0, quality=85, method=4)
PY
rm target/snapshots/demo.rgba
ls -lh assets/screenshots/demo.webp
