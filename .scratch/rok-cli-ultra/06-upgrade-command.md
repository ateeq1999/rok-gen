---
number: 6
title: "Phase 7: Implement `cargo rok upgrade` command"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement `cargo rok upgrade` to apply newer template and recipe versions with a 3-way merge.

  ## Requirements (from spec)
  - Apply newer template and recipe versions with a 3-way merge
  - Project keeps receiving improvements after it was created
  - Handle conflicts gracefully

  ## Acceptance Criteria
  - `cargo rok upgrade` updates templates and recipes
  - 3-way merge preserves local changes
  - Conflicts are reported and can be resolved