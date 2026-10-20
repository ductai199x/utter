#!/usr/bin/env python3
"""Keeps Utter's public branch (main) clean, and publishes dev to it. See dev/README.md.

From the repository root, on dev:

    uv run --no-project python dev/release.py selftest   # the rules catch what they should
    uv run --no-project python dev/release.py scan       # what would block a publish
    uv run --no-project python dev/release.py fix        # the mechanical rewrites (patterns.toml)
    uv run --no-project python dev/release.py publish [--dry-run] [--push] [-m MESSAGE]

publish makes main's next commit from dev's last commit: the private paths removed, links to them
pointed at the dev branch, everything scanned, one commit on top of main. Nothing else reaches main.
"""

from __future__ import annotations

import argparse
import fnmatch
import os
import re
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONFIG = Path(__file__).with_name("patterns.toml")
LOCAL = Path(__file__).with_name("patterns.local.toml")
MAX_BYTES = 2_000_000
LINK = re.compile(r"\]\((?:\.\./)*(docs/[^)\s]*|DESIGN\.md|CLAUDE\.md)\)")


def load() -> dict:
    cfg = tomllib.loads(CONFIG.read_text(encoding="utf-8"))
    if LOCAL.exists():
        extra = tomllib.loads(LOCAL.read_text(encoding="utf-8"))
        for key in ("rule", "fix"):
            cfg[key] = cfg.get(key, []) + extra.get(key, [])
        for key in ("private", "skip"):
            cfg[key] = cfg.get(key, []) + extra.get(key, [])
    for rule in cfg["rule"]:
        rule["_re"] = re.compile(rule["regex"])
        rule["_allow"] = [re.compile(a) for a in rule.get("allow", [])]
    return cfg


def git(*args: str, input: bytes | None = None, env: dict | None = None) -> bytes:
    r = subprocess.run(["git", *args], cwd=ROOT, input=input, capture_output=True, env=env)
    if r.returncode:
        sys.exit(f"git {' '.join(args)}: {r.stderr.decode(errors='replace').strip()}")
    return r.stdout


def private(path: str, cfg: dict) -> bool:
    return any(path.startswith(p) if p.endswith("/") else path == p for p in cfg["private"])


def skipped(path: str, cfg: dict) -> bool:
    return any(fnmatch.fnmatch(path, pat) for pat in cfg["skip"])


def hits(path: str, text: str, cfg: dict) -> list[tuple[int, str, str]]:
    """(line, rule, text) for each place `text` of `path` breaks a rule that applies to it."""
    found = []
    is_private = private(path, cfg)
    for rule in cfg["rule"]:
        if rule["scope"] == "main" and is_private:
            continue
        for n, line in enumerate(text.splitlines(), 1):
            m = rule["_re"].search(line)
            if m and not any(a.search(line) for a in rule["_allow"]):
                found.append((n, rule["name"], line.strip()[:120]))
    return found


def text_of(data: bytes) -> str | None:
    if b"\0" in data[:8192] or len(data) > MAX_BYTES:
        return None
    return data.decode("utf-8", errors="replace")


def working_files() -> list[str]:
    out = git("ls-files", "-z", "--cached", "--others", "--exclude-standard").decode()
    return sorted(p for p in out.split("\0") if p and (ROOT / p).is_file())


def cmd_scan(cfg: dict, quiet: bool = False) -> int:
    total = 0
    by_rule: dict[str, int] = {}
    for path in working_files():
        if skipped(path, cfg):
            continue
        text = text_of((ROOT / path).read_bytes())
        if text is None:
            continue
        for n, rule, line in hits(path, text, cfg):
            total += 1
            by_rule[rule] = by_rule.get(rule, 0) + 1
            if not quiet:
                print(f"{path}:{n}: [{rule}] {line}")
    why = {r["name"]: r["why"] for r in cfg["rule"]}
    for rule, n in sorted(by_rule.items()):
        print(f"{n:4d}  {rule}: {why[rule]}", file=sys.stderr)
    print("clean" if not total else f"{total} to fix", file=sys.stderr)
    return 1 if total else 0


def cmd_fix(cfg: dict) -> int:
    changed = 0
    for path in working_files():
        if skipped(path, cfg) or path.startswith("dev/"):
            continue
        data = (ROOT / path).read_bytes()
        text = text_of(data)
        if text is None:
            continue
        new = text
        for fix in cfg.get("fix", []):
            if fix.get("scope") == "main" and private(path, cfg):
                continue  # the name stays in the docs
            new = re.sub(fix["regex"], fix["replace"], new)
        if new != text:
            (ROOT / path).write_bytes(new.encode("utf-8"))
            changed += 1
            print(f"rewrote {path}")
    print(f"{changed} files")
    return 0


def cmd_selftest(cfg: dict) -> int:
    bad_rules = 0
    for rule in cfg["rule"]:
        for kind, expect in (("bad", True), ("ok", False)):
            for sample in rule.get(kind, []):
                got = bool(rule["_re"].search(sample)) and not any(a.search(sample) for a in rule["_allow"])
                if got != expect:
                    bad_rules += 1
                    print(f"[{rule['name']}] {'should catch' if expect else 'should allow'}: {sample}")
    print("rules ok" if not bad_rules else f"{bad_rules} samples wrong")
    return 1 if bad_rules else 0


def tree_files(tree: str) -> list[tuple[str, str]]:
    out = git("ls-tree", "-r", "-z", tree).decode()
    files = []
    for entry in filter(None, out.split("\0")):
        meta, path = entry.split("\t", 1)
        files.append((meta.split()[2], path))
    return files


def blobs(shas: list[str]) -> dict[str, bytes]:
    """The content of each blob, in one git process."""
    r = subprocess.run(["git", "cat-file", "--batch"], cwd=ROOT, input="".join(s + "\n" for s in shas).encode(), capture_output=True)
    data, pos, found = r.stdout, 0, {}
    for sha in shas:
        end = data.index(b"\n", pos)
        _, _, size = data[pos:end].split()
        start = end + 1
        found[sha] = data[start : start + int(size)]
        pos = start + int(size) + 1
    return found


def cmd_publish(cfg: dict, args: argparse.Namespace) -> int:
    branch = git("rev-parse", "--abbrev-ref", "HEAD").decode().strip()
    if branch != "dev":
        sys.exit(f"publish runs on dev, not {branch}")
    if git("status", "--porcelain").strip():
        sys.exit("the working tree is not clean: commit or stash first")
    if cmd_scan(cfg, quiet=True):
        sys.exit("fix what the scan lists (release.py scan), then publish")
    head = git("rev-parse", "HEAD").decode().strip()
    old_main = git("rev-parse", "refs/heads/main").decode().strip()

    index = Path(tempfile.mkdtemp()) / "index"
    env = {**os.environ, "GIT_INDEX_FILE": str(index)}
    git("read-tree", head, env=env)
    files = tree_files(head)
    for _, path in files:
        if private(path, cfg):
            git("update-index", "--force-remove", "--", path, env=env)
    kept = [(sha, p) for sha, p in files if not private(p, cfg)]
    content = blobs([sha for sha, _ in kept])
    for sha, path in kept:
        if path.endswith(".md"):
            text = text_of(content[sha])
            new = LINK.sub(lambda m: f"]({cfg['dev_url']}{m.group(1)})", text) if text is not None else None
            if new is not None and new != text:
                new_sha = git("hash-object", "-w", "--stdin", input=new.encode("utf-8")).decode().strip()
                git("update-index", "--cacheinfo", f"100644,{new_sha},{path}", env=env)
                content[sha] = new.encode("utf-8")
    tree = git("write-tree", env=env).decode().strip()

    # The last safety net: what would be on main, scanned as main.
    problems = []
    for sha, path in tree_files(tree):
        text = text_of(content[sha]) if sha in content else text_of(blobs([sha])[sha])
        if text is None or skipped(path, cfg):
            continue
        problems += [(path, *h) for h in hits(path, text, cfg)]
    if problems:
        for path, n, rule, line in problems[:50]:
            print(f"{path}:{n}: [{rule}] {line}")
        sys.exit("the tree for main is not clean")

    if git("rev-parse", f"{old_main}^{{tree}}").decode().strip() == tree:
        print("main already has this: nothing to publish")
        return 0
    message = args.message or f"Update from dev ({head[:9]})"
    for rule in cfg["rule"]:
        m = rule["_re"].search(message)
        if m and not any(a.search(message) for a in rule["_allow"]):
            sys.exit(f"the commit message breaks [{rule['name']}]: {m.group(0)!r}")
    print(git("diff", "--stat", "--stat-width=100", old_main, tree).decode().rstrip().splitlines()[-1])
    if args.dry_run:
        print("dry run: main unchanged")
        return 0
    new = git("commit-tree", tree, "-p", old_main, "-m", message).decode().strip()
    git("update-ref", "refs/heads/main", new, old_main)
    print(f"main is now {new[:9]} (from dev {head[:9]})")
    if args.push:
        git("push", "origin", "main")
        print("pushed origin/main")
    else:
        print("not pushed: git push origin main (or publish again with --push)")
    return 0


def cmd_history(cfg: dict, args: argparse.Namespace) -> int:
    """Scan every blob and commit message ever reachable in a repository (default: this one).

    --as main: what a public history must not contain (private paths count as hits themselves);
    --as dev: what no public branch may contain."""
    repo = Path(args.repo or ROOT)

    def run(*a: str, input: bytes | None = None) -> bytes:
        r = subprocess.run(["git", "-C", str(repo), *a], input=input, capture_output=True)
        if r.returncode:
            sys.exit(f"git {' '.join(a)}: {r.stderr.decode(errors='replace').strip()}")
        return r.stdout

    seen: dict[str, str] = {}
    for line in run("rev-list", "--objects", "--all").decode(errors="replace").splitlines():
        sha, _, path = line.partition(" ")
        if path:
            seen.setdefault(sha, path)
    types = run("cat-file", "--batch-check", input="".join(s + "\n" for s in seen).encode()).decode().splitlines()
    blob_shas = [t.split()[0] for t in types if t.split()[1] == "blob"]
    r = subprocess.run(["git", "-C", str(repo), "cat-file", "--batch"], input="".join(s + "\n" for s in blob_shas).encode(), capture_output=True)
    data, pos, total, by_rule = r.stdout, 0, 0, {}
    for sha in blob_shas:
        end = data.index(b"\n", pos)
        size = int(data[pos:end].split()[2])
        body = data[end + 1 : end + 1 + size]
        pos = end + 1 + size + 1
        path = seen[sha]
        if args.mode == "main" and private(path, cfg):
            total += 1
            by_rule["private-path"] = by_rule.get("private-path", 0) + 1
            if args.verbose:
                print(f"{path} (blob {sha[:9]}): [private-path]")
            continue
        text = text_of(body)
        if text is None or skipped(path, cfg):
            continue
        for n, rule, line in hits(path, text, cfg):
            total += 1
            by_rule[rule] = by_rule.get(rule, 0) + 1
            if args.verbose:
                print(f"{path}:{n} (blob {sha[:9]}): [{rule}] {line}")
    msgs = run("log", "--all", "--format=%H%x00%B%x01").decode(errors="replace").split("\x01")
    for item in filter(str.strip, msgs):
        sha, _, msg = item.strip().partition("\0")
        for rule in cfg["rule"]:
            if rule["scope"] == "main" and args.mode == "dev":
                continue
            m = rule["_re"].search(msg)
            if m and not any(a.search(msg) for a in rule["_allow"]):
                total += 1
                by_rule[f"message:{rule['name']}"] = by_rule.get(f"message:{rule['name']}", 0) + 1
                if args.verbose:
                    print(f"commit {sha[:9]}: [{rule['name']}] {m.group(0)!r}")
    for k, n in sorted(by_rule.items()):
        print(f"{n:5d}  {k}", file=sys.stderr)
    print("clean" if not total else f"{total} hits across {len(blob_shas)} blobs", file=sys.stderr)
    return 1 if total else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("selftest", "scan", "fix"):
        sub.add_parser(name)
    pub = sub.add_parser("publish")
    pub.add_argument("--dry-run", action="store_true")
    pub.add_argument("--push", action="store_true")
    pub.add_argument("-m", "--message")
    hist = sub.add_parser("history")
    hist.add_argument("--repo")
    hist.add_argument("--as", dest="mode", choices=["dev", "main"], default="main")
    hist.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()
    cfg = load()
    simple = {"selftest": cmd_selftest, "scan": cmd_scan, "fix": cmd_fix}
    if args.cmd in simple:
        return simple[args.cmd](cfg)
    return (cmd_history if args.cmd == "history" else cmd_publish)(cfg, args)


if __name__ == "__main__":
    sys.exit(main())
