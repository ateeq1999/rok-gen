# rok-gen

`cargo rok`: generators, structural feature integration and quality gates for actix-web API
workspaces built on the [actix-api-starter](https://github.com/ateeq1999/actix-api-starter)
conventions (one crate per feature, `#[module]` routing, utoipa docs, one composition root).

The full design is in [`docs/specs/rok-cli-ultra.md`](docs/specs/rok-cli-ultra.md).

## Status

| Phase | Scope | State |
|---|---|---|
| 0 | Repo, `rok.toml` project contract, atom-engine pinned with every workaround proven | done |
| 1 | `cargo rok check` (quality gates + architecture checker), `hooks install` | done (`openapi` and `coverage` gates pending) |
| 2 | `make:feature`, `make:action`, `make:dto`, `make:migration`, `make:entity`, `make:repository`, `make:service`, `make:error`, `routes` | done (`make:test`/`make:factory` deferred) |
| 3 | `inspect`, selectors (incl. `within`), `integrate` (insert, replace + `--force`, add_field, inject_dependency, add_method, migration, make, nest_routes), scope check | done |
| 4 | journal (token fingerprints, no markers), `undo`, `unlink`, `journal`; git dirty check for `--force`; `add_method` and `migration` steps; presets (`has-many`, `hook`) with `--emit-plan`; `inject_port` (the `port` strategy); `integrate --wizard` | done (the `attachment` preset is deferred until storage is a recipe) |
| 5 | `apply` (batches, rolled back whole on a failed step or gate), `new`, `adopt`, recipes (`add`, `remove`, `list`) | in progress (`doctor` next) |
| 6–7 | MCP, skills, upgrade | planned |

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

## Inspecting and integrating

`cargo rok inspect` lists every place a plan can target:

```text
$ cargo rok inspect posts/service --fn PostService::delete
crates/posts/src/post_service.rs
fn PostService::delete  L72
    [0] L73  if self.posts.delete(id).await? {  <tail>  within {"if":0}
```

Files are named by alias: `<feature>/<role>` (`service`, `action`, `module`, `repository`,
`entity`), `<feature>/lib`, `<feature>/dto/<name>`, `<feature>/<file stem>`, `@root`, `@router`,
`@apidoc`, `@errors`, or a path. `--output json` gives the same tree for tools and agents.

`cargo rok integrate -f plan.yaml` applies a plan's steps, in order, as one transaction. This
example wires `comments` into `posts`:

```yaml
vars: { dep: comments }          # rendered into every string of every step
steps:
  - op: inject_dependency
    into: posts/service
    service: PostService
    dep: comments::CommentRepository
    as: "{{ dep }}"
  - op: insert
    at:
      file: posts/service
      fn: PostService::delete
      within: [{ if: 0 }]        # also: { if: 0, branch: else }, { match: 0, arm: "Some(_)" },
      start: true                #       { for | while | loop | closure | block: n }
    code: "self.{{ dep }}.delete_by_post(id).await?;"
  - op: add_field
    file: posts/dto/create_post
    struct: CreatePostRequest
    field: "#[serde(default)] pub tags: Option<String>"
    default: "None"              # also added to every `CreatePostRequest { .. }` literal
```

| Step | What it does |
|---|---|
| `insert` | adds statements at a place: `before`/`after` a matched statement, or at the `start`/`end` of the block |
| `replace` | replaces the statement `at.stmt` names. It needs `--force` on the command line; a plan can't authorize it |
| `add_field` | adds a field to a struct and, with `default`, to every literal of that struct in the crate |
| `inject_dependency` | adds the field, the `new` parameter, the initializer and the imports; passes the dependency's `Pg*` adapter in the composition root and its `Fake*` in the service's tests; adds the crate dependency and the `fake`-feature dev-dependency; adds the `[architecture].allow` edge. It refuses an edge that would create a dependency cycle |
| `add_method` | adds a method (`signature`, `doc`) to a trait and a body to every implementor in its crate (`bodies`: implementor → body). A missing body is refused, because the lints forbid `todo!()` stubs |
| `migration` | adds the next numbered SQL migration (`name`, `sql`) in the same transaction |
| `make` | runs any generator (`command: "make:dto"`, `args`: the command's own payload) in the same transaction; later steps can target the files it creates |
| `nest_routes` | moves a module under another's prefix: `module: comments/module`, `under: posts/module`, `param: post_id` turns `/comments` into `/posts/{post_id}/comments`, in the module prefix and in every documented path |
| `inject_port` | the `port` strategy, for when `inject_dependency` would create a cycle (the source already depends on the target). The target crate gets a new trait (`port`, `methods`, plus a fake behind its `fake` feature), and its service gets the field, the `new` parameter and the fake in its tests. The source crate implements the trait for its adapter (`implementor: comments::PgCommentRepository`, using each method's `body`). The composition root passes the adapter. The target never depends on the source |

### The wizard

`cargo rok integrate --wizard` builds a plan by asking numbered questions instead of reading
JSON. It can inject a repository into a service, insert code at a statement you pick from a
numbered list, add a field, or apply a preset, and it repeats for as many steps as you want.
Then it applies the plan, shows the diff, or saves the plan to a file. Its plans are the same as
hand-written ones, so they go through the same checks, journal and `undo`. Answers are read line
by line from stdin, so a wizard session can also be scripted.

### Presets

A preset is a plan template:

```sh
cargo rok integrate --preset has-many --var source=comments --var target=posts
cargo rok integrate --preset has-many --var source=comments --var target=posts \
    --emit-plan plans/comments-posts.json        # review or edit it, then `integrate -f`
```

`has-many` needs the source entity to have a required `<target>_id: Uuid` field (override the
name with `--var fk=...`). It does four things:

1. Adds `delete_by_<target>` to the source repository: SQL in the Postgres adapter, `retain` in
   the fake.
2. Injects the source port into the target service.
3. Makes the target's `delete` cascade to the source first.
4. Indexes the foreign key.

`hook` (`--var source=audits --var target=posts --var on=create|update|delete`) adds an
`on_<target>_<op>` method to the source repository and calls it from the target service before
the operation writes. The method starts as a no-op in both the Postgres adapter and the fake,
for you to fill in.

`cargo rok unlink <source> <target>` reverts all of it.

Statements are matched by `index` (or `"tail"`), `call` (`self.posts.delete`; whitespace is
ignored), `let` (the bound name) or `contains`. A matcher that hits several statements is an
error that lists them, unless you add `occurrence` (1-based) or `all: true`. Each snippet must
parse as Rust statements, or as a field, before anything is written. Each snippet may only read
variables that are in scope where it lands: the function's parameters, earlier `let`s, and
enclosing closure, loop, match-arm or `if let` bindings. Anything else is refused, with the
names that are in scope listed. The check is name-based, so type errors are left to `--verify`. `--dry-run` prints the diff,
and `--verify` runs `cargo check` and rolls everything back if it fails.

Every feature crate's repository fakes sit behind a `fake` Cargo feature
(`#[cfg(any(test, feature = "fake"))]`), so other crates' tests can use them.

`replace` with `--force` also refuses files with uncommitted changes, so git can always bring
them back. `--allow-dirty` overrides that.

## Batches: `cargo rok apply`

`apply` runs a list of commands as one batch that lands whole or not at all. Each step takes
the payload `-j` takes for that command (see `cargo rok schema <command>`):

```jsonc
// setup.json
{
  "name": "blog",
  "steps": [
    { "command": "make:feature", "args": { "name": "articles", "fields": [{ "name": "title", "type": "string" }] } },
    { "command": "make:feature", "args": { "name": "comments", "fields": [
        { "name": "body", "type": "text" }, { "name": "article_id", "type": "uuid" } ] } },
    { "command": "integrate", "args": { "preset": "has-many", "vars": { "source": "comments", "target": "articles" } } },
    { "command": "check", "args": { "gates": ["arch", "fmt", "lint"] } }
  ]
}
```

```sh
cargo rok apply -f setup.json --dry-run    # one diff for the whole batch
cargo rok apply -f setup.json --verify     # cargo check after writing; roll back on failure
```

- Every `make:*` and `integrate` step edits one transaction, so a later step sees what earlier
  steps created. Above, the preset finds the `comments` entity the same batch generated.
- A step that fails names itself (`step 3 (integrate): ...`), and nothing is written.
- `check` steps run once the batch is written. If any gate fails, every file is restored and
  the command exits 1. A dry run skips them.
- `dry_run` and `verify` belong to the batch, not to its steps. `--force` and `--allow-dirty`
  are command-line only, as for `integrate`.
- The batch is one journal transaction, labeled with its `name`, so one `cargo rok undo` reverts
  all of it.

## Recipes: `cargo rok add`, `remove`, `list`

A recipe is an optional capability with a choice of drivers. Each one is a workspace crate,
`crates/<recipe>`, with a port trait, the chosen driver's adapter and an in-memory fake behind
the `fake` feature.

| Recipe | Drivers (first is the default) | Requires |
|---|---|---|
| `auth` | `session` (actix-session cookies), `jwt` (jsonwebtoken) | `mail` |
| `mail` | `resend`, `smtp` (lettre), `log` | |
| `storage` | `s3` (aws-sdk-s3, with a Terraform module), `local` | |

```sh
cargo rok add mail --driver resend
cargo rok add auth --driver jwt --driver mail=smtp   # installs mail (with smtp) first
cargo rok add storage --driver s3 --terraform        # also infra/terraform/modules/storage/
cargo rok list                                       # available and installed recipes, with versions
cargo rok remove storage
cargo rok new shop --recipe auth --driver auth=jwt   # recipes at project creation
```

- `add` installs the recipes it requires that aren't installed yet, all in one transaction
  (one `cargo rok undo` reverts it).
- It declares the crate and the third-party crates the driver needs in
  `[workspace.dependencies]`, and records the recipe as `[recipes.<name>]` in `rok.toml`.
- It appends the driver's env keys to `.env.example`. It never reads or writes `.env`.
- `remove` refuses while an installed recipe requires the recipe, or while a crate depends on
  its crate, and names them.
- `remove` also refuses when the recipe's files were edited (formatting aside) or files were
  added to its crate. `--force` removes them anyway; it's command-line only.
- `remove` deletes the crate, the Terraform module, the env keys no other recipe uses and the
  `rok.toml` entry. Third-party entries in `[workspace.dependencies]` stay, since other crates
  may use them.

## Undo, unlink and the journal

Every committed generator, `integrate` or `apply` run is recorded in `.rok/journal.json`; commit it
alongside the code. Your code never gets marker comments.

```sh
cargo rok journal                    # every transaction: id, command, label, active/undone
cargo rok undo                       # revert the latest active transaction
cargo rok undo 1791556766-make-feature --dry-run
cargo rok unlink comments posts      # revert every integration labeled "comments into posts"
```

The journal stores each change as runs of Rust tokens (or lines, for other files), each with a
few tokens of context. Created files are stored as a content hash. Reverting looks for each run
in the current file, ignoring formatting. So a revert still works after rustfmt, after code is
added around the change, after the function moves, and when transactions are undone out of
order.

Created files are deleted only if they're unchanged. If a run can't be found, appears more than
once, or a created file was edited, the revert reports it as drift and changes nothing.

`--verify` runs `cargo check`, which may update `Cargo.lock`. The lockfile isn't part of the
transaction, so `undo` leaves it alone; the next build prunes stale entries.

An integration's label is its plan's `name`, or `<dep> into <target>` taken from its first
`inject_dependency` step.

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

[recipes.mail]        # maintained by `cargo rok add` / `cargo rok remove`
version = "0.1.0"
driver = "resend"
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

## Releasing

CI (`.github/workflows/ci.yml`) also packages and verifies every crate on each push, the same
way crates.io builds them. To release:

```sh
# 1. bump `version` in [workspace.package] in Cargo.toml (all crates share it), commit
git tag v0.2.0
git push origin v0.2.0
```

The tag triggers `.github/workflows/release.yml`. It re-runs fmt, clippy and tests on Linux and
Windows, then checks that the tag matches the workspace version. It then publishes the crates to
crates.io in dependency order with the `CARGO_REGISTRY_TOKEN` repository secret, and creates a
GitHub release with generated notes. Versions already on crates.io are skipped, so a release
that failed partway can be re-run safely. Running the workflow by hand defaults to a dry run.

## License

MIT
