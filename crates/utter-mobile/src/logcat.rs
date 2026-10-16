//! The core's tracing, written to Android's log (`adb logcat -s utter`), one record per line.

use std::ffi::{CString, c_int};
use std::io::Write;

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::{EnvFilter, fmt};

#[cfg(target_os = "android")]
#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const std::ffi::c_char, text: *const std::ffi::c_char) -> c_int;
}

/// ANDROID_LOG_INFO: the level is in the line already.
const INFO: c_int = 4;

/// Sends tracing to the log; elsewhere (tests, the replay example) tracing goes to stderr.
pub fn init() {
    let filter = EnvFilter::new("warn,utter_mobile=info,utter_asr=info,utter_llm=info,utter_pipeline=info");
    let _ = fmt().with_env_filter(filter).with_ansi(false).without_time().with_writer(Logcat).try_init();
}

struct Logcat;

impl<'a> MakeWriter<'a> for Logcat {
    type Writer = Line;

    fn make_writer(&'a self) -> Line {
        Line(Vec::new())
    }
}

/// One record, written to the log when the formatter is done with it.
struct Line(Vec<u8>);

impl Write for Line {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0);
        let text = text.trim_end();
        if text.is_empty() {
            return;
        }
        let Ok(text) = CString::new(text.replace('\0', " ")) else { return };
        write_log(&text);
    }
}

#[cfg(target_os = "android")]
fn write_log(text: &CString) {
    // SAFETY: both are valid NUL-terminated strings for the call's duration.
    unsafe { __android_log_write(INFO, c"utter".as_ptr(), text.as_ptr()) };
}

#[cfg(not(target_os = "android"))]
fn write_log(text: &CString) {
    let _ = INFO;
    eprintln!("{}", text.to_string_lossy());
}
