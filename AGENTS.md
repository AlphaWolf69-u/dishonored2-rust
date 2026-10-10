# Agent guidance for the Dishonored 2 Rust rewrite

## Faithfulness and evidence

- The objective is a complete, faithful, playable Rust rewrite using assets from
  the user's legally owned installation. Never invent or substitute unsupported
  game behavior. Trace the original path before changing a mechanic or parser.
- Before research, search `src/`, existing executable probes/tests in `analysis/`,
  and the local `knowledge/dh2.sqlite`. Reuse verified work; name the exact missing
  branch, layout, caller, or integration step when coverage is partial.
- Inspect the installed executable with Ghidra when saved evidence does not
  establish behavior. Trace callers/callees and check instructions/data references
  where decompiler output is ambiguous. Cross-check against installed data or
  measured behavior before implementing.
- Where available, call `dh2_function_lookup` before original-function research,
  and use Codebase Memory for Rust symbol/caller reuse. These are optional local
  development integrations, not tools bundled with this repository. Inspect source
  when graph indexing misses cfg branches or is stale.
- Implement, integrate, and verify every inspected function whose behavior is
  understood. A Rust/stdlib/dependency equivalent is acceptable when evidenced.
  Keep unknown behavior as an explicit blocker, not a guessed fallback.
- Record original address, Rust symbol/path, evidence, exact scope, verification,
  and remaining work in the local function register (`dh2_function_record` or
  `knowledge/db.py record-function`). Use `partial` for branch-only ports; reserve
  `verified` for the whole supported function with no remaining function blocker.
- Review `dh2_coverage` or `python knowledge/db.py coverage` at session start and
  before completion so unfinished coverage carries across sessions.

## Local research and publication

- `knowledge/dh2.sqlite` is the local source of truth. Store findings, reports,
  interpretations, plans, coverage, and blockers there, not in standalone research
  Markdown/text/JSON files or in this file/README. Refresh its index after changes.
- Do not read or edit `knowledge/seed.json`. It is a private legacy snapshot,
  excluded from Git. Query SQLite directly. Archived evidence labels may identify
  database records rather than filesystem files; use `dh2_evidence_read` or the
  database evidence command for those records.
- Raw captures, screenshots, executable probes, and numerical test inputs may be
  local evidence files linked from SQLite. Preserve provenance and distinguish
  observations from inferences; remove obsolete temporary captures when safe.
- Never commit/publish copyrighted game assets, extracted original shaders/code,
  executable bytes, decompiler output, process dumps, screenshots, research
  databases, secrets, or local agent/session configuration. Keep third-party
  licenses intact. Audit history as well as the current tree before sharing.
- Original-game research must also advance overall rewrite coverage, not just the
  immediate feature. Keep understood functions and exact blockers in the register.

## Ownership and verification

- Diagnose unexpected behavior as an implementation/setup defect until evidence
  establishes the cause. Repair the responsible code/configuration and verify the
  original workflow. Do not call a workaround or visual trick a completed fix.
- Prefer asset-format or engine-rule fixes over object-name exceptions. Investigate
  the class exposed by an example, and state which types remain unsupported.
- Use available access for routine setup, captures, builds, cleanup, and process
  management. Agents may launch/focus/examine/close the original game and rewrite
  for local verification; no extra permission is needed. Ask for an exact gameplay
  setup only when user input is required to answer a concrete implementation question.
- Before deleting files or terminating processes, inspect contents/ownership and
  preserve user data. Report exact tool rejection/errors rather than guessing an
  operating-system, engine, or policy cause.
- The main agent is the sole writer when delegated investigations are read-only.
  Do not cancel unrelated/background agents or discard their research. Every failed
  delegated task must be accounted for; preserve useful work and resume where needed.
- Run the narrowest relevant tests and report what was actually verified. Runtime
  smoke tests are not final visual parity: the user supplies visual/play confirmation.
- Measure startup, CPU/GPU time, frame time, draw calls, and memory separately.

## Repository map

- `src/archive.rs`, `src/world_mesh.rs`, `src/bmodel.rs`, `src/bmd6.rs`,
  `src/entities.rs`, `src/material.rs`, `src/material_surface.rs`, `src/texture.rs`,
  and `src/physworld.rs` parse original data.
- `src/bin/game.rs` is the Bevy client; `src/navigation.rs` handles walk collision;
  `src/tome.rs` implements the currently understood visibility paths.
- `assets/shaders/` contains rewrite-authored WGSL. `vendor/` contains licensed
  patched dependencies. `artifacts/`, `bin/`, `reference/`, and `.pi/` are local-only.
- Research integrations may use a shared Ghidra backend. Use program
  `Dishonored2.exe` and do not switch projects during concurrent research. Keep
  responses focused (start with 80 decompiled lines or 100 instructions). Raw tool
  captures alone do not establish implementation coverage.
- Keep configured delegate reasoning at MAX when overriding it; escalate the model
  only for a concrete difficulty. Verify local tool allowlists after harness changes.
