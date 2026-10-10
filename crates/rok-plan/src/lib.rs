//! Typed contracts shared by every `cargo rok` surface: the CLI's flags, its
//! `-j`/`-f` JSON input, `cargo rok schema`, the `--output json` result, and
//! the `rok.toml` project config.
//!
//! Each command's arguments are one `serde` + `schemars` type, so the flags,
//! the JSON payload and the published schema can't drift apart.

pub mod apply;
pub mod check;
pub mod hooks;
pub mod input;
pub mod integrate;
pub mod make;
pub mod outcome;
pub mod project;
pub mod recipe;
pub mod schema;

/// Version of the JSON payload/plan/result format. Bumped on breaking
/// changes; older payloads are upgraded on load, newer ones are refused.
pub const FORMAT_VERSION: u32 = 1;
