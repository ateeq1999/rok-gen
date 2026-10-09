---
number: 13
title: "Implement `cargo rok db` commands (up, migrate, reset, seed)"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement `cargo rok db up|migrate|reset|seed` for local database lifecycle management.

  ## Requirements (from spec user story #120)
  - `cargo rok db up` - start database
  - `cargo rok db migrate` - run migrations
  - `cargo rok db reset` - reset database
  - `cargo rok db seed` - seed database

  ## Acceptance Criteria
  - All four subcommands work
  - Use project's database config
  - Integrate with sqlx