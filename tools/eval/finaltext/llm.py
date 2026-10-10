"""A local LLM for building eval and training data: Utter's own pinned llama-server (the
`llama-cpp` runtime component), started on a free port with several parallel slots; or, for
bulk work, a server elsewhere that speaks the same API (`Remote`).

Dev only. The product talks to llama-server from Rust (crates/utter-llm).
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import time
import urllib.request
from pathlib import Path
from types import TracebackType
from typing import Self

from eval.paths import data_dir

RUNTIME = data_dir() / "runtime"


def _component_dir(prefix: str) -> Path:
    dirs = sorted(
        p for p in RUNTIME.glob(f"{prefix}-*") if p.is_dir() and not p.name.endswith(".part")
    )
    if not dirs:
        raise FileNotFoundError(f"runtime component {prefix} is not installed (utter-cli setup)")
    return dirs[-1]


def model_path(component: str) -> Path:
    """The GGUF inside an installed model component, e.g. `gemma4-26b-a4b-qat`."""
    ggufs = list(_component_dir(component).glob("*.gguf"))
    if len(ggufs) != 1:
        raise FileNotFoundError(f"{component}: expected one .gguf, found {len(ggufs)}")
    return ggufs[0]


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Chat:
    """An OpenAI-style chat endpoint at `url`."""

    url: str
    # Sent as the request's "model"; llama-server serves one model and ignores it.
    served_model: str | None = None

    def chat(
        self,
        messages: list[dict[str, str]],
        max_tokens: int = 1024,
        temperature: float = 0.7,
        seed: int | None = None,
        json_schema: dict | None = None,
    ) -> str:
        body: dict = {
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "chat_template_kwargs": {"enable_thinking": False},
        }
        if self.served_model is not None:
            body["model"] = self.served_model
        if seed is not None:
            body["seed"] = seed
        if json_schema is not None:
            body["response_format"] = {
                "type": "json_schema",
                # The name is required by the OpenAI API (vLLM checks it); llama-server ignores it.
                "json_schema": {"name": "reply", "schema": json_schema},
            }
        req = urllib.request.Request(
            f"{self.url}/v1/chat/completions",
            data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=600) as r:
            out = json.load(r)
        return out["choices"][0]["message"]["content"]


class Remote(Chat):
    """A server someone else runs (e.g. vLLM on FLAME, reached over an SSH port-forward):
    `with Remote(url) as llm: llm.chat(messages)`. Asks it which model it serves."""

    def __init__(self, url: str) -> None:
        self.url = url.rstrip("/")

    def __enter__(self) -> Self:
        with urllib.request.urlopen(f"{self.url}/v1/models", timeout=10) as r:
            self.served_model = json.load(r)["data"][0]["id"]
        return self

    def __exit__(self, *exc: object) -> None:
        pass


class Server(Chat):
    """`with Server(model, slots=4) as llm: llm.chat(messages)`."""

    def __init__(self, model: Path, slots: int = 4, context_per_slot: int = 4096) -> None:
        self.model = model
        self.slots = slots
        self.context = context_per_slot * slots
        self.port = _free_port()
        self.url = f"http://127.0.0.1:{self.port}"
        self.proc: subprocess.Popen[bytes] | None = None

    def __enter__(self) -> Self:
        llama = _component_dir("llama-cpp")
        cuda = _component_dir("cuda-runtime")
        exe = llama / ("llama-server.exe" if os.name == "nt" else "llama-server")
        env = dict(os.environ)
        env["PATH"] = os.pathsep.join([str(llama), str(cuda), env.get("PATH", "")])
        log = data_dir() / "logs" / "llama-server-tools.log"
        log.parent.mkdir(parents=True, exist_ok=True)
        args = [
            str(exe),
            "--model", str(self.model),
            "--host", "127.0.0.1",
            "--port", str(self.port),
            "--ctx-size", str(self.context),
            "--parallel", str(self.slots),
            "--n-gpu-layers", "999",
            "--flash-attn", "on",
            "--no-webui",
            "--jinja",
        ]  # fmt: skip
        with log.open("wb") as out:
            self.proc = subprocess.Popen(args, stdout=out, stderr=subprocess.STDOUT, env=env)
        deadline = time.monotonic() + 300
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError(f"llama-server exited ({self.proc.returncode}); see {log}")
            try:
                with urllib.request.urlopen(f"{self.url}/health", timeout=2) as r:
                    if r.status == 200:
                        return self
            except OSError:
                pass
            time.sleep(0.5)
        raise TimeoutError(f"llama-server did not become ready; see {log}")

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        if self.proc is not None:
            self.proc.kill()
            self.proc.wait()
