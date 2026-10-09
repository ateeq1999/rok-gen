//! `cargo rok inspect` and `cargo rok integrate`: selectors that name a
//! place in the code, and plans of steps that edit there.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which statement of a block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StmtMatcher {
    /// By position (0-based, as `inspect` numbers them), or `"tail"`: the
    /// block's final expression.
    Index(StmtIndex),
    /// The statement makes this call, e.g. `self.posts.delete` or
    /// `PostService::new`; whitespace is ignored.
    Call(String),
    /// The `let` that binds this name.
    Let(String),
    /// The statement's source contains this text.
    Contains(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum StmtIndex {
    Position(usize),
    Named(TailKeyword),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TailKeyword {
    Tail,
}

/// One step into a nested block. Exactly one of `if`, `match`, `for`,
/// `while`, `loop`, `closure`, `block` is set: the n-th such statement
/// (0-based) of the current block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WithinStep {
    #[serde(rename = "if")]
    pub if_: Option<usize>,
    /// For `if`: `then` (default) or `else`.
    pub branch: Option<Branch>,
    #[serde(rename = "match")]
    pub match_: Option<usize>,
    /// For `match`: the arm's pattern, e.g. `Some(_)` (whitespace ignored).
    pub arm: Option<String>,
    #[serde(rename = "for")]
    pub for_: Option<usize>,
    #[serde(rename = "while")]
    pub while_: Option<usize>,
    #[serde(rename = "loop")]
    pub loop_: Option<usize>,
    pub closure: Option<usize>,
    pub block: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Branch {
    #[default]
    Then,
    Else,
}

/// A place in a function body.
///
/// `file` is an alias (`posts/service`, `posts/dto/create_post`, `@root`,
/// `@router`, `@apidoc`) or a path. `fn` is a top-level function (`run`) or
/// a method (`PostService::delete`). Exactly one of `before`, `after`,
/// `stmt`, `start`, `end` says where.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct At {
    pub file: String,
    #[serde(rename = "fn")]
    pub function: String,
    /// Steps into nested blocks, outermost first.
    pub within: Vec<WithinStep>,
    pub before: Option<StmtMatcher>,
    pub after: Option<StmtMatcher>,
    /// The statement itself (for `replace`).
    pub stmt: Option<StmtMatcher>,
    /// The start of the block.
    pub start: bool,
    /// The end of the block, before its tail expression.
    pub end: bool,
    /// 1-based, when the matcher matches several statements.
    pub occurrence: Option<usize>,
    /// Apply at every match.
    pub all: bool,
}

/// One edit of an integration plan. Snippets (`code`, `field`) are atom
/// templates rendered with the plan's `vars`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// Inserts statements at a place.
    Insert { at: At, code: String },
    /// Replaces the statement `at.stmt` names. Needs `--force` on the
    /// command line; a plan can never authorize it itself.
    Replace { at: At, code: String },
    /// Adds a field to a struct.
    AddField {
        file: String,
        #[serde(rename = "struct")]
        struct_name: String,
        /// e.g. `pub comment_count: i64`.
        field: String,
        /// When set, `<field name>: <default>` is also added to every
        /// `Struct { .. }` literal in the crate (e.g. `None`, `0`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
    },
    /// Constructor injection of a repository port into a service: the
    /// field, the `new` parameter and initializer, the composition-root
    /// argument, crate dependencies, the allowlist edge, and the
    /// service's own test call sites (with the dependency's fake).
    InjectDependency {
        /// The service file, e.g. `posts/service`.
        into: String,
        /// The service struct, e.g. `PostService`.
        service: String,
        /// The port, e.g. `comments::CommentRepository`.
        dep: String,
        /// The field and parameter name, e.g. `comments`.
        #[serde(rename = "as")]
        name: String,
    },
}

/// Arguments for `cargo rok integrate`: the plan itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct IntegrateArgs {
    /// Values available to every snippet template.
    pub vars: Value,
    pub steps: Vec<Step>,
    pub dry_run: bool,
    pub verify: bool,
}

/// Arguments for `cargo rok inspect`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct InspectArgs {
    /// A file alias or path, e.g. `posts/service`.
    pub file: String,
    /// Only this function (`run`, `PostService::delete`).
    #[serde(rename = "fn")]
    pub function: Option<String>,
}
