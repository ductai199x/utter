//! Exercises the platform layer on the machine it runs on, for checking a port (a test VM):
//! the GPU, memory and disk; the focused app and the text before the caret; the keyboard for a
//! few seconds; then pastes a line into the focused app.
//!
//!     cargo run -p utter-platform --example probe -- [seconds of keys] [text to paste]

use std::time::{Duration, Instant};

use utter_platform::{caret, disk, focus, gpu, hotkey, insert, memory, process};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(5);
    let text = args.get(1).cloned().unwrap_or_else(|| "Pasted by Utter's probe (1+1=2) é".into());

    println!("gpu: {:?}", gpu::high_performance_adapter());
    println!("vulkan: {}", gpu::vulkan_available());
    println!("memory: total {:?} available {:?}", memory::total(), memory::available());
    println!("disk free in home: {:?}", disk::free_bytes(&std::env::temp_dir()));
    insert::warm();
    caret::warm();
    std::thread::sleep(Duration::from_millis(1500));
    let target = focus::foreground();
    println!("focus: {target:?}");
    println!("before caret: {:?}", caret::before_cursor(20).get(Duration::from_millis(500)));
    println!("open apps: {:?}", focus::open_apps().iter().map(|a| a.exe.as_str()).collect::<Vec<_>>());

    match hotkey::listen(hotkey::HotkeyConfig::default()) {
        Ok(listener) => {
            println!("hotkey (Right Ctrl): listening {seconds} s");
            let until = Instant::now() + Duration::from_secs(seconds);
            while Instant::now() < until {
                if let Some((action, _)) = listener.recv_timeout(Duration::from_millis(100)) {
                    println!("  action: {action:?}");
                }
            }
        }
        Err(e) => println!("hotkey: {e}"),
    }

    // A child tied to this process (it must die with it).
    let mut sleeper = std::process::Command::new("sleep");
    sleeper.arg("600");
    match process::spawn_tied(sleeper) {
        Ok(child) => println!("tied child: pid {}", child.id()),
        Err(e) => println!("tied child: {e}"),
    }

    if let Some(w) = target {
        let rule = insert::default_rules()
            .iter()
            .find(|(exe, _, _)| exe.eq_ignore_ascii_case(&w.app.exe))
            .map(|(_, _, r)| *r)
            .unwrap_or_default();
        println!("paste rule for {:?}: {rule:?}", w.app.exe);
        println!("paste: {:?}", insert::paste(&w, &text, rule));
        std::thread::sleep(Duration::from_millis(1200));
        println!("clipboard after restore: {:?}", insert::clipboard_text());
    }
}
