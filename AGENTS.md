# Agent guidance for the Dishonored 2 Rust rewrite

Keep this file short and operational. It is not a roadmap or a research log.

## Objective and evidence

- The objective is a faithful, playable Rust rewrite using assets from the user's installed Dishonored 2 copy.
- Use the Dishonored 2 Ghidra MCP whenever a claim about an original mechanic depends on executable code. Machine code can resolve an ambiguity. If Ghidra MCP is unavailable, find other way to use ghidra.
- Implement behavior in the rewrite only when it is supported by original game files, decompiled game code, or measured game behavior. Never invent or substitute any unsupported behavior, regardless of subsystem or implementation detail. If the original behavior is not yet understood, investigate it until it can be implemented from evidence.
- When correcting writing code, first trace how the original game handles the relevant path, then rewrite that behavior in Rust. A special case is not a replacement for the game's algorithm.
- Before starting any research or implementation step, quickly search the Rust rewrite source in `src/`, the existing executable probes and test cases in `analysis/` and `src/` for that original game function or behavior. Use `rg --no-ignore` for `analysis/` (normal ripgrep skips these local-only probes). Reuse work already verified against the game
- Never commit or publish copyrighted game data.

## Ownership and failure response

- Act as the owner of this project's implementation, tools, configuration, generated artifacts, and cleanup. Use the access available to do routine work yourself; do not shift it to the user.
- Treat unexpected behavior as a defect in you implementation or setup until evidence shows otherwise. Do not invoke an unclear engine or a rewrite design choice, environment restriction, or tool policy as an explanation without investigating and identifying the concrete cause.
- Diagnose failures to their root cause, repair the responsible code or setup, and verify the most optimal workflow works again. A copy, bypass, hard-coded exception, or temporary visual trick is not a completed fix; if one is unavoidable, stop and report to user.
- If you need to close the game or a rewrite, close it without asking the user. You are the boss.
- Do not stop working untill you implemented the thing you were asked to implement in its entierty, unless you cannot progress further or need some action from the user.

## Running-game research

- Agents are explicitly authorized to launch, focus, run, examine, and close the user's installed original Dishonored 2 game and the Rust remake whenever needed for research or verification. No additional permission is required for these actions.
- Launch the original through Steam AppID `403640`, never directly through `Dishonored2.exe`: a direct launch enters the free trial.
- Original-game keyboard control needs physical scan-code input: `analysis/shadow_research_window.ps1 -VirtualKey` uses verified foreground ownership and `SendInput`; `SendKeys` and zero-scan `keybd_event` can appear successful without controlling the game. From the title use Space, then Enter for Campaign, then Enter for Continue.
- Use live debugging, breakpoints, memory inspection, and targeted instrumentation to capture function arguments, return values, object layouts, execution paths, and state changes. Combine runtime observations with Ghidra analysis; use the running game when it can resolve an uncertainty more efficiently than further static inspection.
- Agents may ask the user to load a specific level or save, select a character or difficulty, move to a location, perform an action, or reproduce a sequence when user input or gameplay is needed. State the exact setup and action, what evidence will be captured, and which implementation question it will answer. Do routine setup and captures yourself where available tools allow it.
- Be not afraid of corrupting the game/save files. Everything is already backed up.

## Working in this repository

- Build and test with the debug (dev) profile, or `cargo check` when only compilation matters. Use `--release` only for the main branch build the user tests from and for performance measurements (timings, frame rate, latency); the release profile uses LTO and takes several times longer.
- Keep compilation hidden with output redirected to logs while continuing other work. Do not open build-monitor terminals unless the user requests one or the agent is handing control back immediately after starting the build.
- ChatGPT/Codex agents only (not Claude): do not check build progress, poll build sessions, inspect build logs/processes for progress, or spend time waiting for builds. Continue useful project work while a build runs; read its final result when needed after that work.
- Game behavior is the naming default: name files, modules, types, functions, fields, constants, shaders and fixtures by responsibility without redundant `Native`, `Original`, `native_` or `original_` qualifiers. This also applies to project-authored dependency patches and diagnostic controls. Mark host integration or deliberate deviations explicitly (for example `bevy_backend`, `debug_` or `approximate_`); keep meaningful comparison provenance in comments and test descriptions. Preserve installed resource names and historical evidence paths.
- Prefer a general asset-format or engine-rule fix over addingdate object-name exceptions. If an exception is needed, record why it is specific.
- When a reported example exposes a class of failures (fish, shutters, pickups, rigid bodies, constraints, or anything else), investigate and implement the class-wide rule. State which asset and entity types are covered and which remain unsupported; do not present an example-specific parser as a complete systemic fix.
- Build and run the narrowest relevant tests. Report what was actually verified, including unresolved issues. Do not call a visual fix complete before it's tested by a human.
- Write a test only when its expected values come from outside the code under test: outputs of the original executable (oracle probes), live measurements of the running game, or installed game data. Never write circular tests that assert code returns what it already returns, tests for behavior you invented, or heuristic tests whose pass/fail threshold is an unsourced number. When such a test is found, replace it with evidence or delete it. Spend the time on implementation and evidence instead.
- The user provides final visual and play confirmation. Agents may independently launch and inspect either game for runtime research and verification, and request targeted user checks when needed.
- Keep performance measurable: distinguish startup cost, CPU time, GPU load, frame time, draw calls, and memory rather than guessing from FPS alone.
- Ghidra Large responses are preserved by the project extension under `artifacts/ghidra-tool-output/`; read the named capture or use a smaller Ghidra window to retrieve omitted text.
- DH2 Ghidra MCP uses the shared backend in `.pi/mcp.json`; `.pi/tools/ensure-ghidra.py` starts its single project owner automatically. Use `program: "Dishonored2.exe"`; do not switch projects during concurrent research. After an ACP update, run `node analysis/probe_pi_delegate_tools.mjs` to check that read-only delegates retain `codemode`, `tool_search`, and the `dh2_*` register tools.
- Pi uses SSE transport for this project. If model connection failures recur, inspect the recorded error codes in `artifacts/pi-network-errors.jsonl` before changing network settings or adding retries.

## The smallest complete change

Take the first option that fully works:
2. Already in this codebase (a helper, component, service, pattern)? Use it the way the surrounding code does.
3. Standard library or a platform feature? Use it, unless the project has its own. A house component beats a native widget.
4. An installed dependency? Use it. Never add a dependency for a few lines.
5. Can it be one line a reader gets at a glance? One line.


## Function coverage and reusable research

- The ongoing objective is the complete, faithful Rust rewrite. User requests guide priorities; all original-game research must also advance overall rewrite coverage. Do not discard understood functions because they lie outside the immediate feature request.
- Implement every inspected function whose behavior is understood and whose implementation is missing or incomplete. Integrate it and verify it, including functions discovered while investigating another feature. An existing Rust/stdlib/dependency implementation may satisfy the function when its equivalence is evidenced; do not duplicate it.
- If original behavior remains unknown, continue the investigation and save the exact unanswered branch/layout/call path, the evidence collected and the next verification step as a blocker. A missing understanding is not permission to invent behavior or silently forget the function. Return to these blockers as part of completing the rewrite.

## Repository map

- `src/lib.rs` exposes shared code; `src/assets/` reads installed resources and
  original formats. `src/assets/level.rs` resolves native level companions.
- `src/rendering/` contains verified material, lighting, model and shadow rules;
  `src/rendering/bevy_backend/` integrates those rules with Bevy/GPU resources.
- `src/visibility/` implements understood Tome queries; `src/gameplay/` owns
  clocks, entity runtime and navigation; `src/platform/` owns system settings.
- `src/app/` composes the client in focused modules. `src/bin/` contains entry
  points and diagnostic tools, all using the same library.
- `tests/fixtures/` contains numerical comparisons; `assets/shaders/` contains
  rewrite-authored WGSL. `docs/architecture.md` documents boundaries and checks.
- `vendor/` contains licensed patched dependencies. `artifacts/`, `bin/`,
  `reference/`, and `.pi/` are local-only. No extraction is needed to launch.
