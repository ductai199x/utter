//! The command line: Utter takes no arguments from people but `--version` and `--help`, and
//! `--autostart` from its sign-in entry (splash.rs). Anything else is refused, so a mistyped flag
//! never starts the app.

use crate::splash::AUTOSTART_ARG;

const USAGE: &str = "Usage: utter [--version | --help]

Starts Utter: hold the dictation key (Right Ctrl unless changed in Settings), speak, and the
text appears where you type.

Options:
  -V, --version  Print the version and exit
  -h, --help     Print this help and exit";

/// Utter restarting itself (restart_from): the copy it starts waits for this pid to end first.
pub const RESTARTED_FROM_ARG: &str = "--restarted-from";

/// How long a restarted copy waits for the one that started it to end.
const RESTART_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// What the arguments ask for: `true` to start the app; `false` once they are answered.
pub fn handle() -> bool {
    match parse(std::env::args().skip(1)) {
        Ok(Action::Run) => true,
        Ok(Action::RunAfter(pid)) => {
            // The copy that restarted Utter still holds the one-instance lock until it ends:
            // started before that, this copy would hand over to it and quit, and nothing would
            // run (Windows, 2026-10-19: Settings' Restart now left no Utter).
            if !utter_platform::process::wait_for_exit(pid, RESTART_WAIT) {
                eprintln!("utter: the previous copy ({pid}) did not end in time");
            }
            true
        }
        Ok(Action::Print(text)) => {
            utter_platform::console::attach_parent();
            println!("{text}");
            false
        }
        Err(text) => {
            utter_platform::console::attach_parent();
            eprintln!("{text}");
            std::process::exit(2);
        }
    }
}

#[derive(Debug, PartialEq)]
enum Action {
    Run,
    /// Run once this process has ended (Utter restarting itself).
    RunAfter(u32),
    Print(String),
}

fn parse(args: impl Iterator<Item = String>) -> Result<Action, String> {
    let args: Vec<String> = args.collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | [AUTOSTART_ARG] => Ok(Action::Run),
        [RESTARTED_FROM_ARG, pid] => {
            pid.parse().map(Action::RunAfter).map_err(|_| format!("utter: not a process id: {pid}"))
        }
        ["-V" | "--version"] => Ok(Action::Print(format!("Utter {}", env!("CARGO_PKG_VERSION")))),
        ["-h" | "--help"] => Ok(Action::Print(USAGE.to_owned())),
        _ => Err(format!("utter: unknown arguments: {}\n\n{USAGE}", args.join(" "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Action, String> {
        parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn starts_with_no_arguments_or_from_sign_in() {
        assert_eq!(run(&[]), Ok(Action::Run));
        assert_eq!(run(&["--autostart"]), Ok(Action::Run));
    }

    #[test]
    fn answers_version_and_help() {
        assert_eq!(run(&["--version"]), Ok(Action::Print(format!("Utter {}", env!("CARGO_PKG_VERSION")))));
        assert_eq!(run(&["-V"]), run(&["--version"]));
        assert!(matches!(run(&["-h"]), Ok(Action::Print(t)) if t.starts_with("Usage:")));
    }

    #[test]
    fn a_restart_names_the_copy_to_wait_for() {
        assert_eq!(run(&["--restarted-from", "4242"]), Ok(Action::RunAfter(4242)));
        assert!(run(&["--restarted-from", "x"]).is_err());
        assert!(run(&["--restarted-from"]).is_err());
    }

    #[test]
    fn refuses_anything_else() {
        assert!(run(&["--verison"]).is_err());
        assert!(run(&["--version", "--help"]).is_err());
        assert!(run(&["--autostart", "x"]).is_err());
    }
}
