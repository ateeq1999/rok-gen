---
number: 4
title: "Phase 5: Implement `cargo rok doctor` command"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement `cargo rok doctor` to report project health issues.

  ## Requirements (from spec)
  - Report drifted snippets
  - Report non-conforming files
  - Report unrecorded cross-feature edges
  - Report env-example drift
  - Report pending migrations

  ## Acceptance Criteria
  - `cargo rok doctor` runs and reports all categories
  - Output is actionable with clear next steps
  - Can be run in CI to catch drift