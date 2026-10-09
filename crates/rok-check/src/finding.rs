use std::fmt;

use serde::{Deserialize, Serialize};

/// One architecture rule. The serialized form is the rule's stable id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    /// A feature crate depends on another outside `[architecture].allow`.
    FeatureEdge,
    /// A dependency cycle between workspace crates.
    Cycle,
    /// A `*_service.rs` file uses `sqlx`.
    ServicePurity,
    /// A `Pg*` adapter is constructed outside the composition root.
    AdapterConstruction,
    /// A handler has no `#[utoipa::path]`.
    HandlerOpenapi,
    /// A handler isn't listed in any `#[module]`.
    HandlerRegistered,
    /// A handler isn't listed in `ApiDoc`'s `paths(...)`.
    HandlerApidoc,
    /// A DTO is missing `Validate`/`ToSchema` derives.
    DtoDerives,
    /// A handler takes unvalidated `Json<T>`.
    ValidatedJson,
    /// A file's main type doesn't match its file name.
    Naming,
    /// A source file or attribute couldn't be parsed.
    Parse,
}

impl Rule {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::FeatureEdge => "feature-edge",
            Self::Cycle => "cycle",
            Self::ServicePurity => "service-purity",
            Self::AdapterConstruction => "adapter-construction",
            Self::HandlerOpenapi => "handler-openapi",
            Self::HandlerRegistered => "handler-registered",
            Self::HandlerApidoc => "handler-apidoc",
            Self::DtoDerives => "dto-derives",
            Self::ValidatedJson => "validated-json",
            Self::Naming => "naming",
            Self::Parse => "parse",
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// One rule violation, located in the project (paths are relative to the
/// project root, with `/` separators; lines and columns are 1-based).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Finding {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub rule: Rule,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{} [{}] {}",
            self.file, self.line, self.column, self.rule, self.message
        )
    }
}
