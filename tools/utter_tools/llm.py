"""A language model behind an OpenAI-style chat API: a server someone runs (vLLM on FLAME, for
bulk work) or Utter's own llama-server started locally on a free port.

    with connect(url) as llm:
        llm.chat(messages, json_schema=...)
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
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from utter_tools import common

# The local model the judge and the generator use without a URL
# (installed by utter-cli setup gemma4-26b-a4b-qat).
LOCAL_MODEL = "gemma4-26b-a4b-qat"


class Chat:
    def __init__(self, url: str, model: str | None = None) -> None:
        self.url = url.rstrip("/")
        self.model = model
        # One kept-alive connection per thread: a connection per request runs Windows out of
        # ephemeral ports at a few hundred requests a second.
        self._local = threading.local()

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
        if self.model:
            body["model"] = self.model
        if seed is not None:
            body["seed"] = seed
        if json_schema is not None:
            # vLLM requires the name; llama-server ignores it.
            body["response_format"] = {
                "type": "json_schema",
                "json_schema": {"name": "reply", "schema": json_schema},
            }
        reply = json.loads(self._post("/v1/chat/completions", json.dumps(body).encode()))
        return reply["choices"][0]["message"]["content"]

    def ask(
        self, system: str, user: str, json_schema: dict, seed: int, max_tokens: int = 400
    ) -> dict | None:
        """The model's proposal as data, or None if its reply is not readable JSON."""
        messages = [{"role": "system", "content": system}, {"role": "user", "content": user}]
        reply = self.chat(messages, max_tokens, 0.9, seed, json_schema)
        try:
            return json.loads(reply)
        except json.JSONDecodeError:
            return None

    def _post(self, path: str, body: bytes) -> bytes:
        for attempt in range(2):
            conn = getattr(self._local, "conn", None)
            if conn is None:
                conn = http.client.HTTPConnection(
                    urllib.parse.urlsplit(self.url).netloc, timeout=600
                )
                self._local.conn = conn
            try:
                conn.request("POST", path, body, {"Content-Type": "application/json"})
                resp = conn.getresponse()
                data = resp.read()
            except (http.client.HTTPException, ConnectionError):
                # The server closed an idle connection: reconnect once.
                conn.close()
                self._local.conn = None
                if attempt:
                    raise
                continue
            if resp.status != 200:
                raise RuntimeError(f"{self.url}{path}: HTTP {resp.status}: {data[:300]!r}")
            return data
        raise AssertionError("unreachable")


@contextmanager
def connect(url: str = "", slots: int = 8) -> Iterator[Chat]:
    """A Chat on `url` (asking it which model it serves), or on a local llama-server running
    LOCAL_MODEL with `slots` parallel requests, stopped on exit."""
    if url:
        with urllib.request.urlopen(f"{url.rstrip('/')}/v1/models", timeout=10) as r:
            yield Chat(url, json.load(r)["data"][0]["id"])
        return
    ggufs = list(common.component(LOCAL_MODEL).glob("*.gguf"))
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    exe, env = common.llama_server()
    log = common.data_dir() / "logs" / "llama-server-tools.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    args = [exe, "--model", str(ggufs[0]), "--port", str(port), "--parallel", str(slots)]
    args += ["--ctx-size", str(4096 * slots), "--n-gpu-layers", "999", "--flash-attn", "on"]
    args += ["--host", "127.0.0.1", "--no-webui", "--jinja"]
    with log.open("wb") as out:
        proc = subprocess.Popen(args, stdout=out, stderr=subprocess.STDOUT, env=env)
    try:
        _wait_healthy(f"http://127.0.0.1:{port}", proc, log)
        yield Chat(f"http://127.0.0.1:{port}")
    finally:
        proc.kill()
        proc.wait()


def _wait_healthy(url: str, proc: subprocess.Popen, log: Path) -> None:
    deadline = time.monotonic() + 300
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise RuntimeError(f"llama-server exited ({proc.returncode}); see {log}")
        try:
            with urllib.request.urlopen(f"{url}/health", timeout=2) as r:
                if r.status == 200:
                    return
        except OSError:
            pass
        time.sleep(0.5)
    raise TimeoutError(f"llama-server did not become ready; see {log}")
