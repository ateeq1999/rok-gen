//! `cargo rok`: generators, structural feature integration and quality
//! gates for actix-web API workspaces.
//!
//! [`run`] is the whole CLI as a function: arguments, working directory and
//! stdin in; exit code and captured output out. The `cargo-rok` binary is a
//! thin wrapper around it, and it is the seam every behavior is tested
//! through, so in-process tests and real invocations can't diverge.

mod cli;
mod commands;
mod error;
mod generate;
mod output;
mod routes;

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;

use clap::Parser;

pub use error::CliError;

/// Everything passed: gates green, command succeeded.
pub const EXIT_OK: i32 = 0;
/// The command ran but reported failure (a gate failed).
pub const EXIT_FAILED: i32 = 1;
/// The command couldn't run: bad arguments, payload, or project.
pub const EXIT_USAGE: i32 = 2;

/// One finished invocation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Execution {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `cargo rok` with `args` (excluding the program name and cargo's
/// `rok` subcommand word) in `cwd`.
pub fn run<I, S>(args: I, cwd: &Path, stdin: &mut dyn Read) -> Execution
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let command_line =
        std::iter::once(OsString::from("cargo-rok")).chain(args.into_iter().map(Into::into));
    let cli = match cli::Cli::try_parse_from(command_line) {
        Ok(cli) => cli,
        Err(err) => {
            let rendered = err.render().to_string();
            return if err.use_stderr() {
                Execution {
                    code: EXIT_USAGE,
                    stdout: String::new(),
                    stderr: rendered,
                }
            } else {
                // --help / --version.
                Execution {
                    code: EXIT_OK,
                    stdout: rendered,
                    stderr: String::new(),
                }
            };
        }
    };
    commands::execute(cli, cwd, stdin)
}
