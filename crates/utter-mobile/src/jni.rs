//! The app's calls into the core: the static methods of `dev.utter.app.Native` (Kotlin's
//! `@JvmStatic` functions of `object Native`). A loaded core is a handle (a pointer, as a Long)
//! that the app frees once. Errors reach the app as RuntimeExceptions; nothing unwinds into the
//! JVM.
//!
//! ```kotlin
//! object Native {
//!     @JvmStatic external fun load(configJson: String): Long
//!     @JvmStatic external fun touch(handle: Long, down: Boolean, timeMs: Int)
//!     @JvmStatic external fun cancel(handle: Long)
//!     @JvmStatic external fun pushAudio(handle: Long, pcm: ShortArray, length: Int)
//!     @JvmStatic external fun nextEvent(handle: Long, timeoutMs: Int): String?
//!     @JvmStatic external fun free(handle: Long)
//! }
//! ```

use std::time::Duration;

use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JClass, JShortArray, JString};
use jni::sys::{jboolean, jint, jlong};
use jni::{Env, EnvUnowned};

use crate::{Config, Mobile};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("{0}")]
    Jni(#[from] jni::errors::Error),
    #[error("config: {0}")]
    Config(#[from] serde_json::Error),
    #[error("{0}")]
    Core(#[from] crate::MobileError),
}

/// The core behind a handle from [`load`].
///
/// # Safety
/// `handle` came from `load` and has not been freed: the app keeps it until `free`.
unsafe fn core<'a>(handle: jlong) -> &'a Mobile {
    // SAFETY: as the caller guarantees.
    unsafe { &*(handle as *const Mobile) }
}

/// Loads the core from the JSON of a [`Config`]; returns its handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_utter_app_Native_load<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    config: JString<'caller>,
) -> jlong {
    env.with_env(|_env: &mut Env| -> Result<jlong, Error> {
        let cfg: Config = serde_json::from_str(&config.to_string())?;
        let mobile = Mobile::load(&cfg)?;
        Ok(Box::into_raw(Box::new(mobile)) as jlong)
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_utter_app_Native_touch<'caller>(
    _env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    handle: jlong,
    down: jboolean,
    time_ms: jint,
) {
    // SAFETY: the app passes the handle `load` gave it, before `free`.
    unsafe { core(handle) }.touch(down, time_ms as u32);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_utter_app_Native_cancel<'caller>(
    _env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    handle: jlong,
) {
    // SAFETY: as for touch.
    unsafe { core(handle) }.cancel();
}

/// The first `length` samples of `pcm`: 16 kHz mono, 16-bit, as AudioRecord reads it.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_utter_app_Native_pushAudio<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    handle: jlong,
    pcm: JShortArray<'caller>,
    length: jint,
) {
    env.with_env(|env: &mut Env| -> Result<(), Error> {
        let mut samples = vec![0i16; length.max(0) as usize];
        pcm.get_region(env, 0, &mut samples)?;
        let samples = samples.iter().map(|&s| f32::from(s) / 32_768.0).collect();
        // SAFETY: as for touch.
        unsafe { core(handle) }.push_audio(samples);
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// The next event as JSON (see [`crate::Event`]), or null after `timeout_ms` without one.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_utter_app_Native_nextEvent<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    handle: jlong,
    timeout_ms: jint,
) -> JString<'caller> {
    env.with_env(|env: &mut Env<'caller>| -> Result<JString<'caller>, Error> {
        // SAFETY: as for touch.
        let event = unsafe { core(handle) }.next_event(Duration::from_millis(timeout_ms.max(0) as u64));
        match event {
            Some(e) => Ok(JString::from_str(env, serde_json::to_string(&e)?)?),
            None => Ok(JString::default()),
        }
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Frees the core: its worker ends, and with it the models and the cleanup server.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_utter_app_Native_free<'caller>(
    _env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: the handle came from `load`, and the app frees it once.
        drop(unsafe { Box::from_raw(handle as *mut Mobile) });
    }
}
