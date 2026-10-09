//! Quality gates (`cargo rok check`) and the architecture checker for
//! rok-managed actix-web workspaces.
//!
//! The architecture rules turn the reference project's conventions, which
//! were previously enforced only by README and review, into findings:
//! feature crates only depend on each other through the `rok.toml`
//! allowlist, services never touch `sqlx`, only the composition root
//! constructs `Pg*` adapters, and every handler is documented, registered
//! and listed in `ApiDoc`.

pub mod arch;
mod finding;
pub mod gates;
mod workspace;

pub use finding::{Finding, Rule};
pub use gates::{GateResult, GateStatus, Report};
