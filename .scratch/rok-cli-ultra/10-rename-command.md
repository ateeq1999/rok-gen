---
number: 10
title: "Implement `cargo rok rename` command"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement `cargo rok rename posts articles` to rename a feature crate, files, types, routes, dependent integrations, and add a table-rename migration.

  ## Requirements (from spec user story #116)
  - Rename the crate, files, types, routes
  - Update dependent integrations
  - Add a table-rename migration

  ## Acceptance Criteria
  - `cargo rok rename posts articles` renames everything
  - Migration is created for table rename
  - All references updated
  - Project still compiles and passes checks