//! The `cargo-rok` binary. Cargo runs it as `cargo-rok rok <args>`; it can
//! also be run directly as `cargo-rok <args>`.

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "rok") {
        args.remove(0);
    }

    let execution = match std::env::current_dir() {
        Ok(cwd) => rok_gen::run_with_prompts(
            args,
            &cwd,
            &mut std::io::stdin().lock(),
            &mut std::io::stderr(),
        ),
        Err(err) => rok_gen::Execution {
            code: rok_gen::EXIT_USAGE,
            stdout: String::new(),
            stderr: format!("error: could not read the current directory: {err}\n"),
        },
    };

    // Nothing useful can be done if the terminal itself is gone.
    let _ = std::io::stdout().write_all(execution.stdout.as_bytes());
    let _ = std::io::stderr().write_all(execution.stderr.as_bytes());
    ExitCode::from(u8::try_from(execution.code).unwrap_or(1))
}
