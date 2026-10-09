---
number: 7
title: "Phase 7: Implement template packs (`templates eject`, `templates diff`)"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement template pack management: eject built-in templates and diff against current versions.

  ## Requirements (from spec)
  - `cargo rok templates eject <name>` copies a built-in template into project as override
  - `cargo rok templates diff` compares overrides with current built-in templates
  - Template packs can be installed from git URL
  - Packs declare which CLI versions they support

  ## Acceptance Criteria
  - `cargo rok templates eject feature/Cargo.toml` creates project override
  - `cargo rok templates diff` shows differences
  - Packs can be installed from git URL
  - Version compatibility is checked