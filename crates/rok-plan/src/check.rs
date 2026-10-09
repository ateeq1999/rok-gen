//! `cargo rok check` arguments.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One quality gate. Gates run in declaration order.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Gate {
    /// `cargo fmt --check`.
    Fmt,
    /// `cargo clippy`, failing on whatever the workspace lints `deny`.
    Lint,
    /// `cargo nextest run`, falling back to `cargo test`.
    Test,
    /// `cargo deny check` (licenses, advisories, bans, sources).
    Deny,
    /// `cargo machete` (unused dependencies).
    Machete,
    /// `cargo sqlx prepare --check` against committed offline data.
    Sqlx,
    /// `OpenAPI` compatibility with the base branch. Not implemented yet.
    Openapi,
    /// The architecture checker.
    Arch,
    /// Per-crate coverage floors. Not implemented yet.
    Coverage,
}

impl Gate {
    /// Every gate, in run order.
    pub const ALL: [Self; 9] = [
        Self::Fmt,
        Self::Lint,
        Self::Test,
        Self::Deny,
        Self::Machete,
        Self::Sqlx,
        Self::Openapi,
        Self::Arch,
        Self::Coverage,
    ];

    /// What `cargo rok check` runs when no gate is named: everything that is
    /// implemented.
    pub const DEFAULT: [Self; 7] = [
        Self::Fmt,
        Self::Lint,
        Self::Test,
        Self::Deny,
        Self::Machete,
        Self::Sqlx,
        Self::Arch,
    ];

    /// The fast subset the pre-commit hook runs.
    pub const PRE_COMMIT: [Self; 3] = [Self::Fmt, Self::Lint, Self::Arch];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fmt => "fmt",
            Self::Lint => "lint",
            Self::Test => "test",
            Self::Deny => "deny",
            Self::Machete => "machete",
            Self::Sqlx => "sqlx",
            Self::Openapi => "openapi",
            Self::Arch => "arch",
            Self::Coverage => "coverage",
        }
    }
}

impl fmt::Display for Gate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Arguments for `cargo rok check`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CheckArgs {
    /// Gates to run. Empty means [`Gate::DEFAULT`]. Duplicates are ignored
    /// and gates always run in [`Gate::ALL`] order.
    pub gates: Vec<Gate>,
    /// Keep running the remaining gates after one fails.
    pub keep_going: bool,
    /// Emit GitHub Actions annotations for every finding and failed gate.
    pub ci: bool,
}

impl CheckArgs {
    /// The gates to run, deduplicated and in run order.
    #[must_use]
    pub fn selected(&self) -> Vec<Gate> {
        if self.gates.is_empty() {
            return Gate::DEFAULT.to_vec();
        }
        Gate::ALL
            .into_iter()
            .filter(|gate| self.gates.contains(gate))
            .collect()
    }
}
