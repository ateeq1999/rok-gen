---
number: 2
title: "Phase 5: Implement `cargo rok adopt` command"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement `cargo rok adopt` to infer file roles, write the project config, and list non-conforming files for existing actix projects.

  ## Requirements (from spec)
  - Infer file roles from existing project structure
  - Write `rok.toml` project config
  - List files that don't conform to naming conventions
  - Allow gradual adoption of older projects

  ## Acceptance Criteria
  - `cargo rok adopt` in an existing actix project creates `rok.toml`
  - Non-conforming files are listed with suggestions
  - Project can then use other `cargo rok` commands