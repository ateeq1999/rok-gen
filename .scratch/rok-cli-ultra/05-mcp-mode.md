---
number: 5
title: "Phase 6: Implement `cargo rok mcp` MCP server mode"
labels: ["ready-for-agent"]
body: |
  ## Description
  Implement `cargo rok mcp` to expose inspect, plan, dry-run, apply and check as MCP tools with their JSON Schemas.

  ## Requirements (from spec)
  - Expose inspect, plan, dry-run, apply, check as MCP tools
  - Each tool has its JSON Schema
  - AI agents can make structural, rule-checked edits instead of free-form text edits
  - Project skills (add-feature, add-endpoint, integrate-features, quality-gate) call the CLI

  ## Acceptance Criteria
  - `cargo rok mcp` starts an MCP server
  - Tools are discoverable via MCP
  - JSON schemas are published for each tool
  - Project skills work with the MCP server