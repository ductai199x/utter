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

/// What the arguments ask for: `true` to start the app; `false` once they are answered.
pub fn handle() -> bool {
    match parse(std::env::args().skip(1)) {
        Ok(Action::Run) => true,
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
    Print(String),
}

fn parse(args: impl Iterator<Item = String>) -> Result<Action, String> {
    let args: Vec<String> = args.collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | [AUTOSTART_ARG] => Ok(Action::Run),
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
    fn refuses_anything_else() {
        assert!(run(&["--verison"]).is_err());
        assert!(run(&["--version", "--help"]).is_err());
        assert!(run(&["--autostart", "x"]).is_err());
    }
}
