# rok-gen

`cargo rok`: generators, structural feature integration and quality gates for actix-web API
workspaces built on the [actix-api-starter](https://github.com/ateeq1999/actix-api-starter)
conventions (one crate per feature, `#[module]` routing, utoipa docs, one composition root).

The full design is in the reference project's spec, `docs/specs/rok-cli-ultra.md`.

## Status

| Phase | Scope | State |
|---|---|---|
| 0 | Repo, `rok.toml` project contract, atom-engine pinned with every workaround proven | done |
| 1 | `cargo rok check` (quality gates + architecture checker), `hooks install` | done (`openapi` and `coverage` gates pending) |
| 2 | `make:feature`, `make:action`, `make:dto`, `make:migration`, `make:entity`, `make:repository`, `make:service`, `make:error`, `routes` | done (`make:test`/`make:factory` deferred) |
| 3–7 | `inspect`/`integrate`, journal/undo, recipes, MCP, upgrade | planned |

## Generators

Every generator renders its files from the embedded atom-engine templates
(`crates/rok-gen/templates/`), wires them into the shared files by parsing them (never with
marker comments), runs rustfmt on every Rust file, and then writes everything at once, or
nothing if any step fails.

```sh
cargo rok make:feature posts --field title:string --field body:text --field summary:string? \
    --guard auth            # none | auth (default) | admin
cargo rok make:action posts publish_post --method post --path "/{id}/publish" \
    --body PublishPostRequest --guard admin
cargo rok make:dto posts PostSummaryResponse --kind response --field title:string
cargo rok make:migration add_post_slug --sql "ALTER TABLE posts ADD COLUMN slug TEXT;"

# Building blocks inside an existing feature:
cargo rok make:error audits                                  # AuditsError in the error crate
cargo rok make:entity posts tag_label --field label:string   # TagLabel, NewTagLabel, TagLabelUpdate
cargo rok make:repository posts revision --field body:text   # entity + port + Pg adapter + fake + table
cargo rok make:service posts publishing --repository post --repository revision
cargo rok routes                                             # the route table, read from source
```

`make:repository` writes the in-memory fake into the repository file, under
`#[cfg(test)] pub mod fake`, together with a round-trip test, so every port always ships with a
test double. It requires the feature's error type, which `make:error` creates. `make:service`
builds the service in the composition root from fresh `Pg*` adapters and registers it as app
data.

Every generator also takes `--dry-run` (print the unified diff, write nothing), `--verify` (run
`cargo check --workspace --all-targets` after writing and roll everything back if it fails) and
the usual `-j`/`-f` JSON input.

Field types: `string`, `text`, `uuid`, `int`, `bigint`, `bool`, `float`, `datetime`, `date`,
`json`, or a Rust type with a Postgres equivalent; a trailing `?` makes the field optional.

`make:feature posts` creates the `crates/posts` crate (entity, create/update DTOs, repository
trait with its `PgPostRepository` adapter, an in-memory fake, a service with unit tests, five
CRUD handlers, the `#[module]`), a `PostsError` in the `error` crate, and the next numbered
migration. It then wires the crate in:

| File | Change |
|---|---|
| `Cargo.toml` | `posts = { path = "crates/posts" }` in the internal block of `[workspace.dependencies]`, and the root crate's dependency |
| router and `ApiDoc` crates' `Cargo.toml` | `posts.workspace = true` |
| `crates/error/src/lib.rs` | `mod posts;` and `pub use posts::PostsError;` |
| router `#[module(imports = [...])]` | `posts::PostModule` |
| `ApiDoc` `paths(...)`, `schemas(...)`, `tags(...)` | the five handlers, the entity, both DTOs, their envelopes, a `Posts` tag |
| composition root `run` | the adapter and service, built after the existing services, and `.app_data(web::Data::new(post_service.clone()))` |

New list entries follow each list's own layout. New imports go into the right group
(external or `crate::`). Inserted dependency lines keep the manifest's comments, grouping and
sort order.

`make:action` adds a documented `501 Not Implemented` stub with the right extractors (path
params typed `Uuid` for `id`/`*_id`, the body DTO, `AdminUser` for `--guard admin`), its
imports and crate dependencies, its `#[module]` registration and its `ApiDoc` path. It generates
the body DTO if it doesn't exist yet.

The generated code passes the reference project's fmt, clippy and architecture gates with no
warnings from the generated files, and its tests pass. The end-to-end test checks this against
the real project.

`[project]` in `rok.toml` says where the shared files are:

```toml
[project]
composition_root = "src"                  # lib.rs (or main.rs) inside it
composition_fn = "run"
router = "crates/router/src/lib.rs"
apidoc = "crates/router/src/openapi.rs"
errors = "crates/error/src/lib.rs"
migrations = "crates/db/migrations"
```

All of these default to the reference project's layout.

## Install

```sh
cargo install --path crates/rok-gen   # from a checkout, until it's published
```

## A rok-managed project

A workspace is rok-managed when its root has a `rok.toml`:

```toml
[rok]
cli = "^0.1"          # rok-gen versions allowed to touch this project
templates = "0.1.0"   # template set the project was generated with

[project]
composition_root = "src"                      # the only place adapters may be constructed
apidoc = "crates/router/src/openapi.rs"       # optional; found automatically otherwise

[architecture]
allow = ["auth -> users"]   # sanctioned feature-to-feature dependencies
# features = ["auth", "users"]   # optional; inferred from `src/*_module.rs` otherwise
```

`cargo rok schema project` prints its JSON Schema.

## `cargo rok check`

```sh
cargo rok check                 # every implemented gate, stopping at the first failure
cargo rok check fmt lint arch   # just these, always in canonical order
cargo rok check --keep-going    # run everything, report every failure
cargo rok check --ci            # plus GitHub Actions annotations
```

| Gate | Runs | When the tool is missing |
|---|---|---|
| `fmt` | `cargo fmt --all -- --check` | fails |
| `lint` | `cargo clippy --workspace --all-targets` (the project's own lint levels decide what fails) | fails |
| `test` | `cargo nextest run`, else `cargo test` | — |
| `deny` | `cargo deny check` | skipped, with install hint |
| `machete` | `cargo machete` | skipped, with install hint |
| `sqlx` | `cargo sqlx prepare --workspace --check` | skipped (also when there's no `.sqlx/`) |
| `arch` | the architecture checker, in-process | — |
| `openapi`, `coverage` | not implemented yet | always skipped |

### Architecture rules

| Rule | Flags |
|---|---|
| `feature-edge` | a feature crate depending on another feature crate outside `[architecture].allow` |
| `cycle` | a dependency cycle between workspace crates |
| `service-purity` | `sqlx` used in a `*_service.rs` |
| `adapter-construction` | a `Pg*` adapter (declared in a `*_repository.rs`) constructed outside the composition root |
| `handler-openapi` | a route handler without `#[utoipa::path]` |
| `handler-registered` | a handler not listed in any `#[module(controllers/protected = [...])]` |
| `handler-apidoc` | a handler missing from `ApiDoc`'s `paths(...)` |
| `dto-derives` | a `dto/` type missing `ToSchema` (and `Validate`, for request structs) |
| `validated-json` | a handler taking `Json<T>` instead of `ValidatedJson<T>` |
| `naming` | `x_service.rs`/`x_module.rs` without `struct XService`/`XModule`, `x_repository.rs` without `trait XRepository` |
| `parse` | a file or attribute that can't be parsed |

Anything under `#[cfg(test)]` is ignored. Dev-dependencies aren't architecture.

## `cargo rok hooks install`

Writes `.git/hooks/pre-commit` running `cargo rok check fmt lint arch` (or the gates you name).
It never replaces a hook it didn't write unless you pass `--overwrite`.

## Input and output

Every payload-taking command accepts its arguments as JSON, layered as
**defaults < `-f` file < `-j` inline < flags**:

```sh
cargo rok check -j '{"gates": ["fmt", "arch"], "keep_going": true}'
cargo rok check -f check.yaml          # .json/.jsonc, .yaml/.yml or .toml
echo '{"gates":["arch"]}' | cargo rok check -f -
cargo rok check -f check.json --print-payload   # show the merged payload, don't run
cargo rok schema check                 # JSON Schema of the payload
```

Errors name the field by JSON pointer (`invalid payload at /gaets: unknown field ... (did you
mean `gates`?)`). `--output json` prints one result object on stdout for every command:

```json
{ "version": 1, "command": "check", "ok": true, "files_created": [], "files_changed": [],
  "warnings": [], "data": { "gates": [ ... ] } }
```

Exit codes: `0` success, `1` a gate failed, `2` the command couldn't run.

## Workspace

| Crate | Responsibility |
|---|---|
| `rok-gen` | the `cargo-rok` binary; `rok_gen::run` is the whole CLI as a function |
| `rok-plan` | argument types, layered JSON input, `rok.toml`, JSON Schemas |
| `rok-check` | quality gates and the architecture checker |
| `rok-render` | atom-engine rendering with rok's filters and the workarounds below |
| `rok-locate` | parses Rust with `syn` and turns structural anchors (`#[module]` lists, `#[openapi]` lists, statements, `.app_data` chains, imports) into byte offsets |
| `rok-edit` | all-or-nothing transactions: rustfmt (always), CRLF preservation, unified diffs, staged writes with retries, rollback, `cargo check` verification |

### atom-engine workarounds (`rok-render`)

atom-engine is pinned to `=5.0.2`. Its gaps for generating Rust are worked around in
`rok-render`, never patched upstream, and each has a test in
`crates/rok-render/tests/atom_workarounds.rs`:

- **HTML auto-escaping**: Tera auto-escapes `.html`/`.htm`/`.xml` template names with no
  off switch, so every template is registered under an internal `.atom` name.
- **Stacks don't work**: `push`/`stack` never round-trip, so imports use rok's own
  `import(path=...)`/`imports()`, resolved after rendering (deduplicated, grouped std → external
  → crate).
- **Brace collisions**: `{{ names | braces }}` → `{A, B}` instead of unwritable `{{{`.
- **Untrusted packs**: `env()`/`file_exists()` are disabled for `Trust::Pack`.
- **Lost render errors**: atom-engine drops Tera's cause chain; failures are re-rendered on a
  plain Tera to report the real cause.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
ROK_REFERENCE_PROJECT=../Project-Starters/actix-api-starter cargo test --workspace
ROK_REFERENCE_PROJECT=../Project-Starters/actix-api-starter ROK_E2E=1 cargo test --workspace
```

Every behavior is tested through `rok_gen::run` against fixture workspaces in temp
directories (see `crates/rok-gen/tests/common`). Setting `ROK_REFERENCE_PROJECT` also checks
that the real reference project passes the architecture checker untouched. Adding `ROK_E2E=1`
also generates a feature and an action into a copy of it with `--verify`, then runs its
fmt/lint/arch gates and the generated tests (about a minute and a half on a cold target dir).

## License

MIT
