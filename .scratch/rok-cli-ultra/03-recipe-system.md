---
number: 3
title: "Phase 5: Implement recipe system (`add`, `remove`, `list`)"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement the recipe system for adding/removing optional capabilities like auth, mail, storage.

  ## Requirements (from spec)
  - `cargo rok add mail --driver resend` adds a capability after project creation
  - Recipes declare what they require; `add` installs those first
  - `cargo rok remove` refuses while other features depend on the recipe, and names them
  - Recipes add env keys to example env file, never to the real one
  - Recipes optionally bring Terraform modules
  - `cargo rok list` shows installed and available recipes and their versions

  ## Acceptance Criteria
  - `cargo rok add auth --driver session` works
  - `cargo rok add mail --driver resend` works
  - `cargo rok add storage --driver s3` works
  - `cargo rok remove` refuses with dependent features named
  - `cargo rok list` shows recipes with versions
  - Env keys go to `.env.example` only