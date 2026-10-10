# Agent guidance for the Dishonored 2 Rust rewrite

Keep this file short and operational. It is not a roadmap or a research log.

## Objective and evidence

- The objective is a faithful, playable Rust rewrite using assets from the user's installed Dishonored 2 copy.
- Use the Dishonored 2 Ghidra MCP whenever a claim about an original mechanic depends on executable code. Trace callers and callees, inspect decompiled functions and machine instructions, and check data references when call arguments, layouts, or control flow are ambiguous. Cross-check conclusions against installed game files or measured runtime behavior before implementing them, and record function addresses and supporting evidence in `knowledge/dh2.sqlite`. Machine code can resolve an ambiguity. If Ghidra MCP is unavailable, find other way to use ghidra.
- During research, use Ghidra MCP directly when saved decompiled snippets do not establish the relevant function, call path, branch, data layout, or instruction behavior. Inspect the original executable in Ghidra before drawing a conclusion; do not infer missing behavior from names, file formats, or the rewrite.
- Implement behavior in the rewrite only when it is supported by original game files, decompiled game code, or measured game behavior. Never invent or substitute any unsupported behavior, regardless of subsystem or implementation detail. If the original behavior is not yet understood, investigate it until it can be implemented from evidence. Record that evidence in the database with the implementation.
- When correcting writing code, first trace how the original game handles the relevant path, then rewrite that behavior in Rust. A generic engine feature, cached answer, or special case is not a replacement for the game's algorithm.
- `knowledge/dh2.sqlite` is the source of truth for reverse-engineering findings. After a verified finding, refresh the index, record the evidence-linked finding.
- Store all research findings, interpretations, verification conclusions, coverage, plans and blockers directly in `knowledge/dh2.sqlite`. Standalone research notes/reports in Markdown, text, JSON or another file format are forbidden, including `RE_NOTES.*`, `analysis/evidence/`, Markdown reproduction notes under `knowledge/`, and findings/report files under `artifacts/`. Do not put research records into README.md or AGENTS.md; those files are operational guides.
- Raw tool/runtime captures, screenshots, executable probes and numerical test fixtures may be files when they are actual inputs or evidence linked from SQLite. Save conclusions in structured findings/function records, not alongside the capture in a narrative file. Remove temporary payload files and obsolete diagnostics once their necessary evidence is preserved in SQLite.
- Before starting any research or implementation step, quickly search the Rust rewrite source in `src/`, the existing executable probes and test cases in `analysis/` and `src/`, and the findings in `knowledge/dh2.sqlite` for that original game function or behavior. Reuse work already verified against the game; if it is partial, name the exact missing branch, data layout, call path, or integration step before continuing.
- Never commit or publish copyrighted game data.

## Ownership and failure response

- Act as the owner of this project's implementation, tools, configuration, generated artifacts, and cleanup. Use the access available to do routine work yourself; do not shift it to the user.
- Treat unexpected behavior as a defect in our implementation or setup until evidence shows otherwise. Do not invoke an unclear engine or a rewrite design choice, environment restriction, or tool policy as an explanation without investigating and identifying the concrete cause.
- Diagnose failures to their root cause, repair the responsible code or setup, and verify the original workflow works again. A copy, bypass, hard-coded exception, or temporary visual trick is not a completed fix; if one is unavoidable, label it and track the remaining root issue.
- Before calling a project or file stale, broken, or disposable, inspect its owner, process handles, configuration, and actual contents. Clean up artifacts and processes created by our work when safe, preserving user data.
- If a tool genuinely rejects an action, report the exact rejection and its impact. Do not confuse a command-tool rejection with a game, Ghidra, or operating-system failure.
- If you need to close the game or a rewrite, close it without asking the user. You are the boss.

## Running-game research

- Agents are explicitly authorized to launch, focus, run, examine, and close the user's installed original Dishonored 2 game and the Rust remake whenever needed for research or verification. No additional permission is required for these actions.
- Use live debugging, breakpoints, memory inspection, and targeted instrumentation to capture function arguments, return values, object layouts, execution paths, and state changes. Combine runtime observations with Ghidra analysis; use the running game when it can resolve an uncertainty more efficiently than further static inspection.
- Agents may ask the user to load a specific level or save, select a character or difficulty, move to a location, perform an action, or reproduce a sequence when user input or gameplay is needed. State the exact setup and action, what evidence will be captured, and which implementation question it will answer. Do routine setup and captures yourself where available tools allow it.
- Record runtime findings and their supporting evidence in `knowledge/dh2.sqlite`, including the executable version, level/save or scenario, reproduction steps, observed values, and relevant function addresses. Keep supporting captures in local artifacts and distinguish direct observations from inferences.
- Be not afraid of corrupting the game/save files. Everything is already backed up.

## Working in this repository

- Prefer a general asset-format or engine-rule fix over addingdate object-name exceptions. If an exception is needed, record why it is specific.
- When a reported example exposes a class of failures (fish, shutters, pickups, rigid bodies, constraints, or anything else), investigate and implement the class-wide rule. State which asset and entity types are covered and which remain unsupported; do not present an example-specific parser as a complete systemic fix.
- Build and run the narrowest relevant tests. Report what was actually verified, including unresolved issues. Do not call a visual fix complete before it's tested by a human.
- The user provides final visual and play confirmation. Agents may independently launch and inspect either game for runtime research and verification, and request targeted user checks when needed.
- Keep performance measurable: distinguish startup cost, CPU time, GPU load, frame time, draw calls, and memory rather than guessing from FPS alone.
- Interact with database directly, do not edit or read seed.json file.
- Delegate defaults are configured in `.pi/acp.json`: worker, researcher, reviewer, and planner use `openai-codex/gpt-6-luna`; oracle uses `openai-codex/gpt-6.1-sol`. All roles use MAX reasoning. Keep MAX when specifying per-call overrides; escalate the model for a concrete difficulty and state why.
- Keep Ghidra research output focused: begin with 80 decompiled lines or 100 instructions, inspect relevant branches and callers, and request further windows as needed. Large responses are preserved by the project extension under `artifacts/ghidra-tool-output/`; read the named capture or use a smaller Ghidra window to retrieve omitted text. Preserve exact evidence and record verified findings in `knowledge/dh2.sqlite`.
- DH2 Ghidra MCP uses the shared backend in `.pi/mcp.json`; `.pi/tools/ensure-ghidra.py` starts its single project owner automatically. Use `program: "Dishonored2.exe"`; do not switch projects during concurrent research. After an ACP update, run `node analysis/probe_pi_delegate_tools.mjs` to check that read-only delegates retain `codemode`, `tool_search`, and the `dh2_*` register tools.
- Pi uses SSE transport for this project. If model connection failures recur, inspect the recorded error codes in `artifacts/pi-network-errors.jsonl` before changing network settings or adding retries.

## Function coverage and reusable research

- The ongoing objective is the complete, faithful Rust rewrite. User requests guide priorities; all original-game research must also advance overall rewrite coverage. Do not discard understood functions because they lie outside the immediate feature request.
- For every original function inspected, first use `dh2_function_lookup` (or `python knowledge/db.py function <address>`). Check its exact Rust mapping, evidence, scope and remaining branches. Reuse verified implementations; a finding marked `implemented` may describe only one branch of a larger function.
- Implement every inspected function whose behavior is understood and whose implementation is missing or incomplete. Integrate it and verify it, including functions discovered while investigating another feature. An existing Rust/stdlib/dependency implementation may satisfy the function when its equivalence is evidenced; do not duplicate it.
- If original behavior remains unknown, continue the investigation and save the exact unanswered branch/layout/call path, the evidence collected and the next verification step as a blocker. A missing understanding is not permission to invent behavior or silently forget the function. Return to these blockers as part of completing the rewrite.
- Use `dh2_function_record` before leaving a function or compressing context: original entry address, summary, implementation path and symbol, original evidence, verification evidence, status and blocker. Use `partial` for a branch-only port. Reserve `verified` for the whole function's supported behavior, with verification evidence and no remaining function blocker. Related caller/subsystem work remains in that caller's own record.
- The project extension saves successful Ghidra function responses and their arguments under `artifacts/ghidra-tool-output/` and registers inspections in SQLite, including small responses. This does not record the agent's interpretation or establish coverage; the agent must record those. Historical batched requests are labelled unconfirmed. Interior instruction addresses remain unresolved until the owning entry is established.
- Use `dh2_coverage` at session start and before reporting completion to continue unfinished rewrite work across sessions. The function register supplements the existing findings and call graph in the same database; it is not a second source of truth.
- Use Codebase Memory for Rust symbol/reuse and caller/callee investigations: `cbm_search_graph`, `cbm_get_code_snippet`, `cbm_search_code`; load `cbm_trace_path` through `cbm_search_tools`. Project: `D-Docs-Macros-MyCheats-DH2`. The adapter checks graph freshness at startup and refreshes after Rust source changes. `/cbm:index-status` checks it; `/cbm:index-repository` rebuilds it. The graph is best effort: inspect source when cfg branches or other index gaps hide an edge; a zero-result graph query does not prove absence. Original-game behavior still requires original evidence.

## Project map

- `src/archive.rs`, `src/world_mesh.rs`, `src/bmodel.rs`, `src/entities.rs`, `src/material.rs`, `src/texture.rs`, and `src/physworld.rs` parse original data.
- `src/bin/game.rs` is the current Bevy game client; `src/navigation.rs` handles current walk collision.
- `artifacts/` contains extracted local test inputs and captures; `bin/` contains the user-facing executable; `knowledge/` contains the indexed findings and supporting scripts.
