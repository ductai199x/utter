"""The README's pictures, from the app's own pages (never shipped, not one of the five commands).

With the app's pages served (`npm run dev` in app/, port 1430), from the repository root:

    uv run --with playwright --with pillow python tools/media/record.py

It drives the installed Microsoft Edge (Playwright's msedge channel: no browser download) and
writes docs/media/:
- demo.gif: promo.html, one dictation through the real Bar, recorded at its real timings;
- history-*.png, shortcuts-*.png, rules-*.png, setup-*.png: the windows on the pages' stand-in
  data (core.ts), in the light and the dark theme, each framed on the same 16:10 backdrop.
"""

from __future__ import annotations

import base64
import io
import time
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter
from playwright.sync_api import Page, sync_playwright

PAGES = "http://localhost:1430"
OUT = Path(__file__).resolve().parents[2] / "docs" / "media"
# The GIF: Edge's frames kept at most this often (25 a second), on a palette this big.
FRAME_MS = 40
COLORS = 256


def record_demo(page: Page) -> None:
    page.set_viewport_size({"width": 960, "height": 540})
    page.goto(f"{PAGES}/promo.html")
    page.wait_for_function("window.promo !== undefined")
    # Edge's screencast: a frame each time the page paints, with when it painted.
    cdp = page.context.new_cdp_session(page)
    frames: list[tuple[float, bytes]] = []

    def on_frame(event: dict) -> None:
        frames.append((event["metadata"]["timestamp"], base64.b64decode(event["data"])))
        cdp.send("Page.screencastFrameAck", {"sessionId": event["sessionId"]})

    cdp.on("Page.screencastFrame", on_frame)
    cdp.send("Page.startScreencast", {"format": "png", "maxWidth": 960, "maxHeight": 540})
    # Not awaited: the frames come while it plays.
    page.evaluate("void window.promo.start()")
    page.wait_for_function("window.promo.done", polling=100, timeout=60_000)
    ended = time.time()
    cdp.send("Page.stopScreencast")
    kept = [frames[0]]
    for t, png in frames[1:]:
        if (t - kept[-1][0]) * 1000 >= FRAME_MS:
            kept.append((t, png))
    # The last frame holds until the promo ended (nothing painted after it).
    ends = [t for t, _ in kept[1:]] + [ended]
    durations = [round((end - t) * 1000) for (t, _), end in zip(kept, ends, strict=True)]
    images = [Image.open(io.BytesIO(png)).convert("RGB") for _, png in kept]
    # One palette for every frame (no flicker), from frames across the whole recording, so the
    # dimmed words' grays are in it as well as the cleaned text and the check.
    picks = [images[len(images) * k // 6] for k in range(1, 6)]
    strip = Image.new("RGB", (picks[0].width, picks[0].height * len(picks)))
    for k, im in enumerate(picks):
        strip.paste(im, (0, k * im.height))
    palette = strip.quantize(colors=COLORS, method=Image.Quantize.MEDIANCUT)
    gif = [im.quantize(palette=palette, dither=Image.Dither.NONE) for im in images]
    out = OUT / "demo.gif"
    gif[0].save(
        out, save_all=True, append_images=gif[1:], duration=durations, loop=0, optimize=True
    )
    print(
        f"{out.name}: {len(gif)} frames, {sum(durations) / 1000:.1f} s, {out.stat().st_size / 1e6:.1f} MB"
    )


def shot(page: Page, name: str, theme: str, clip: dict | None = None) -> None:
    out = OUT / f"{name}-{theme}.png"
    page.wait_for_timeout(400)
    window = Image.open(io.BytesIO(page.screenshot(type="png", clip=clip))).convert("RGB")
    framed(window, theme).save(out, optimize=True)
    print(f"{out.name}: {out.stat().st_size / 1e3:.0f} kB")


# Every window is set on the same backdrop, so the README's tour lines up: 16:10, at 2x.
CANVAS = (2240, 1400)
MARGIN = 120
RADIUS = 24
BACKDROP = {"light": ((238, 240, 250), (221, 226, 246)), "dark": ((11, 14, 24), (24, 30, 58))}


def framed(window: Image.Image, theme: str) -> Image.Image:
    """`window` scaled to fit the canvas inside its margin, with rounded corners and a soft
    shadow, on the theme's backdrop (the app's paper or ink, top to bottom)."""
    top, bottom = BACKDROP[theme]
    canvas = Image.new("RGB", CANVAS)
    for y in range(CANVAS[1]):
        f = y / (CANVAS[1] - 1)
        canvas.paste(
            tuple(round(a + (b - a) * f) for a, b in zip(top, bottom, strict=True)),
            (0, y, CANVAS[0], y + 1),
        )
    scale = min(
        (CANVAS[0] - 2 * MARGIN) / window.width, (CANVAS[1] - 2 * MARGIN) / window.height, 1
    )
    window = window.resize(
        (round(window.width * scale), round(window.height * scale)), Image.Resampling.LANCZOS
    )
    x, y = (CANVAS[0] - window.width) // 2, (CANVAS[1] - window.height) // 2
    mask = Image.new("L", window.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, *window.size), RADIUS, fill=255)
    shadow = Image.new("L", CANVAS, 0)
    ImageDraw.Draw(shadow).rounded_rectangle(
        (x, y + 18, x + window.width, y + window.height + 18), RADIUS, fill=110
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(36))
    canvas.paste(Image.new("RGB", CANVAS, (0, 0, 0)), (0, 0), shadow)
    canvas.paste(window, (x, y), mask)
    return canvas


def record_windows(page: Page, theme: str) -> None:
    page.set_viewport_size({"width": 1120, "height": 700})
    page.goto(f"{PAGES}/home.html")
    shot(page, "history", theme)
    page.get_by_role("button", name="Settings").click()
    # The shortcuts alone: the dictation key and paste and copy last.
    heading = page.get_by_role("heading", name="Shortcuts")
    heading.evaluate("h => h.scrollIntoView({block: 'start'})")
    top, key, last = (
        heading.bounding_box(),
        page.locator("#key").bounding_box(),
        page.get_by_text("Copy last dictation").bounding_box(),
    )
    assert top and key and last
    x, y = top["x"] - 40, top["y"] - 28
    clip = {"x": x, "y": y, "width": key["x"] + key["width"] + 40 - x, "height": last["y"] + 64 - y}
    shot(page, "shortcuts", theme, clip)
    page.set_viewport_size({"width": 880, "height": 620})
    page.goto(f"{PAGES}/rules.html")
    shot(page, "rules", theme)
    page.set_viewport_size({"width": 640, "height": 520})
    page.goto(f"{PAGES}/setup.html?mode=first")
    page.get_by_role("button", name="Continue").click()
    shot(page, "setup", theme)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch(channel="msedge")
        demo = browser.new_page(device_scale_factor=1)
        record_demo(demo)
        for theme in ("light", "dark"):
            context = browser.new_context(color_scheme=theme, device_scale_factor=2)  # type: ignore[arg-type]
            record_windows(context.new_page(), theme)
            context.close()
        browser.close()


if __name__ == "__main__":
    main()
