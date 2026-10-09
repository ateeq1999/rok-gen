# Spec: rok-cli-ultra — `rok-gen`, the `cargo rok` generator and integration CLI

Status: ready-for-agent
Reference project: actix-api-starter

## Problem Statement

I maintain actix-api-starter, a NestJS-shaped actix-web template: one Cargo crate per feature, and the
same file roles in every feature crate (module, action, service, repository, entity, dto), a
`#[module]` attribute for route registration, utoipa for API docs, and explicit wiring of every adapter
in a single composition root. Those conventions are the template's whole value, but today they live only
in a ~550-line README and in people's heads.

Adding one feature means hand-editing roughly eight files across four crates: the new feature crate, the
workspace manifest, the router's `AppModule`, the aggregated `ApiDoc`, and the composition root. Each of
those edits is easy to get subtly wrong: a handler not listed in `ApiDoc` silently disappears from the
spec, a feature crate quietly starts depending on another, a service reaches for sqlx directly. The
architecture rules ("services depend only on repository traits", "no feature crate reaches into another",
"only the composition root constructs adapters") are enforced by convention, not by any tool.

Connecting two features is worse. Wiring `comments` into `posts` means adding a constructor dependency,
threading it through the composition root in the right order, inserting calls at exactly the right
statement inside existing service functions, nesting routes, extending DTOs, and adding a migration. Then
it means checking that none of this created a dependency cycle. There is no repeatable, reviewable,
reversible way to do this, and no way for an AI agent to do it safely without free-form text edits.

Finally, the quality bar (strict workspace lints, no unwrap/panic, formatting, supply-chain hygiene) is
only partly automated, and nothing checks the template's architectural rules at all.

## Solution

A separate, published Rust package, **`rok-gen`**, installed with `cargo install rok-gen` and invoked as
**`cargo rok …`**. It reads a project's conventions and turns them into commands:

- **Generators** (`cargo rok make:*`, `cargo rok new`) produce feature crates, actions, DTOs, services,
  repositories (the trait, the Postgres adapter, and an in-memory fake), entities, migrations, errors,
  tests and factories. Their output already passes the project's lints, formatting and architecture
  rules, and every shared file (manifest, router, ApiDoc, composition root) is wired automatically.
- **Recipes** (`cargo rok add|remove <recipe>`) add or remove optional capabilities such as auth, mail
  and storage, with driver choices, and track what depends on what.
- **Structural integration** (`cargo rok inspect`, `cargo rok integrate`) wires one feature into another
  at locations the user picks: which file role, which function, and before, after or in place of which
  statement, including inside nested blocks. Integrations are planned as multi-step JSON plans and
  applied as all-or-nothing transactions. They can be previewed as diffs, verified with `cargo check`,
  undone, and unlinked later. No marker comments are ever written into user code.
- **Quality gates** (`cargo rok check`) run formatting, lints, tests, supply-chain checks, sqlx offline
  data, OpenAPI compatibility, and a custom architecture checker. They also install a pre-commit hook and
  generate CI.
- **One input contract for people, scripts and agents.** Every command accepts its full argument set as
  JSON: inline, from a file, or from stdin. Every command publishes a JSON Schema and can report its
  result as JSON. The same engine is exposed as an MCP server and backs project skills, so an AI agent
  makes structural, rule-checked edits instead of free-form text edits.

Code templates are rendered with **atom-engine**. Where atom-engine is missing something (escaping
control, brace collisions, whitespace handling), the gap is worked around inside the rendering crate
rather than fixed upstream. All generated and edited Rust is always passed through rustfmt.

## User Stories

### Installing and project setup

1. As an API developer, I want to install the tool with `cargo install rok-gen`, so that I don't need any non-Rust tooling.
2. As an API developer, I want to run it as `cargo rok`, so that it feels like a native cargo subcommand.
3. As an API developer, I want `cargo rok new <name>` to create a complete, compiling project from the reference template, so that I start from a working baseline.
4. As an API developer, I want to choose recipes and drivers at `new` time (e.g. auth, mail via Resend, storage via S3), so that my project contains only what I use.
5. As an API developer, I want a new project to come with its git repo initialized, CI workflow, pre-commit hook, supply-chain config and project skills already set up, so that quality checks run from the first commit.
6. As an API developer, I want each project to record which tool version and template version it was made with, so that an incompatible CLI refuses to modify it instead of corrupting it.
7. As an API developer with an existing actix project, I want `cargo rok adopt` to infer file roles, write the project config, and list the files that don't conform, so that I can bring an older project under the tool gradually.
8. As an API developer, I want a clear error when I run a command outside a rok-managed project, so that I don't scaffold into the wrong directory.

### Generators

9. As an API developer, I want `cargo rok make:feature posts` to create a full feature crate with module, action, service, repository, entity, DTO and migration, so that I never hand-write boilerplate.
10. As an API developer, I want to pass entity fields to `make:feature`, so that the entity, DTOs, migration and repository SQL are generated with real columns.
11. As an API developer, I want the tool to derive every name from the feature name (singular types in a plural crate, snake-case files, Pascal-case types), so that naming is always consistent.
12. As an API developer, I want `make:action` to generate a handler with its OpenAPI annotation, auth or admin guard, request body and response envelope, and register it in its module and in ApiDoc, so that a new endpoint is documented and reachable.
13. As an API developer, I want `make:dto` to produce a validated, schema-documented DTO, so that request validation and docs are never forgotten.
14. As an API developer, I want `make:repository` to always produce the trait, the Postgres adapter and an in-memory fake together, so that every service is unit-testable without a database.
15. As an API developer, I want `make:service` to generate a service that depends only on repository traits, so that the architecture rules hold by default.
16. As an API developer, I want `make:migration` to create the next correctly numbered migration file, so that migration ordering never collides.
17. As an API developer, I want `make:error` to add a feature error type to the shared error crate with HTTP mappings, so that error handling stays centralized.
18. As an API developer, I want `make:test` to generate integration tests per action and fake-based unit tests per service, so that new code starts with coverage.
19. As an API developer, I want `make:factory` to generate test-data factories per entity, so that tests and seeds share one source of fixtures.
20. As an API developer, I want every generator to wire the workspace manifest, router, ApiDoc and composition root automatically, so that I don't have to remember the shared files.
21. As an API developer, I want generated code to pass the project's strict clippy, rustfmt and architecture rules unchanged, so that generating code never breaks the build or CI.
22. As an API developer, I want `cargo rok routes` to print the route table from the module declarations, so that I can see the API surface without starting the server.

### JSON input and output

23. As an API developer, I want every command to accept its arguments as inline JSON (`-j`), so that I can script the tool.
24. As an API developer, I want every command to accept a JSON file (`-f`), or stdin (`-f -`), so that I can keep reusable setups in files and pipe them in.
25. As a Windows developer, I want file input in JSONC, YAML or TOML, so that I avoid PowerShell quoting problems with inline JSON.
26. As an API developer, I want explicit flags to override file and inline JSON values (defaults < file < inline < flags), so that I can reuse a file and tweak one field.
27. As an API developer, I want `--print-payload` to show the merged input, so that I can debug which source won.
28. As an API developer, I want `cargo rok schema <cmd>` to print a JSON Schema, so that my editor autocompletes and validates payloads.
29. As an API developer, I want validation errors to name the exact field by JSON pointer, with "did you mean" hints, so that I can fix a payload quickly.
30. As a script author, I want `--output json` on every command, listing created and changed files, line ranges and warnings, so that other tools can consume the results.
31. As an API developer, I want every plan file to carry a format version, and old formats to be upgraded on load, so that my saved plans keep working across CLI releases.

### Inspecting code and choosing locations

32. As an API developer, I want to target files by feature and role (e.g. `posts/service`) rather than path, so that plans stay readable and survive file moves the tool knows about.
33. As an API developer, I want special targets for the composition root, the router and ApiDoc, so that I can address shared wiring points directly.
34. As an API developer with a non-conforming file, I want to fall back to an explicit path, so that the tool still works on files that break the naming convention.
35. As an API developer, I want `cargo rok inspect posts/service` to list structs, fields, functions and each function's numbered statements with line numbers and previews, so that I can pick an insertion point.
36. As an AI agent, I want `inspect --output json`, so that I can choose locations programmatically.
37. As an API developer, I want to match a statement by index, by the method call it makes, by the variable a `let` binds, or by a text substring, so that I can choose the most stable matcher.
38. As an API developer, I want call matching to ignore formatting, so that my plan doesn't break when code is reformatted.
39. As an API developer, I want a selector that matches more than one statement to fail with every candidate listed, unless I specify an occurrence or "all", so that the tool never guesses.
40. As an API developer, I want to address statements inside `if`/`else` branches, `match` arms, loops, closures and plain blocks, so that I can insert code exactly where the logic needs it.
41. As an API developer, I want to target a function's tail expression directly, so that "just before returning" is easy to express.
42. As an API developer, I want to add struct fields, constructor parameters, impl or trait methods, and `#[module]` list entries as positions, so that every structural extension point is covered.
43. As an API developer, I want `use` imports to be managed automatically (deduplicated and sorted), so that I never write imports by hand in a plan.
44. As an API developer, I want selectors that can disambiguate same-named functions under different `cfg` attributes, so that conditional code isn't matched twice.

### Integrating features

45. As an API developer, I want to write a multi-step integration plan (e.g. comments into posts), so that a whole cross-feature change is described once and can be reviewed.
46. As an API developer, I want an `inject_dependency` step that adds the field, the constructor parameter and the composition-root wiring in the right order, so that dependency injection is a single step.
47. As an API developer, I want an `insert` step that places code before, after or at the start/end of a chosen statement or function, so that I control exactly where behavior hooks in.
48. As an API developer, I want `add_field` and `nest_routes` steps, so that DTO extensions and nested resources (e.g. comments under a post's routes) are first-class.
49. As an API developer, I want plans to include generator steps such as a migration, so that schema changes are part of the same transaction.
50. As an API developer, I want snippets to be templates with the source and target features' names available, so that one plan works across many feature pairs.
51. As an API developer, I want the tool to check, before writing, that the identifiers a snippet uses are in scope at the insertion point, so that obvious mistakes are caught early.
52. As an API developer, I want every snippet checked for the right syntax kind (statement, item, field, expression) before it's inserted, so that broken code is never written.
53. As an API developer, I want a crate-level dependency cycle detected before any write, so that an integration never leaves the workspace unbuildable.
54. As an API developer, I want a `port` strategy that inverts the dependency through a trait the target owns and the source implements, so that I can integrate features that would otherwise form a cycle.
55. As an API developer, I want adding a method to a repository trait to require a body for every implementor (Postgres and fake), so that I never end up with stubs the lints forbid.
56. As an API developer, I want the dependency allowlist updated automatically when an integration adds a crate edge, so that the architecture check accepts sanctioned edges and rejects unsanctioned ones.
57. As an API developer, I want presets (`has-many`, `hook`, `attachment`) that generate whole plans, so that common integrations take one command.
58. As an API developer, I want `--emit-plan` to save the plan a preset or the wizard produced, so that I can edit it and apply it later.
59. As an API developer, I want an interactive wizard that walks through what, where, which function and which statement, with previews, so that I can integrate features without writing JSON.
60. As an API developer, I want the wizard, JSON plans, presets and agents to all produce the same plan format, so that every path runs through the same validated engine.
61. As an API developer, I want `cargo rok apply -f setup.json` to run a list of commands (new, features, integrations, check) as one batch, so that a whole project is reproducible from one file.

### Replace and --force

62. As an API developer, I want a `replace` step for a statement, a function body or a whole item, so that I can change existing behavior and not only add to it.
63. As an API developer, I want `replace` to need `--force` on the command line, and never be authorized from inside a plan file, so that a downloaded plan can't overwrite my code silently.
64. As an API developer, I want replaced code shown separately and in red in the dry-run diff, so that destructive changes stand out in review.
65. As an API developer, I want the original code of every replace recorded, so that undo can restore it.
66. As an API developer, I want a warning when a replace overlaps code a previous integration inserted, so that I know that integration's unlink will stop working.
67. As an API developer, I want `--force` to refuse to touch files with uncommitted changes unless I pass `--allow-dirty`, so that I can always recover through git.

### Transactions, journal, undo

68. As an API developer, I want every integration to be all-or-nothing, with every selector resolved before any write, so that a failure never leaves a half-applied change.
69. As an API developer, I want `--dry-run` to print the unified diff without writing, so that I can review before applying.
70. As an API developer, I want `--verify` to run one `cargo check` on the affected crates and roll back on failure, so that applied integrations always compile.
71. As an API developer, I want `--emit-patch` to write a patch file instead of changing files, so that I can review it in a PR or apply it later with git.
72. As an API developer, I want `--commit` and `--branch`, so that each transaction can land as its own commit on its own branch.
73. As an API developer, I want no marker comments in my code, with inserted code found again by token fingerprints recorded in a journal, so that my source stays clean.
74. As an API developer, I want inserted code to be found again after reformatting, line shifts and moves within the file, so that unlink and undo survive normal maintenance.
75. As an API developer, I want `cargo rok undo` to revert the last transaction (or a named one), so that experiments are cheap.
76. As an API developer, I want `cargo rok unlink comments posts` to remove everything an integration added, so that integrations are reversible.
77. As an API developer, I want inserted code that was edited afterwards ("drifted") to be reported and left untouched, so that the tool never deletes my changes.
78. As an API developer, I want an interrupted transaction to be detectable and repairable from the journal, so that a crash or a locked file on Windows can be recovered from.
79. As a team member, I want the journal to be committed and to merge cleanly in normal cases, so that integrations are shared through git.

### Formatting and file handling

80. As an API developer, I want every file the tool writes or edits to be formatted with rustfmt using my project's config, so that output is always canonical.
81. As an API developer, I want formatting-only changes in files that weren't clean before to be reported separately from semantic changes in the diff, so that review stays focused.
82. As a Windows developer, I want each file's line endings (CRLF or LF) preserved, so that diffs aren't flooded with line-ending changes.
83. As a Windows developer, I want file writes retried with backoff when an editor, rust-analyzer or antivirus has a file locked, so that transactions don't fail spuriously.
84. As an API developer, I want my comments and formatting outside the edited region kept, because the tool edits the original text and doesn't re-print the syntax tree.

### Recipes

85. As an API developer, I want `cargo rok add mail --driver resend`, so that I can add a capability after the project was created.
86. As an API developer, I want recipes to declare what they require, and `add` to install those first, so that I never get a half-working capability.
87. As an API developer, I want `remove` to refuse while other features depend on the recipe, and name them, so that removal never breaks the build.
88. As an API developer, I want recipes to add their env keys to the example env file and never to the real one, so that secrets are never written.
89. As an API developer, I want recipes to optionally bring their Terraform modules, so that the infrastructure for a capability ships with it.
90. As an API developer, I want `cargo rok list` to show installed and available recipes and their versions, so that I know my project's composition.

### Quality gates

91. As an API developer, I want `cargo rok check` to run formatting, clippy with the workspace lints, tests, supply-chain checks, unused-dependency detection, sqlx offline data and the architecture rules in one command, so that I have one definition of "green".
92. As an API developer, I want the architecture checker to flag a feature crate depending on another outside the allowlist, so that boundaries stay real.
93. As an API developer, I want it to flag services that import sqlx or construct Postgres adapters, so that services stay testable.
94. As an API developer, I want it to flag adapters constructed outside the composition root, so that wiring stays in one place.
95. As an API developer, I want it to flag handlers missing an OpenAPI annotation, a module registration or an ApiDoc entry, so that no endpoint is ever undocumented or unreachable.
96. As an API developer, I want it to flag DTOs missing validation or schema derives, and handlers taking unvalidated JSON, so that input validation is never forgotten.
97. As an API developer, I want it to flag file and type names that break the naming conventions, so that the codebase stays navigable.
98. As an API developer, I want it to flag any dependency cycle between feature crates, so that cycles are caught early.
99. As an API developer, I want it to flag a mismatch between the allowlist and the edges the journal recorded, so that hand-made cross-feature dependencies are surfaced.
100. As a reviewer, I want `check openapi` to fail on breaking API changes compared with the base branch, so that clients aren't broken by accident.
101. As a CI pipeline, I want `check --ci` to emit GitHub annotations, so that failures show inline on the PR.
102. As a CI pipeline, I want `check --changed` to run only on crates affected since the base branch, so that CI stays fast as the workspace grows.
103. As an API developer, I want `cargo rok hooks install` to set up a fast pre-commit hook (format, lint, architecture), so that problems are caught before commit.
104. As an API developer, I want optional coverage floors per crate, so that teams can enforce test coverage where they choose to.

### Templates

105. As an API developer, I want `cargo rok templates eject <name>` to copy a built-in template into my project as an override, so that I can customize output without forking the tool.
106. As an API developer, I want `cargo rok templates diff` to compare my overrides with the current built-in templates, so that I can pick up upstream improvements.
107. As a template author, I want naming filters (snake, pascal, camel, kebab, singular, plural, rust_type, sql_type, rust_ident), so that templates never hand-roll name transforms.
108. As a template author, I want templates to push `use` lines to a shared import stack that's rendered once, so that imports are collected and deduplicated automatically.
109. As a template author, I want reusable components for repeated blocks (OpenAPI annotations, handler signatures, fakes, constructor params), so that changing a convention means changing one template.
110. As a template author, I want each template to declare its props, and a test to check every command's schema against its templates' props, so that payload/template mismatches are caught in CI and not at render time.
111. As a template author, I want Rust brace collisions with template delimiters handled by a helper, so that templates don't break on nested braces.
112. As a template author, I want HTML auto-escaping guaranteed off for Rust, SQL and TOML templates, so that generated code is never HTML-escaped.
113. As a security-conscious user, I want environment and filesystem functions disabled for third-party template packs, so that a pack can't copy secrets into generated code.

### Project maintenance

114. As an API developer, I want `cargo rok doctor` to report drifted snippets, non-conforming files, unrecorded cross-feature edges, env-example drift and pending migrations, so that I know how healthy the project is.
115. As an API developer, I want `cargo rok upgrade` to apply newer template and recipe versions with a 3-way merge, so that my project keeps receiving improvements after it was created.
116. As an API developer, I want `cargo rok rename posts articles` to rename the crate, files, types, routes and dependent integrations, and add a table-rename migration, so that renaming a feature is one safe step.
117. As an API developer, I want project-wide changes driven by glob selectors (e.g. add tracing instrumentation to every service method), so that cross-cutting changes are one transaction.
118. As an API developer, I want `cargo rok graph` to draw features, recipes and integration edges as Mermaid or DOT, highlighting cycles and edges not on the allowlist, so that I can see the architecture.
119. As an API developer, I want `cargo rok env sync` to keep the example env file in sync with the keys the config crate reads, so that onboarding never misses a variable.
120. As an API developer, I want `cargo rok db up|migrate|reset|seed`, so that the local database lifecycle is one tool.
121. As an API developer, I want lifecycle hooks in the project config (e.g. prepare sqlx data after a migration), so that project-specific follow-ups run automatically.
122. As an API developer, I want parameterized plans that can be reused across entities (e.g. soft-delete for any entity), so that my team's integration patterns become reusable.

### AI agents and editor integration

123. As an AI agent, I want `cargo rok mcp` to expose inspect, plan, dry-run, apply and check as MCP tools with their JSON Schemas, so that I can make structural, rule-checked edits instead of free-form text edits.
124. As an AI agent, I want project skills (add-feature, add-endpoint, integrate-features, quality-gate) that call the CLI, so that I follow the project's conventions without re-deriving them.
125. As an AI agent, I want apply to require a prior dry-run in agent workflows, and to ask the user before applying, so that a human approves every structural change.
126. As a VS Code user, I want code lenses ("Insert after…", "Replace…", "Inject dependency…") that build selectors for me and preview the diff, so that I never write selector JSON by hand.
127. As a tool author, I want the plan and locate engines published as libraries, so that editors and other tools can build on them.

### Ecosystem

128. As a team lead, I want to install versioned template and preset packs from a git URL, so that my organization can share its own conventions.
129. As a pack author, I want packs to declare which CLI versions they support, so that users get a clear error instead of broken output.
130. As an API developer, I want URL-sourced plans and packs to default to dry-run and show a summary to confirm, so that remote content never applies unseen.

## Implementation Decisions

### Packaging and naming
- The product is codenamed **rok-cli-ultra**. The published binary package is **`rok-gen`**. Its binary is named so that cargo dispatches it as `cargo rok`.
- It lives in its own repository and is released independently of actix-api-starter. actix-api-starter becomes the **reference project**: the tool's end-to-end tests generate it, modify it and check it.
- Every crate in the tool's workspace uses the `rok-` prefix. As of 2026-10-09, all of the names below except `rok-lock` were free on crates.io. `rok-lock` is taken, so the journal crate is `rok-journal`.

| Crate | Responsibility |
|---|---|
| `rok-gen` | Binary: command definitions, input merging (defaults < file < inline JSON < flags), output formatting, wizard, MCP server mode |
| `rok-plan` | Plan, step, selector and command-argument types; JSON Schema export; plan format versioning and upgrades |
| `rok-render` | atom-engine wrapper: embedded templates, project overrides, naming filters, import stacks, workarounds for atom-engine gaps |
| `rok-locate` | Parsing, `inspect`, file-role resolution, selector resolution to byte ranges, scope checks |
| `rok-edit` | Text edits on the original source, rustfmt, line-ending preservation, staging, rollback |
| `rok-journal` | Fingerprints, transaction journal, finding edits again, undo, unlink, drift detection |
| `rok-check` | Quality gates and the architecture checker |

- `rok-plan` and `rok-locate` are also published as libraries, for the editor extension and other tools.

### Project contract
- A project is rok-managed when its root has the project config. The config holds: the CLI and template version pins, installed recipes and their drivers, the cross-feature dependency allowlist, the composition-root function name, hooks, and gate settings (e.g. coverage floors).
- File roles resolve by naming convention: a feature name plus a role (module, action, service, repository, entity, dto/<name>, lib, cargo). The special targets are the composition root, the router and ApiDoc. An explicit path is always accepted as a fallback.
- Generator wiring of shared files uses parsed structure, not text anchors. Manifests are edited in a way that keeps their comments. Rust files are edited by inserting text at positions found by parsing.

### Input and output contract
- Every command's arguments are one typed structure that drives the flags, the JSON input and the JSON Schema, so the three can't drift apart.
- Input sources: inline JSON, a file (JSON, JSONC, YAML or TOML, chosen by extension), or stdin. Merge order: defaults < file < inline < flags.
- Every plan carries a format version. Older formats are upgraded on load. Newer ones are refused.
- `--output json` gives a stable result object listing created and changed files, line ranges per edit, warnings and the transaction id.
- `--force` is a command-line flag only. It can never be set from a plan or a payload.

### Locating code
- Parsing uses syn, with source positions enabled, so every node maps back to a line and column in the original text.
- Positions: field, constructor parameter, method (impl or trait), function start, function end (before the tail), before or after a statement, replace (a statement, a function body or a whole item), `#[module]` list entries (controllers, protected, imports), and automatically managed imports.
- Statement matchers: index (including "tail"), method-call path compared token by token, the name a `let` binds, and a substring. A selector must match exactly one statement unless an occurrence or "all" is given. When it's ambiguous, the error lists every candidate.
- Nested addressing (`within`) is a fixed list of steps: if/else branch, match arm (by pattern text), for, while, loop, closure, plain block. It is deliberately not a general query language.
- Selectors can carry a `cfg` filter. File and function targets accept globs, for project-wide changes.
- The CLI's own `#[module]` attribute and `#[utoipa::path]` get dedicated parsers. Other macro contents are opaque.

### Rendering (atom-engine)
- atom-engine is pinned to an exact version. The command payload is the render context. Project-wide values (naming variants, project config) are made available to every template through atom-engine's global context.
- Custom filters: snake, pascal, camel, kebab, singular, plural, rust_type, sql_type, rust_ident.
- Shared blocks (OpenAPI annotation, handler signature, fake implementation, constructor parameter) are components with declared props. Imports are collected through a stack and rendered once.
- **atom-engine gaps are worked around in `rok-render`, never patched upstream.** That covers: guaranteeing no HTML escaping for non-HTML outputs, a brace helper for Rust brace collisions, any whitespace-control gaps, and disabling the environment and filesystem functions for third-party packs.
- Rendered output is parsed as its intended syntax kind before it's used. A failure reports the template and the payload that produced it.
- Built-in templates are compiled into the binary. Project overrides take priority and are managed with eject and diff. Presets are plan templates rendered to plan JSON.

### Editing and formatting
- Edits are text insertions and replacements on the original source at positions found by parsing. The tool never re-prints the syntax tree.
- **Formatting is never skipped.** Every written or edited file goes through rustfmt with the project's config. If a file wasn't rustfmt-clean before the edit, its formatting-only changes are reported separately from the semantic changes in the diff.
- Each file's CRLF or LF line endings are detected and preserved.
- File writes go to a staging area and are then moved into place file by file, with retry and backoff for locked files on Windows.

### Transactions and journal
- Transaction order: validate → resolve every selector → render and check every snippet's syntax → check scope → check for dependency cycles and pick the strategy → apply in memory → format → stage → write → optionally `cargo check` the affected crates once, rolling back on failure → record in the journal.
- **No marker comments.** Each edit is journaled with: transaction id, file, the function it's in, the selector as written, edit kind, the normalized token sequence of the inserted code, a content hash, and for replace, the original text.
- Finding an edit again: locate the recorded function, then the statement whose tokens match the record. Edits that can't be found, or whose hash no longer matches, are "drifted": they are reported and never modified.
- `replace` needs `--force`. It also needs the target files to have no uncommitted changes unless `--allow-dirty` is passed. Overlap with code a previous integration inserted produces a warning.
- The journal keeps one entry per edit, sorted, so most concurrent changes merge cleanly. An interrupted transaction is detectable from the journal and can be repaired by undo.

### Integration semantics
- Steps: inject_dependency (field + constructor parameter + composition-root wiring in dependency order + manifest entry + allowlist entry), insert, replace, add_field, nest_routes, add_method (needs a body for every implementor), and any generator command as a step.
- Strategies: `direct` (the target crate depends on the source crate), allowed only when it creates no cycle; and `port` (the target owns a trait, the source implements it, the composition root wires them). When `direct` would create a cycle, the CLI refuses and suggests `port`.
- Presets: has-many, hook, attachment. The wizard, presets, JSON plans and agents all produce the same plan format.
- `apply` runs a list of command steps as one batch, rolled back as a whole on failure.

### Quality gates
- Gates, in order: format, lint (clippy honoring the project's own `[workspace.lints]` levels: `deny` fails, `warn` is reported but doesn't fail, so a project that deliberately keeps `pedantic` at `warn` isn't overridden), tests (nextest when present), supply chain, unused dependencies, sqlx offline data, OpenAPI compatibility with the base branch, architecture, and optional coverage.
- The architecture rules cover: feature-crate edges against the allowlist; no cycles; services free of sqlx and concrete adapters; adapters constructed only at the composition root; every handler annotated, registered in a module and listed in ApiDoc; DTOs deriving validation and schema; handlers taking validated JSON; naming conventions; the allowlist matching the journal.
- Modes: fail-fast run, a CI mode with GitHub annotations, a changed-crates-only mode, and a fast pre-commit subset (format, lint, architecture).

### Agent surfaces
- The MCP server mode exposes inspect, plan, dry-run, apply and check, with the commands' own JSON Schemas.
- Project skills are thin wrappers over the CLI. The rules live in one place, in the CLI. In agent workflows, apply requires a prior dry-run and the user's approval.

## Testing Decisions

### What makes a good test
- Tests exercise external behavior only: given a project on disk and a command with its JSON input, assert the result JSON, the resulting files, and that the project still passes the gates. No test asserts on internal syntax trees, intermediate byte ranges or private function calls.

### The seam
- **One primary seam: the CLI's top-level entry, invoked in-process with arguments, working directory and stdin, returning the exit code and the result JSON.** It's the same entry the binary uses, so in-process tests and real invocations can't diverge. It's also the same contract the MCP mode and skills use.
- Every behavior (generators, selectors, transactions, journal, gates) is tested through that seam against a project copied into a temp directory.
- Exception, only where the primary seam is too coarse: a property-style test of statement fingerprinting (token normalization stays stable through rustfmt and line shifts), run against the `rok-journal` public API. This is the only second seam proposed.

### What is tested
- **Generators:** snapshot tests of the generated file trees, plus an end-to-end test that generates into a copy of the reference project and requires `cargo check` and `cargo rok check` to pass.
- **Selectors:** a fixture project with known statements. Every matcher, `within` step, `cfg` filter and glob resolves to the expected location. Ambiguous and missing matches fail with the expected candidate lists.
- **Transactions:** a plan with one bad selector writes nothing. A plan whose `cargo check` fails rolls back to byte-identical files. A simulated interrupted write can be repaired with undo.
- **Journal:** insert, then rustfmt, add lines above and move the function, then unlink, restores the file. Edited snippets are reported as drifted and left unchanged. Undo of a replace restores the original.
- **Replace safety:** a plan with a replace fails without `--force`. With `--force`, a dirty file is refused unless `--allow-dirty` is passed.
- **Cycles:** a `direct` integration that would create a cycle is refused with a suggestion to use `port`. The `port` strategy produces a project that compiles.
- **Rendering:** schema-versus-props consistency for every command. Payloads containing `<`, `&`, `"` and nested braces render without escaping or corruption. Third-party packs can't call the environment or filesystem functions.
- **Formatting:** every touched file is rustfmt-clean afterwards. Formatting-only changes are reported separately. CRLF fixtures keep CRLF.
- **Architecture checker:** one fixture per rule that violates exactly that rule, each producing exactly that finding. The untouched reference project produces none.
- **Input merging:** precedence (defaults < file < inline < flags), JSONC, YAML and TOML equivalence, version upgrades, and errors located by JSON pointer.

### Prior art
- The reference project unit-tests services against in-memory fakes of each repository trait (the users service against a fake user repository; auth's shared test-support fakes, including a mailer that records instead of sending). The generated fakes and generated tests copy that pattern.
- The reference project tests its local storage driver against a real temp directory. The CLI's tests use the same approach: a real project copy in a temp directory, no filesystem mocking.
- The CLI adds snapshot testing (insta) for generated output, which the reference project doesn't use yet.

## Out of Scope

- Frameworks other than actix-web, and databases other than Postgres (the `rok-` naming leaves room for later).
- General-purpose refactoring: renaming any symbol everywhere, type-aware moves, extracting functions. These belong to rust-analyzer. `rename` covers only renaming a whole feature.
- Addressing code inside macro invocations other than `#[module]` and `#[utoipa::path]`.
- Arbitrary expression positions (method-call arguments, nested struct-literal fields). Use statement-level `replace` instead.
- Type-checking snippets before insertion. The scope check is name-based, and type errors are caught by `--verify`.
- Code that only exists at build time (build scripts, `include!`, macro expansion).
- Any dependency-injection container or runtime auto-wiring. The composition root stays explicit, and the CLI only edits it.
- Fixing atom-engine upstream. All gaps are worked around in `rok-render`.
- Marker comments in user code, in any form.
- Per-feature migration ownership. Migrations stay centralized in the db crate, as in the reference project.

## Further Notes

### Known limitations to document for users
- Code inside macro calls is opaque.
- Comments between statements may end up above or below an insertion.
- Formatting files that weren't rustfmt-clean adds formatting changes to the diff (reported separately).
- `within` covers a fixed set of block types.
- The composition root must keep a recognizable shape: one function, configurable by name. Injecting inside driver-selection `match` arms isn't supported.
- Drifted snippets block unlink, undo and upgrade for that edit.
- Two identical snippets in one function are told apart only by their recorded order.
- Edits made by hand are invisible to the journal unless `doctor` infers them.
- `--verify` cost grows with the workspace size.
- sqlx compile-time checks need a database or prepared offline data.
- New Rust syntax depends on syn support.
- Editors with unsaved buffers can overwrite the tool's writes.

### atom-engine
- atom-engine 5.0.2 was first published in March 2026 and has one maintainer. The exact version pin and the rendering test suite are the safety net.
- Whether atom-engine auto-escapes based on the template name's extension (inherited from Tera) must be confirmed first. `rok-render` must guarantee no escaping regardless of the answer.

### Delivery order
1. Phase 0: create the repo, reserve the crate names, pin atom-engine and prove the workarounds, and put the reference project under git with a project config and supply-chain config.
2. Phase 1: `rok-check`.
3. Phase 2: `rok-plan`, `rok-render` and the generators.
4. Phase 3: `rok-locate` and `rok-edit`, prototyped against the reference project's users and auth crates.
5. Phase 4: `rok-journal`, replace, presets, the port strategy, the wizard.
6. Phase 5: new, recipes, apply, doctor.
7. Phase 6: MCP mode, skills, the editor extension.
8. Phase 7: upgrade and template packs.
