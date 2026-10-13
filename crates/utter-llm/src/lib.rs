//! Utter's language model: llama.cpp's `llama-server`, run as Utter's own child process from
//! the pinned `llama-cpp` runtime component, never from another installed application.
//!
//! [`LlamaServer::start`] launches it on a free local port with the CUDA libraries of the
//! `cuda-runtime` component, waits until `/health` answers, and logs its output to the rotating
//! `<data_dir>/logs/llama-server.log` (utter_core::logs). Dropping the handle stops it. [`LlamaServer::chat`]
//! sends one chat completion and returns the text with llama-server's own timings.
//! [`cleanup`] builds the cleanup request.

pub mod cleanup;

use std::io::{BufRead, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// llama-server's log, shared by every server this process runs (two overlap while a model
/// switch loads the new one beside the old).
const SERVER_LOG_NAME: &str = "llama-server.log";
static SERVER_LOG: LazyLock<Mutex<utter_core::logs::RotatingFile>> =
    LazyLock::new(|| Mutex::new(utter_core::logs::RotatingFile::new(SERVER_LOG_NAME)));

fn server_log(line: &str) {
    if let Ok(mut log) = SERVER_LOG.lock() {
        let _ = log.write(line.as_bytes());
        let _ = log.flush();
    }
}

/// Copies one of the server's output streams into its log, a line at a time, until it closes.
fn copy_to_log(stream: impl std::io::Read + Send + 'static) {
    let spawned = std::thread::Builder::new().name("utter-llama-log".into()).spawn(move || {
        let mut reader = std::io::BufReader::new(stream);
        let mut line = Vec::new();
        while reader.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
            server_log(&String::from_utf8_lossy(&line));
            line.clear();
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("llama-server's output will not be logged: {e}");
    }
}

use serde::{Deserialize, Serialize};

#[cfg(windows)]
const SERVER_EXE: &str = "llama-server.exe";
#[cfg(not(windows))]
const SERVER_EXE: &str = "llama-server";

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("{0}")]
    Runtime(#[from] utter_runtime::RuntimeError),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("llama-server did not become ready within {0:?}; see {1}")]
    StartTimeout(Duration, PathBuf),
    #[error("llama-server exited during start ({0}); see {1}")]
    Exited(String, PathBuf),
    #[error("llama-server request: {0}")]
    Request(String),
}

#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub model: PathBuf,
    /// Context window in tokens.
    pub context: u32,
    /// Layers offloaded to the GPU (999: all).
    pub gpu_layers: u32,
    pub flash_attention: bool,
    /// How long to wait for the model to load.
    pub start_timeout: Duration,
    /// More arguments, passed through as they are.
    pub extra_args: Vec<String>,
}

impl ServerOptions {
    pub fn new(model: impl Into<PathBuf>) -> Self {
        Self {
            model: model.into(),
            context: 4096,
            gpu_layers: 999,
            flash_attention: true,
            start_timeout: Duration::from_secs(180),
            extra_args: Vec::new(),
        }
    }
}

pub struct LlamaServer {
    child: Child,
    base_url: String,
    log: PathBuf,
    agent: ureq::Agent,
    /// What it was started with, for a restart.
    opts: ServerOptions,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }
}

#[derive(Debug, Clone)]
pub struct ChatParams {
    pub max_tokens: u32,
    pub temperature: f32,
    /// Let a reasoning model think before answering. Off for cleanup and commands: thinking
    /// costs latency, and a model that thinks first can spend the whole budget on it.
    pub thinking: bool,
    /// Extra fields merged into the request body (llama-server specific options).
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for ChatParams {
    fn default() -> Self {
        Self { max_tokens: 512, temperature: 0.0, thinking: false, extra: serde_json::Map::new() }
    }
}

/// llama-server's own measurements of one request.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct ServerTimings {
    #[serde(default)]
    pub prompt_n: u32,
    #[serde(default)]
    pub prompt_ms: f64,
    #[serde(default)]
    pub predicted_n: u32,
    #[serde(default)]
    pub predicted_ms: f64,
    #[serde(default)]
    pub cache_n: u32,
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub server: ServerTimings,
    /// Wall time of the HTTP request, as the caller saw it.
    pub wall: Duration,
}

impl LlamaServer {
    pub fn start(opts: &ServerOptions) -> Result<Self, LlmError> {
        // Development only: a llama-server built elsewhere (the eval on Linux, where Utter has no
        // runtime component yet), with the system's CUDA. The app never sets it.
        let (exe, path) = match std::env::var_os("UTTER_LLAMA_SERVER") {
            Some(exe) => (PathBuf::from(exe), None),
            None => {
                let llama = utter_runtime::component("llama-cpp")?.installed_dir()?;
                let cuda = utter_runtime::component("cuda-runtime")?.installed_dir()?;
                // The CUDA runtime comes from Utter's component, for this child only.
                (llama.join(SERVER_EXE), Some(prepend_path(&[llama, cuda])))
            }
        };
        if !opts.model.is_file() {
            return Err(LlmError::Io {
                path: opts.model.clone(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "model file not found"),
            });
        }
        let port = free_port()?;
        let log = utter_core::logs::dir().join(SERVER_LOG_NAME);

        let mut cmd = Command::new(&exe);
        cmd.arg("--model").arg(&opts.model);
        cmd.args(["--host", "127.0.0.1", "--port", &port.to_string()]);
        cmd.args(["--ctx-size", &opts.context.to_string(), "--n-gpu-layers", &opts.gpu_layers.to_string()]);
        // --no-warmup: the caller's first request warms it (cleanup sends a real one), so the
        // server's own empty pass only delays ready.
        cmd.args(["--parallel", "1", "--no-webui", "--jinja", "--no-warmup"]);
        cmd.args(["--flash-attn", if opts.flash_attention { "on" } else { "off" }]);
        cmd.args(&opts.extra_args);
        if let Some(path) = path {
            cmd.env("PATH", path);
        }
        // Its output goes to the rotating log (utter_core::logs), or nowhere when logs are off.
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        utter_platform::process::no_console_window(&mut cmd);
        let mut child = cmd.spawn().map_err(|source| LlmError::Io { path: exe.clone(), source })?;
        server_log(&format!("--- llama-server started, model {}\n", opts.model.display()));
        if let Some(out) = child.stdout.take() {
            copy_to_log(out);
        }
        if let Some(err) = child.stderr.take() {
            copy_to_log(err);
        }
        // If Utter dies without running Drop, the OS ends the server, so it never keeps
        // holding the GPU's memory. Drop still stops it in the normal case.
        if let Err(e) = utter_platform::process::tie_to_this_process(&child) {
            tracing::warn!("{e}: llama-server may outlive a crash of Utter");
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut server = Self { child, base_url: format!("http://127.0.0.1:{port}"), log, agent, opts: opts.clone() };
        server.wait_ready(opts.start_timeout)?;
        tracing::info!(model = %opts.model.display(), url = %server.base_url, "llama-server ready");
        Ok(server)
    }

    fn wait_ready(&mut self, timeout: Duration) -> Result<(), LlmError> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Some(status) = self.child.try_wait().map_err(|e| LlmError::Request(e.to_string()))? {
                return Err(LlmError::Exited(status.to_string(), self.log.clone()));
            }
            if let Ok(resp) = self.agent.get(&format!("{}/health", self.base_url)).call()
                && resp.status() == 200
            {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(LlmError::StartTimeout(timeout, self.log.clone()))
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// How its process ended, if it has (a crash, a GPU driver reset): `None` while it runs.
    pub fn exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.to_string()),
            Ok(None) => None,
            Err(e) => Some(e.to_string()),
        }
    }

    /// Starts a new server with the same options in this one's place (the old process is ended
    /// first, so the two never hold the GPU's memory at once).
    pub fn restart(&mut self) -> Result<(), LlmError> {
        let _ = self.child.kill();
        let _ = self.child.wait();
        *self = Self::start(&self.opts)?;
        Ok(())
    }

    pub fn log_path(&self) -> &Path {
        &self.log
    }

    /// One chat completion, not streamed. A reply cut off at its length limit, or with no text,
    /// is an error: not the whole answer.
    pub fn chat(&self, messages: &[Message], params: &ChatParams) -> Result<Completion, LlmError> {
        let (reply, wall) = self.send(messages, params)?;
        let choice = &reply["choices"][0];
        if choice["finish_reason"].as_str() == Some("length") {
            return Err(LlmError::Request("the reply was cut off at its length limit".into()));
        }
        let Some(text) = choice["message"]["content"].as_str() else {
            return Err(LlmError::Request(format!("no text in the reply: {reply}")));
        };
        Ok(Completion { text: text.to_owned(), server: timings(&reply), wall })
    }

    /// Sends one completion request; returns the reply's JSON and the request's wall time.
    fn send(&self, messages: &[Message], params: &ChatParams) -> Result<(serde_json::Value, Duration), LlmError> {
        let mut body = serde_json::json!({
            "messages": messages,
            "max_tokens": params.max_tokens,
            "temperature": params.temperature,
            "stream": false,
            "cache_prompt": true,
            // Read by the chat templates of models that can reason (Qwen and others);
            // ignored by templates that cannot.
            "chat_template_kwargs": { "enable_thinking": params.thinking },
        });
        if let serde_json::Value::Object(map) = &mut body {
            map.extend(params.extra.clone());
        }
        let start = Instant::now();
        let resp = self
            .agent
            .post(&format!("{}/v1/chat/completions", self.base_url))
            .send_json(&body)
            .map_err(|e| LlmError::Request(e.to_string()))?;
        let status = resp.status();
        let text = resp.into_body().read_to_string().map_err(|e| LlmError::Request(e.to_string()))?;
        let wall = start.elapsed();
        if status != 200 {
            return Err(LlmError::Request(format!("HTTP {status}: {text}")));
        }
        let reply = serde_json::from_str(&text).map_err(|e| LlmError::Request(e.to_string()))?;
        Ok((reply, wall))
    }
}

/// llama-server's timings in a reply (zero when it gave none).
fn timings(reply: &serde_json::Value) -> ServerTimings {
    serde_json::from_value(reply["timings"].clone()).unwrap_or_default()
}

/// The bench's (utter-cli llm-bench): the pipeline uses neither.
impl LlamaServer {
    /// Processes `messages` into the prompt cache without generating anything: a later prompt
    /// that starts with the same tokens reuses them (llm-bench --prefill-fraction).
    pub fn prefill(&self, messages: &[Message]) -> Result<Completion, LlmError> {
        self.complete_without_text(messages, 0)
    }

    /// One token, to keep the GPU and the server's threads from going idle: a request after
    /// a few idle seconds takes more than twice as long (docs/eval-results.md). `messages`
    /// should share the real request's system prompt, so the prompt cache keeps it.
    pub fn keep_warm(&self, messages: &[Message]) -> Result<Completion, LlmError> {
        self.complete_without_text(messages, 1)
    }

    /// A request for at most `max_tokens`, whose text does not matter.
    fn complete_without_text(&self, messages: &[Message], max_tokens: u32) -> Result<Completion, LlmError> {
        let (reply, wall) = self.send(messages, &ChatParams { max_tokens, ..ChatParams::default() })?;
        Ok(Completion { text: String::new(), server: timings(&reply), wall })
    }
}

impl Drop for LlamaServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A port nothing listens on right now. Another process could take it before llama-server
/// binds it; start then fails with a clear error and can be retried.
fn free_port() -> Result<u16, LlmError> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|source| LlmError::Io { path: "127.0.0.1:0".into(), source })?;
    let port = listener.local_addr().map_err(|source| LlmError::Io { path: "127.0.0.1:0".into(), source })?.port();
    Ok(port)
}

fn prepend_path(dirs: &[PathBuf]) -> std::ffi::OsString {
    let mut paths: Vec<PathBuf> = dirs.to_vec();
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).unwrap_or_default()
}
