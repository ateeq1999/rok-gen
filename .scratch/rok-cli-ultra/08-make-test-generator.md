---
number: 8
title: "Missing generator: `cargo rok make:test`"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement `make:test` generator to generate integration tests per action and fake-based unit tests per service.

  ## Requirements (from spec user story #18)
  - Generate integration tests per action
  - Generate fake-based unit tests per service
  - New code starts with coverage

  ## Acceptance Criteria
  - `cargo rok make:test posts` generates tests for all actions in posts
  - Integration tests use real database
  - Unit tests use in-memory fakes
  - Generated tests pass `cargo test`