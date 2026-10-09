---
number: 9
title: "Missing generator: `cargo rok make:factory`"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement `make:factory` generator to generate test-data factories per entity.

  ## Requirements (from spec user story #19)
  - Generate test-data factories per entity
  - Tests and seeds share one source of fixtures

  ## Acceptance Criteria
  - `cargo rok make:factory posts` generates factory for Post entity
  - Factories can create valid test data
  - Seeds use the same factories