"""Logo concepts for Utter through OpenAI's Images API, for choosing among, never for shipping:
an app icon must be crisp at 16 px, so the chosen concept is rebuilt as a clean two-colour SVG
(the chess companion's tools/brand did the same, Tai, 2026-09-21).

    uv run python -m brand.logo_ideas --dry-run      # the estimate, no call
    uv run python -m brand.logo_ideas                # 8 concepts x 2 renders
    uv run python -m brand.logo_ideas --only wave-caret ripple-line --n 4 --quality high

The key is OPENAI_API_KEY from the environment, or from the .env file named by --env-file.
Every call goes on a ledger beside the images and the run stops at --budget-usd. Images land
under <dev data>/brand/<stamp>/ with their prompts, and a contact sheet (sheet.png, on the
splash's night blue) shows them all at once.
Pricing (2026-09-21): output $30 per million tokens, text input $5; a 1024x1024 image is about
1,100 tokens at medium and 4,200 at high (the response's `usage` corrects it per call).
"""

from __future__ import annotations

import argparse
import base64
import concurrent.futures as cf
import json
import os
import threading
import time
from pathlib import Path

import httpx

from eval.paths import data_dir

ENDPOINT = "https://api.openai.com/v1/images/generations"
PRICE_OUT_PER_M = 30.0
PRICE_IN_PER_M = 5.0
TOKENS_PER_IMAGE = {"low": 300, "medium": 1100, "high": 4200}

# What Utter is, for every concept: hold a key, speak, and clean text appears where you type;
# local, private, calm. The mark has to work at 16 px in the tray and at 1024 on the splash.
STYLE = (
    " Flat vector app icon mark: one bold, simple symbol in at most two colours -- a soft"
    " electric indigo (#6B7CFF) for the main shape and deep night blue (#0E1330) for details --"
    " that also works as a one-colour silhouette. Calm, friendly, modern, balanced geometry,"
    " smooth curves, thick even strokes, generous negative space. No gradients, no shading, no"
    " 3D, no glow, no text unless the concept asks for one letter. Fully transparent background,"
    " one centered mark with generous padding, clean alpha edges, no backdrop, no rounded-square"
    " tile, no checkerboard, no watermark. It must read clearly at 16 pixels and at 1024."
)

CONCEPTS: dict[str, str] = {
    "wave-caret": (
        "Logo for a voice dictation app that turns speech into clean typed text: a smooth sound"
        " wave that flows from left to right and, as it reaches the right, settles into a"
        " perfectly straight horizontal line that ends in a tall text cursor (an I-beam caret)."
    ),
    "ripple-line": (
        "Logo for a voice-to-text app: three concentric arcs of sound ripples spreading from a"
        " small dot on the left, the outermost arc straightening into a horizontal line of text,"
        " like speech becoming a written line."
    ),
    "bubble-caret": (
        "Logo for a calm voice typing app: a soft rounded speech bubble, and inside it in negative"
        " space a single blinking text cursor (a vertical bar), centered."
    ),
    "u-wave": (
        "Logo monogram for an app called Utter: a lowercase letter u drawn as one continuous"
        " smooth stroke whose bottom curve gently ripples like a sound wave."
    ),
    "breath-lines": (
        "Logo for a voice dictation app: a single flowing ribbon, like a breath or a voice,"
        " that curls and lays itself down into three neat horizontal lines of a paragraph."
    ),
    "nib-wave": (
        "Logo for an app where you write by speaking: a fountain pen nib, its central slit"
        " drawn as a small sound wave."
    ),
    "mic-ibeam": (
        "Logo for a speech-to-text app: a simple rounded microphone capsule whose stand is a"
        " text I-beam cursor, merged into one clean symbol."
    ),
    "echo-stack": (
        "Logo for a voice typing app: four rounded vertical bars of an audio level meter, of"
        " varying heights, where the last bar on the right becomes a slim blinking text cursor."
    ),
}


class Ledger:
    """Every paid call on the record; the run stops at the budget."""

    def __init__(self, path: Path, budget_usd: float) -> None:
        self.path, self.budget = path, budget_usd
        self.spent = 0.0
        self.lock = threading.Lock()

    def allow(self, estimate: float) -> bool:
        with self.lock:
            return self.spent + estimate <= self.budget

    def record(self, concept: str, usage: dict, n: int, quality: str) -> float:
        out_tokens = usage.get("output_tokens") or TOKENS_PER_IMAGE[quality] * n
        in_tokens = usage.get("input_tokens") or 0
        cost = out_tokens * PRICE_OUT_PER_M / 1e6 + in_tokens * PRICE_IN_PER_M / 1e6
        with self.lock:
            self.spent += cost
            with self.path.open("a", encoding="utf-8") as fh:
                fh.write(
                    json.dumps({"concept": concept, "usage": usage, "usd": round(cost, 4)}) + "\n"
                )
        return cost


def read_key(env_file: str | None) -> str:
    if os.environ.get("OPENAI_API_KEY"):
        return os.environ["OPENAI_API_KEY"]
    if env_file:
        for line in Path(env_file).read_text(encoding="utf-8").splitlines():
            if line.strip().startswith("OPENAI_API_KEY="):
                return line.split("=", 1)[1].strip().strip('"').strip("'")
    raise SystemExit("no OPENAI_API_KEY in the environment or the --env-file")


def generate(
    client: httpx.Client, key: str, model: str, prompt: str, n: int, quality: str
) -> tuple[list[bytes], dict]:
    body = {
        "model": model,
        "prompt": prompt,
        "n": n,
        "size": "1024x1024",
        "quality": quality,
        "background": "transparent",
        "output_format": "png",
    }
    response = client.post(ENDPOINT, headers={"Authorization": f"Bearer {key}"}, json=body)
    if response.status_code != 200:
        raise RuntimeError(f"{response.status_code}: {response.text[:400]}")
    data = response.json()
    return [base64.b64decode(d["b64_json"]) for d in data.get("data", [])], data.get("usage") or {}


def contact_sheet(folder: Path, labels: list[tuple[str, Path]], cell: int = 256) -> Path:
    """Every image on the splash's night blue with its name under it, one PNG."""
    from PIL import Image, ImageDraw

    cols = 4
    rows = (len(labels) + cols - 1) // cols
    pad, text_h = 16, 22
    sheet = Image.new(
        "RGB", (cols * (cell + pad) + pad, rows * (cell + pad + text_h) + pad), "#0E1330"
    )
    draw = ImageDraw.Draw(sheet)
    for k, (name, path) in enumerate(labels):
        img = Image.open(path).convert("RGBA")
        img.thumbnail((cell, cell))
        x = pad + (k % cols) * (cell + pad)
        y = pad + (k // cols) * (cell + pad + text_h)
        sheet.paste(img, (x, y), img)
        draw.text((x, y + cell + 4), name, fill="#C8CEF5")
    out = folder / "sheet.png"
    sheet.save(out)
    return out


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", default="gpt-image-2.5-sunburst")
    parser.add_argument("--quality", default="medium", choices=list(TOKENS_PER_IMAGE))
    parser.add_argument("--n", type=int, default=2, help="renders per concept")
    parser.add_argument("--only", nargs="*", default=None)
    parser.add_argument("--budget-usd", type=float, default=2.0)
    parser.add_argument("--env-file", default=None)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    names = args.only or list(CONCEPTS)
    each = TOKENS_PER_IMAGE[args.quality] * args.n * PRICE_OUT_PER_M / 1e6
    print(f"{len(names)} concepts x {args.n} at {args.quality}: about ${each * len(names):.2f}")
    if args.dry_run:
        return
    key = read_key(args.env_file)
    folder = data_dir() / "brand" / time.strftime("%Y%m%d-%H%M%S")
    folder.mkdir(parents=True)
    ledger = Ledger(folder / "ledger.jsonl", args.budget_usd)
    labels: list[tuple[str, Path]] = []

    def one(name: str) -> None:
        if not ledger.allow(each):
            print(f"{name}: skipped, the budget is spent")
            return
        prompt = CONCEPTS[name] + STYLE
        images, usage = generate(client, key, args.model, prompt, args.n, args.quality)
        cost = ledger.record(name, usage, args.n, args.quality)
        (folder / f"{name}.prompt.txt").write_text(prompt, encoding="utf-8")
        for i, png in enumerate(images, 1):
            path = folder / f"{name}-{i}.png"
            path.write_bytes(png)
            labels.append((f"{name}-{i}", path))
        print(f"{name}: {len(images)} images, ${cost:.3f}")

    with httpx.Client(timeout=300) as client, cf.ThreadPoolExecutor(4) as pool:
        for f in [pool.submit(one, n) for n in names]:
            try:
                f.result()
            except Exception as e:  # noqa: BLE001 -- one concept failing should not stop the rest
                print(f"failed: {e}")
    labels.sort()
    if labels:
        print(f"sheet: {contact_sheet(folder, labels)}")
    print(f"spent ${ledger.spent:.2f} of ${ledger.budget:.2f}; {folder}")


if __name__ == "__main__":
    main()
