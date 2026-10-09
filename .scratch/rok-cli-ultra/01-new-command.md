---
number: 1
title: "Phase 5: Implement `cargo rok new` command"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement `cargo rok new <name>` to create a complete, compiling project from the reference template.

  ## Requirements (from spec)
  - Create a complete, compiling project from the reference template (actix-api-starter)
  - Allow choosing recipes and drivers at `new` time (e.g. auth, mail via Resend, storage via S3)
  - Initialize git repo, CI workflow, pre-commit hook, supply-chain config and project skills
  - Record tool version and template version in project config
  - Project should pass `cargo check` and `cargo rok check` from the first commit

  ## Acceptance Criteria
  - `cargo rok new my-api` creates a working project
  - `cargo rok new my-api --recipe auth --driver resend` adds auth with Resend
  - Generated project has git repo, CI, pre-commit hook, supply-chain config
  - Project config records CLI and template versions
  - `cargo check` and `cargo rok check` pass in the new project