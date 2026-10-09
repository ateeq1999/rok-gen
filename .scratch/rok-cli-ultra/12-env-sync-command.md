---
number: 12
title: "Implement `cargo rok env sync` command"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement `cargo rok env sync` to keep the example env file in sync with the keys the config crate reads.

  ## Requirements (from spec user story #119)
  - Keep `.env.example` in sync with config crate keys
  - Onboarding never misses a variable

  ## Acceptance Criteria
  - `cargo rok env sync` updates `.env.example`
  - All config keys are present
  - No secrets in `.env.example`