//! The speech encoder on LiteRT, for Android's GPU (docs/plan-android.md, "Speech on the
//! phone's GPU"): onnxruntime has no mobile GPU provider, LiteRT runs the encoder there in
//! float16 at about 320 ms a 5 s clip on the test phone.
//!
//! The model comes from `tools/envs/litert/export_litert.py`: one `.tflite` with a signature per
//! fixed input length (`frames_<T>`, sharing the weights), and beside it a `.json` describing the
//! lengths and the subsampling stages. A clip runs in the shortest signature that holds it,
//! padded, with one float mask per stage marking its valid frames.
//!
//! LiteRT is loaded at run time (`libLiteRt.so`, from the app's native libraries, or
//! `UTTER_LITERT_DYLIB`) through its C API, so this compiles on every platform and costs nothing
//! where it is not used. The GPU accelerator (`libLiteRtClGlAccelerator.so`) is found by LiteRT
//! beside it. The compiled GPU programs are cached on disk: loading took 52-107 s without the
//! cache and 8-9 s with it on the phone.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::OnceLock;
use std::time::Instant;

use serde::Deserialize;

use crate::AsrError;

#[cfg(not(windows))]
const LITERT_LIB: &str = "libLiteRt.so";
#[cfg(windows)]
const LITERT_LIB: &str = "LiteRt.dll";

/// Where the encoder runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accelerator {
    /// The GPU through OpenCL or OpenGL, computing in float16.
    Gpu,
    /// LiteRT's CPU kernels (XNNPACK): for the emulator, which has no usable GPU.
    Cpu,
}

/// `<model>.json`, written by the export beside the model.
#[derive(Debug, Clone, Deserialize)]
pub struct EncoderMeta {
    /// The input lengths in mel frames, one signature `frames_<T>` each.
    pub frames: Vec<usize>,
    pub mels: usize,
    pub hidden: usize,
    /// The subsampling's convolutions along time, each followed by a mask.
    pub stages: Vec<Stage>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Stage {
    pub kernel: usize,
    pub stride: usize,
    pub padding: usize,
}

impl EncoderMeta {
    /// The length after each stage, starting from `length` mel frames.
    pub fn stage_lengths(&self, length: usize) -> Vec<usize> {
        let mut n = length;
        self.stages
            .iter()
            .map(|s| {
                n = (n + 2 * s.padding).saturating_sub(s.kernel) / s.stride + 1;
                n
            })
            .collect()
    }
}

type Status = c_int;
type Handle = *mut c_void;
const OK: Status = 0;
const LOCK_READ: c_int = 0;
const LOCK_WRITE: c_int = 1;
const ACCELERATOR_CPU: c_int = 1;
const ACCELERATOR_GPU: c_int = 2;
const PRECISION_FP16: i32 = 1;

/// `LiteRtRankedTensorType`: ABI stable (litert_model_types.h, litert_layout.h).
#[repr(C)]
#[derive(Clone, Copy)]
struct RankedTensorType {
    element_type: c_int,
    /// The bit fields `rank : 7` and `has_strides : 1`.
    rank_bits: u32,
    dimensions: [i32; 8],
    strides: [u32; 8],
}

impl RankedTensorType {
    fn dims(&self) -> &[i32] {
        &self.dimensions[..(self.rank_bits & 0x7f) as usize]
    }
}

macro_rules! api {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// The LiteRT C API functions this module calls (litert/c/*.h, LiteRT 2.3.0).
        #[allow(non_snake_case)]
        struct Api {
            _lib: libloading::Library,
            $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }

        impl Api {
            #[allow(non_snake_case)]
            fn load(path: &Path) -> Result<Self, AsrError> {
                // SAFETY: loading LiteRT runs its initializers, which have no preconditions; the
                // symbols' signatures are those of the C headers of the pinned version.
                unsafe {
                    let lib = libloading::Library::new(path)
                        .map_err(|e| litert_error(format!("loading {}: {e}", path.display())))?;
                    $(let $name = *lib
                        .get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(concat!(stringify!($name), "\0").as_bytes())
                        .map_err(|e| litert_error(format!("{}: {e}", stringify!($name))))?;)*
                    Ok(Self { _lib: lib, $($name,)* })
                }
            }
        }
    };
}

api! {
    LiteRtGetStatusString: fn(Status) -> *const c_char;
    LiteRtCreateEnvironment: fn(c_int, *const c_void, *mut Handle) -> Status;
    LiteRtDestroyEnvironment: fn(Handle);
    LiteRtCreateModelFromFile: fn(Handle, *const c_char, *mut Handle) -> Status;
    LiteRtDestroyModel: fn(Handle);
    LiteRtGetNumModelSignatures: fn(Handle, *mut usize) -> Status;
    LiteRtGetModelSignature: fn(Handle, usize, *mut Handle) -> Status;
    LiteRtGetSignatureKey: fn(Handle, *mut *const c_char) -> Status;
    LiteRtGetNumSignatureInputs: fn(Handle, *mut usize) -> Status;
    LiteRtGetNumSignatureOutputs: fn(Handle, *mut usize) -> Status;
    LiteRtGetSignatureInputName: fn(Handle, usize, *mut *const c_char) -> Status;
    LiteRtGetSignatureInputTensorByIndex: fn(Handle, usize, *mut Handle) -> Status;
    LiteRtGetSignatureOutputTensorByIndex: fn(Handle, usize, *mut Handle) -> Status;
    LiteRtGetRankedTensorType: fn(Handle, *mut RankedTensorType) -> Status;
    LiteRtCreateOptions: fn(*mut Handle) -> Status;
    LiteRtDestroyOptions: fn(Handle);
    LiteRtSetOptionsHardwareAccelerators: fn(Handle, c_int) -> Status;
    LiteRtCreateOpaqueOptions: fn(*const c_char, *mut c_void, Option<unsafe extern "C" fn(*mut c_void)>, *mut Handle) -> Status;
    LiteRtAddOpaqueOptions: fn(Handle, Handle) -> Status;
    LiteRtCreateCompiledModel: fn(Handle, Handle, Handle, *mut Handle) -> Status;
    LiteRtDestroyCompiledModel: fn(Handle);
    LiteRtGetCompiledModelInputBufferRequirements: fn(Handle, usize, usize, *mut Handle) -> Status;
    LiteRtGetCompiledModelOutputBufferRequirements: fn(Handle, usize, usize, *mut Handle) -> Status;
    LiteRtCreateManagedTensorBufferFromRequirements: fn(Handle, *const RankedTensorType, Handle, *mut Handle) -> Status;
    LiteRtDestroyTensorBuffer: fn(Handle);
    LiteRtGetTensorBufferPackedSize: fn(Handle, *mut usize) -> Status;
    LiteRtLockTensorBuffer: fn(Handle, *mut *mut c_void, c_int) -> Status;
    LiteRtUnlockTensorBuffer: fn(Handle) -> Status;
    LiteRtRunCompiledModel: fn(Handle, usize, usize, *mut Handle, usize, *mut Handle) -> Status;
}

static API: OnceLock<Result<Api, String>> = OnceLock::new();
static LIBRARY: OnceLock<PathBuf> = OnceLock::new();

/// Loads LiteRT from `lib`, as `UTTER_LITERT_DYLIB` does: a full path, for the app's native
/// library folder. Call it before the first encoder loads; later calls do nothing.
pub fn use_library(lib: PathBuf) {
    let _ = LIBRARY.set(lib);
}

/// LiteRT, loaded once per process and never unloaded, as onnxruntime is.
fn api() -> Result<&'static Api, AsrError> {
    API.get_or_init(|| {
        let lib = LIBRARY
            .get()
            .cloned()
            .or_else(|| std::env::var_os("UTTER_LITERT_DYLIB").map(PathBuf::from))
            .unwrap_or_else(|| LITERT_LIB.into());
        Api::load(&lib).map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| AsrError::Runtime(e.clone()))
}

fn litert_error(message: String) -> AsrError {
    AsrError::Runtime(format!("LiteRT: {message}"))
}

impl Api {
    fn check(&self, status: Status, what: &str) -> Result<(), AsrError> {
        if status == OK {
            return Ok(());
        }
        // SAFETY: LiteRtGetStatusString returns a static string for any status.
        let text = unsafe { CStr::from_ptr((self.LiteRtGetStatusString)(status)) };
        Err(litert_error(format!("{what}: {}", text.to_string_lossy())))
    }
}

/// One signature: an input length and, once it has run, its buffers.
struct Signature {
    frames: usize,
    index: usize,
    inputs: Vec<Buffer>,
    outputs: Vec<Buffer>,
}

struct Buffer {
    handle: Handle,
    floats: usize,
}

pub struct LiteRtEncoder {
    api: &'static Api,
    env: Handle,
    model: Handle,
    compiled: Handle,
    meta: EncoderMeta,
    /// By input length, shortest first.
    signatures: Vec<Signature>,
}

// SAFETY: the handles are used only through `&mut self` (one thread at a time); LiteRT's
// objects are not tied to the thread that created them.
unsafe impl Send for LiteRtEncoder {}

impl LiteRtEncoder {
    /// Loads `model` (and `<model>.json`) and compiles it for `accelerator`. On the GPU the
    /// compiled programs are kept in `cache` (created if missing) and reused while the model
    /// file is unchanged.
    pub fn load(model: &Path, accelerator: Accelerator, cache: Option<&Path>) -> Result<Self, AsrError> {
        let meta_path = model.with_extension("json");
        let text = std::fs::read_to_string(&meta_path).map_err(|e| AsrError::io(&meta_path, e))?;
        let meta: EncoderMeta =
            serde_json::from_str(&text).map_err(|e| AsrError::Model(format!("{}: {e}", meta_path.display())))?;
        let api = api()?;
        let mut this = Self { api, env: null_mut(), model: null_mut(), compiled: null_mut(), meta, signatures: vec![] };
        let t = Instant::now();
        let path = c_string(model)?;
        // SAFETY: each call gets live handles from the calls before it and out-pointers to
        // fill; a failure leaves the out-pointer unset and `Drop` frees what was made.
        unsafe {
            let api = this.api;
            api.check((api.LiteRtCreateEnvironment)(0, std::ptr::null(), &mut this.env), "creating the environment")?;
            api.check((api.LiteRtCreateModelFromFile)(this.env, path.as_ptr(), &mut this.model), "loading the model")?;
            let mut count = 0;
            api.check((api.LiteRtGetNumModelSignatures)(this.model, &mut count), "counting signatures")?;
            for index in 0..count {
                let mut signature = null_mut();
                api.check((api.LiteRtGetModelSignature)(this.model, index, &mut signature), "reading a signature")?;
                let mut key = std::ptr::null();
                api.check((api.LiteRtGetSignatureKey)(signature, &mut key), "reading a signature's name")?;
                let key = CStr::from_ptr(key).to_string_lossy();
                let frames = key
                    .strip_prefix("frames_")
                    .and_then(|f| f.parse().ok())
                    .ok_or_else(|| AsrError::Model(format!("unexpected signature {key}")))?;
                this.signatures.push(Signature { frames, index, inputs: vec![], outputs: vec![] });
            }
        }
        this.signatures.sort_by_key(|s| s.frames);
        let mut lengths: Vec<usize> = this.signatures.iter().map(|s| s.frames).collect();
        lengths.dedup();
        if lengths != this.meta.frames || lengths.is_empty() {
            return Err(AsrError::Model(format!(
                "signatures {lengths:?}, but {} lists {:?}",
                meta_path.display(),
                this.meta.frames
            )));
        }
        let load_ms = t.elapsed().as_secs_f64() * 1e3;

        let t = Instant::now();
        this.compile(model, accelerator, cache)?;
        let compile_ms = t.elapsed().as_secs_f64() * 1e3;
        tracing::info!(?accelerator, load_ms, compile_ms, frames = ?this.meta.frames, "litert encoder");
        Ok(this)
    }

    fn compile(&mut self, model: &Path, accelerator: Accelerator, cache: Option<&Path>) -> Result<(), AsrError> {
        let api = self.api;
        let mut options = null_mut();
        // SAFETY: as in `load`; the options are the caller's to free after compiling.
        unsafe {
            api.check((api.LiteRtCreateOptions)(&mut options), "creating options")?;
            let result = (|| {
                let flag = match accelerator {
                    Accelerator::Gpu => ACCELERATOR_GPU,
                    Accelerator::Cpu => ACCELERATOR_CPU,
                };
                api.check((api.LiteRtSetOptionsHardwareAccelerators)(options, flag), "choosing the accelerator")?;
                // Each accelerator's options travel as a TOML string under its name
                // (litert/c/options/litert_{gpu,cpu}_options.cc), freed by LiteRT with the callback.
                let (name, toml) = match accelerator {
                    Accelerator::Gpu => (c"gpu_options", gpu_options(model, cache)?),
                    Accelerator::Cpu => (c"xnnpack", cpu_options(cache)?),
                };
                if !toml.is_empty() {
                    let payload = CString::new(toml).map_err(|e| AsrError::Model(e.to_string()))?.into_raw();
                    let mut opaque = null_mut();
                    let made = (api.LiteRtCreateOpaqueOptions)(
                        name.as_ptr(),
                        payload.cast(),
                        Some(free_c_string),
                        &mut opaque,
                    );
                    if made != OK {
                        drop(CString::from_raw(payload));
                    }
                    api.check(made, "creating the accelerator's options")?;
                    api.check((api.LiteRtAddOpaqueOptions)(options, opaque), "adding the accelerator's options")?;
                }
                api.check(
                    (api.LiteRtCreateCompiledModel)(self.env, self.model, options, &mut self.compiled),
                    "compiling",
                )
            })();
            (api.LiteRtDestroyOptions)(options);
            result
        }
    }

    /// The longest clip it takes, in mel frames.
    pub fn max_frames(&self) -> usize {
        self.meta.frames.last().copied().unwrap_or(0)
    }

    pub fn hidden(&self) -> usize {
        self.meta.hidden
    }

    /// Encodes `frames` mel frames (`features`, frame-major, `mels` each) into the projected
    /// encoder frames, `hidden` each, frame-major.
    pub fn encode(&mut self, features: &[f32], frames: usize) -> Result<Vec<f32>, AsrError> {
        let (mels, hidden) = (self.meta.mels, self.meta.hidden);
        if features.len() != frames * mels {
            return Err(AsrError::Model(format!("{} features for {frames} frames of {mels}", features.len())));
        }
        let which = self.signatures.iter().position(|s| s.frames >= frames).ok_or_else(|| {
            let encoder_frames = |n| self.meta.stage_lengths(n).last().copied().unwrap_or(0);
            AsrError::TooLong { frames: encoder_frames(frames), max: encoder_frames(self.max_frames()) }
        })?;
        self.prepare(which)?;
        let padded = self.signatures[which].frames;
        let full = self.meta.stage_lengths(padded);
        let valid = self.meta.stage_lengths(frames);
        let api = self.api;
        let sig = &mut self.signatures[which];
        if sig.inputs.len() != full.len() + 1 {
            return Err(AsrError::Model(format!("{} inputs, expected {}", sig.inputs.len(), full.len() + 1)));
        }
        // SAFETY: each buffer is locked for exactly its packed size in floats, checked when it
        // was made, and unlocked before the run.
        unsafe {
            write(api, &sig.inputs[0], |data| {
                data[..features.len()].copy_from_slice(features);
                data[features.len()..].fill(0.0);
            })?;
            for (k, buffer) in sig.inputs[1..].iter().enumerate() {
                write(api, buffer, |data| {
                    data[..valid[k]].fill(1.0);
                    data[valid[k]..].fill(0.0);
                })?;
            }
            let mut inputs: Vec<Handle> = sig.inputs.iter().map(|b| b.handle).collect();
            let mut outputs: Vec<Handle> = sig.outputs.iter().map(|b| b.handle).collect();
            api.check(
                (api.LiteRtRunCompiledModel)(
                    self.compiled,
                    sig.index,
                    inputs.len(),
                    inputs.as_mut_ptr(),
                    outputs.len(),
                    outputs.as_mut_ptr(),
                ),
                "running the encoder",
            )?;
            let keep = valid.last().copied().unwrap_or(0) * hidden;
            let mut out = vec![0.0; keep];
            let buffer = &sig.outputs[0];
            let mut data = null_mut();
            api.check((api.LiteRtLockTensorBuffer)(buffer.handle, &mut data, LOCK_READ), "reading the output")?;
            out.copy_from_slice(&std::slice::from_raw_parts(data.cast::<f32>(), buffer.floats)[..keep]);
            api.check((api.LiteRtUnlockTensorBuffer)(buffer.handle), "unlocking the output")?;
            Ok(out)
        }
    }

    /// Makes signature `which`'s buffers the first time it runs: the buffers of every length
    /// would hold several hundred MB that short clips never use.
    fn prepare(&mut self, which: usize) -> Result<(), AsrError> {
        if !self.signatures[which].outputs.is_empty() {
            return Ok(());
        }
        let (env, model, compiled) = (self.env, self.model, self.compiled);
        let index = self.signatures[which].index;
        let api = self.api;
        let mut inputs = vec![];
        let mut outputs = vec![];
        // SAFETY: handles from the model and the compiled model, both alive; LiteRT owns the
        // requirements, the buffers are ours (freed in Drop).
        let result: Result<(), AsrError> = unsafe {
            (|| {
                let mut signature = null_mut();
                api.check((api.LiteRtGetModelSignature)(model, index, &mut signature), "reading a signature")?;
                let (mut n_in, mut n_out) = (0, 0);
                api.check((api.LiteRtGetNumSignatureInputs)(signature, &mut n_in), "counting inputs")?;
                api.check((api.LiteRtGetNumSignatureOutputs)(signature, &mut n_out), "counting outputs")?;
                for i in 0..n_in {
                    let mut name = std::ptr::null();
                    api.check((api.LiteRtGetSignatureInputName)(signature, i, &mut name), "reading an input's name")?;
                    tracing::trace!(input = i, name = %CStr::from_ptr(name).to_string_lossy());
                    let mut tensor = null_mut();
                    api.check(
                        (api.LiteRtGetSignatureInputTensorByIndex)(signature, i, &mut tensor),
                        "reading an input",
                    )?;
                    let mut req = null_mut();
                    api.check(
                        (api.LiteRtGetCompiledModelInputBufferRequirements)(compiled, index, i, &mut req),
                        "input requirements",
                    )?;
                    inputs.push(buffer(api, env, tensor, req)?);
                }
                for i in 0..n_out {
                    let mut tensor = null_mut();
                    api.check(
                        (api.LiteRtGetSignatureOutputTensorByIndex)(signature, i, &mut tensor),
                        "reading an output",
                    )?;
                    let mut req = null_mut();
                    api.check(
                        (api.LiteRtGetCompiledModelOutputBufferRequirements)(compiled, index, i, &mut req),
                        "output requirements",
                    )?;
                    outputs.push(buffer(api, env, tensor, req)?);
                }
                Ok(())
            })()
        };
        let sig = &mut self.signatures[which];
        sig.inputs = inputs;
        sig.outputs = outputs;
        result?;
        // The inputs are the features [1, T, mels], then one mask [1, n] per stage.
        let full = self.meta.stage_lengths(sig.frames);
        let expected: Vec<usize> = std::iter::once(sig.frames * self.meta.mels).chain(full.iter().copied()).collect();
        let got: Vec<usize> = sig.inputs.iter().map(|b| b.floats).collect();
        let out = full.last().copied().unwrap_or(0) * self.meta.hidden;
        if got != expected || sig.outputs.len() != 1 || sig.outputs[0].floats != out {
            return Err(AsrError::Model(format!(
                "frames_{}: inputs of {got:?} floats and outputs of {:?}, expected {expected:?} and [{out}]",
                sig.frames,
                sig.outputs.iter().map(|b| b.floats).collect::<Vec<_>>()
            )));
        }
        Ok(())
    }
}

impl Drop for LiteRtEncoder {
    fn drop(&mut self) {
        let api = self.api;
        // SAFETY: every handle was made by LiteRT and is freed once, buffers before the
        // compiled model, the compiled model before the model and the environment.
        unsafe {
            for sig in &self.signatures {
                for b in sig.inputs.iter().chain(&sig.outputs) {
                    (api.LiteRtDestroyTensorBuffer)(b.handle);
                }
            }
            if !self.compiled.is_null() {
                (api.LiteRtDestroyCompiledModel)(self.compiled);
            }
            if !self.model.is_null() {
                (api.LiteRtDestroyModel)(self.model);
            }
            if !self.env.is_null() {
                (api.LiteRtDestroyEnvironment)(self.env);
            }
        }
    }
}

unsafe extern "C" fn free_c_string(payload: *mut c_void) {
    // SAFETY: the payload is the CString given to LiteRtCreateOpaqueOptions.
    drop(unsafe { CString::from_raw(payload.cast()) });
}

/// A float32 buffer for `tensor`, as the compiled model requires.
unsafe fn buffer(api: &Api, env: Handle, tensor: Handle, req: Handle) -> Result<Buffer, AsrError> {
    let mut ty = RankedTensorType { element_type: 0, rank_bits: 0, dimensions: [0; 8], strides: [0; 8] };
    // SAFETY: live handles from the caller; `ty` matches LiteRtRankedTensorType.
    unsafe {
        api.check((api.LiteRtGetRankedTensorType)(tensor, &mut ty), "reading a tensor's type")?;
        if ty.element_type != 1 {
            return Err(AsrError::Model(format!("tensor of element type {}, not float32", ty.element_type)));
        }
        let floats = ty.dims().iter().map(|&d| d.max(0) as usize).product::<usize>();
        let mut handle = null_mut();
        api.check(
            (api.LiteRtCreateManagedTensorBufferFromRequirements)(env, &ty, req, &mut handle),
            "making a buffer",
        )?;
        let mut bytes = 0;
        let sized = (api.LiteRtGetTensorBufferPackedSize)(handle, &mut bytes);
        if sized != OK || bytes != floats * 4 {
            (api.LiteRtDestroyTensorBuffer)(handle);
            api.check(sized, "sizing a buffer")?;
            return Err(AsrError::Model(format!("buffer of {bytes} bytes for {:?}", ty.dims())));
        }
        Ok(Buffer { handle, floats })
    }
}

unsafe fn write(api: &Api, buffer: &Buffer, fill: impl FnOnce(&mut [f32])) -> Result<(), AsrError> {
    let mut data = null_mut();
    // SAFETY: the locked memory holds `floats` floats until unlocked.
    unsafe {
        api.check((api.LiteRtLockTensorBuffer)(buffer.handle, &mut data, LOCK_WRITE), "writing an input")?;
        fill(std::slice::from_raw_parts_mut(data.cast::<f32>(), buffer.floats));
        api.check((api.LiteRtUnlockTensorBuffer)(buffer.handle), "unlocking an input")
    }
}

/// The GPU accelerator's options: float16 math, one copy of the weights for all the signatures,
/// and the compiled programs and converted weights kept in `cache` under a key that changes
/// with the model file.
fn gpu_options(model: &Path, cache: Option<&Path>) -> Result<String, AsrError> {
    let mut toml = format!("precision = {PRECISION_FP16}\nenable_constant_tensors_sharing = true\n");
    if let Some(dir) = cache {
        std::fs::create_dir_all(dir).map_err(|e| AsrError::io(dir, e))?;
        let file = std::fs::metadata(model).map_err(|e| AsrError::io(model, e))?;
        let modified =
            file.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        let key = format!("encoder-{}-{modified}", file.len());
        let dir = dir.to_str().ok_or_else(|| AsrError::Model(format!("{} is not UTF-8", dir.display())))?;
        toml.push_str(&format!(
            "serialization_dir = \"{}\"\nmodel_cache_key = \"{key}\"\nserialize_program_cache = true\nserialize_external_tensors = true\n",
            dir.replace('\\', "/")
        ));
    }
    Ok(toml)
}

/// XNNPACK's options: its float32 copy of the weights in a file in `cache`, mapped into memory
/// and shared by the signatures. Without it each signature unpacked its own, and four of them
/// took the 8 GB emulator down.
fn cpu_options(cache: Option<&Path>) -> Result<String, AsrError> {
    let Some(dir) = cache else { return Ok(String::new()) };
    std::fs::create_dir_all(dir).map_err(|e| AsrError::io(dir, e))?;
    let file = dir.join("xnnpack-weights.bin");
    let file = file.to_str().ok_or_else(|| AsrError::Model(format!("{} is not UTF-8", file.display())))?;
    Ok(format!("weight_cache_file_path = \"{}\"\n", file.replace('\\', "/")))
}

fn c_string(path: &Path) -> Result<CString, AsrError> {
    path.to_str()
        .and_then(|p| CString::new(p).ok())
        .ok_or_else(|| AsrError::Model(format!("{} is not a usable path", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> EncoderMeta {
        // Parakeet's subsampling: three stride-2 convolutions (kernel 3, padding 1), each but the
        // first followed by a pointwise one (kernel 1), every one masked.
        let s2 = Stage { kernel: 3, stride: 2, padding: 1 };
        let pw = Stage { kernel: 1, stride: 1, padding: 0 };
        EncoderMeta { frames: vec![500], mels: 128, hidden: 640, stages: vec![s2, s2, pw, s2, pw] }
    }

    #[test]
    fn stage_lengths_follow_each_convolution() {
        assert_eq!(meta().stage_lengths(500), vec![250, 125, 125, 63, 63]);
        assert_eq!(meta().stage_lengths(301), vec![151, 76, 76, 38, 38]);
        assert_eq!(meta().stage_lengths(1), vec![1, 1, 1, 1, 1]);
    }

    #[test]
    fn ranked_tensor_type_matches_the_c_layout() {
        assert_eq!(std::mem::size_of::<RankedTensorType>(), 72);
    }
}
