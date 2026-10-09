---
number: 11
title: "Implement `cargo rok graph` command"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement `cargo rok graph` to draw features, recipes and integration edges as Mermaid or DOT, highlighting cycles and edges not on the allowlist.

  ## Requirements (from spec user story #118)
  - Draw features, recipes and integration edges
  - Output as Mermaid or DOT
  - Highlight cycles
  - Highlight edges not on the allowlist

  ## Acceptance Criteria
  - `cargo rok graph --format mermaid` outputs Mermaid diagram
  - `cargo rok graph --format dot` outputs DOT diagram
  - Cycles are highlighted in red
  - Unsanctioned edges are highlighted