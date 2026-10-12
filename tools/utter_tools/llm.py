"""A language model behind an OpenAI-style chat API, for the tools (the app talks to llama-server
from Rust, crates/utter-llm):

- `Server`: Utter's own llama-server on a free local port with several parallel slots (the grader,
  data generation on this machine);
- `Remote`: a server someone else runs, e.g. vLLM on a GPU cluster, for bulk work.

`connect(url, model)` picks one: `with connect(url) as llm: llm.chat(messages)`.
"""

from __future__ import annotations

import http.client
import json
import socket
import subprocess
import threading
import time
import urllib.parse
import urllib.request
from pathlib import Path
from types import TracebackType
from typing import Self

from utter_tools import config

# The local reference model: the grader and the data generator's default (an optional component,
# utter-cli setup gemma4-26b-a4b-qat).
REFERENCE_MODEL = "gemma4-26b-a4b-qat"


def model_path(component: str) -> Path:
    """The GGUF inside an installed model component."""
    ggufs = list(config.component_dir(component).glob("*.gguf"))
    if len(ggufs) != 1:
        raise FileNotFoundError(f"{component}: expected one .gguf, found {len(ggufs)}")
    return ggufs[0]


# Per thread: the kept-alive connection to each server.
_THREAD = threading.local()


class Chat:
    """An OpenAI-style chat endpoint at `url`."""

    url: str
    # Sent as the request's "model"; llama-server serves one model and ignores it.
    served_model: str | None = None

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        pass

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
        out = json.loads(self._post("/v1/chat/completions", json.dumps(body).encode()))
        return out["choices"][0]["message"]["content"]

    def _post(self, path: str, body: bytes) -> bytes:
        """POSTs over this thread's kept-alive connection. A connection per request runs Windows
        out of ephemeral ports at a few hundred requests a second (WinError 10048)."""
        conns: dict[str, http.client.HTTPConnection] = _THREAD.__dict__.setdefault("conns", {})
        for attempt in range(2):
            conn = conns.get(self.url)
            if conn is None:
                u = urllib.parse.urlsplit(self.url)
                conn = conns[self.url] = http.client.HTTPConnection(u.netloc, timeout=600)
            try:
                conn.request("POST", path, body, {"Content-Type": "application/json"})
                resp = conn.getresponse()
                data = resp.read()
            except (http.client.HTTPException, ConnectionError):
                # The server closed an idle connection: reconnect once.
                conn.close()
                del conns[self.url]
                if attempt:
                    raise
                continue
            if resp.status != 200:
                raise RuntimeError(f"{self.url}{path}: HTTP {resp.status}: {data[:300]!r}")
            return data
        raise AssertionError("unreachable")


class Remote(Chat):
    """A server someone else runs, reached at `url`. Asks it which model it serves."""

    def __init__(self, url: str) -> None:
        self.url = url.rstrip("/")

    def __enter__(self) -> Self:
        with urllib.request.urlopen(f"{self.url}/v1/models", timeout=10) as r:
            self.served_model = json.load(r)["data"][0]["id"]
        return self


class Server(Chat):
    """Utter's llama-server (config.llama_server) on `model`, with `slots` parallel requests."""

    def __init__(self, model: Path, slots: int = 4, context_per_slot: int = 4096) -> None:
        self.model = model
        self.slots = slots
        self.context = context_per_slot * slots
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            self.port = s.getsockname()[1]
        self.url = f"http://127.0.0.1:{self.port}"
        self.proc: subprocess.Popen[bytes] | None = None

    def __enter__(self) -> Self:
        exe, env = config.llama_server()
        log = config.data_dir() / "logs" / "llama-server-tools.log"
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


def connect(url: str = "", model: str = REFERENCE_MODEL, slots: int = 8) -> Chat:
    """`Remote(url)` when a URL is given, else a local `Server` on the installed `model`."""
    return Remote(url) if url else Server(model_path(model), slots=slots)
