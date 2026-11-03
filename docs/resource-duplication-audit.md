# Resource duplication audit

Work predictions (2026-11-03), before the corresponding ownership/format
changes. Costs are measured at acquisition and at the affected owner, rather
than inferred from whole-frame FPS.

| Change | Expected result | Evidence and measurement |
|---|---|---|
| Font metadata and lazy glyph outlines | Metadata queries decode zero outlines; library registration decodes zero outlines instead of every glyph. Only glyphs used by layout/drawing acquire host outline storage. | `14102bd60` installs callbacks into the serialized font; `14102ba20` reads metrics/bounds through the original table. `inspect-fonts` measures 200 acquisitions per mode and compares every installed outline. |
| Glyph cache R8 storage | Exactly 75% fewer CPU page bytes and GPU texel/upload bytes, with identical coverage and draw count. | `14107b7e0` creates format 6; GDraw format 6 maps to DXGI R8_UNORM (`1402e2c00`). Preserve shader red-channel coverage for all RGBA components. |
| Shared particle acquisition | One PMODEL/field-image/declaration/material decode per distinct resource in a level, including player and world references. | Capture reference and cache-miss counts before/after; keep descriptor order and per-instance selection/RNG state. |
| Shared FX graph definitions | Zero definition deep copies per runtime/declaration instance; variables, bindings and queues remain per instance. | Existing `7bc1d0`/`7bb9d0` research and runtime. Count unique definitions and instance references; owner overrides belong to runtime. |
| Compact deferred FX admission | Retain the exact deferred success/error instead of generic unsupported-family syntax. | First measure retained syntax bytes, then compare admission outcomes, including omitted descriptor defaults and malformed arrays. |

Original startup/main-menu memory snapshot: IGGY 1,377,682 bytes / 70
allocations; FONTS 9,178 / 9; PARTICLE 24,613,636 / 2,706; DECL 5,386,056 /
17,339. Read through `analysis/memory_tags.py` from the full Steam game. This
is not a same-level budget or a claim that these tags cover GPU memory.
The existing same-level baseline is `tests/fixtures/original_eotw_budget.txt`
(2026-10-28), with raw counters in the private
`artifacts/parity/original-eotw-memory-tags.tsv`: IGGY 86,057,717 bytes / 595
allocations; FONTS 9,178 / 9; PARTICLE 46,524,457 / 13,628; KISCULE
22,018,157 / 93,089; DECL 14,226,448 / 47,478; FX 9,149,240 / 15,839.
Reuse that baseline; a new level load is not needed to recover these values.

English font acquisition (200 rounds, dev optimized profile): metadata 587 us,
eager 471,074 us, lazy 35,526 us. All 1,266 outlines match; point-vector
capacity alone is 1,235,808 bytes when all outlines are decoded. Russian:
metadata 649 us, eager 525,541 us, lazy 37,993 us; all 1,476 outlines match;
point-vector capacity 1,276,152 bytes. These are acquisition measurements,
not a frame-rate estimate or a total resident-memory reduction: displayed
glyphs still decode and stay cached for the font's lifetime.

Installed Edge of the World acquisition A/B (same executable): 91 declaration
references require 91 reads without sharing and 87 with sharing; PMODEL reads
45 -> 26, field-image reads 17 -> 7. Dust District: declarations 109 -> 101,
PMODELs 27 -> 12, field images 24 -> 4. Material reads are unchanged in these
world-only acquisitions (44 and 48); player and world production acquisition
now use the same cache. The single missing-edit-scope `empty.fx` failure is
identical in both modes. Cold-versus-warm whole-load timings are not used as
proof of a CPU gain.

Edge of the World: 306 graph instances reference 138 prepared definitions.
The former deep-copy mode retains 306 runtime definitions in addition to those
prepared definitions; sharing adds no runtime definition copies. Mutable gate
values, variables and queues remain per runtime. Declaration owners are runtime
overrides; changing one cannot modify another instance's definition.

Dust District unsupported-family syntax held at least 59,888 heap payload
bytes (not allocator overhead); compact deferred outcomes hold 403. Edge of
the World has no such retained payload. This is a small absolute saving,
not a major source of the game's memory footprint. Default descriptor slots,
enabled/disabled admission and parse errors still take effect when activated.

Pause movie acquisition/draw audit: eager and lazy modes produce the same
252 vertices, 16 batches and CPU-rasterized pixel hash
`4ea5cd8f67a25f61083190ac3c60c2d90dd988fba7b29265d33c97aaf04a3a3c`.
The authored cache page is 1024x1024: 1,048,576 R8 bytes instead of 4,194,304
RGBA bytes on CPU and GPU, with the same draw count. Hardware sampling
verification is separate from this software draw audit.

Tagged live allocations (`debug_allocations`, same-build A/B over installed
Edge of the World data): particle-definition residency after releasing the
acquisition cache 1,279,016 -> 1,113,697 bytes; runtime FX graphs/state
14,625,096 -> 483,219 bytes; prepared graph definitions remain 8,963,295
bytes in both modes. Both graph modes produce the same 37 loading events,
one existing loading error and event hash
`a96064482b7d1bcaff3304f75c60a2f9c6818722b9e8869744b7e676c6d9995a`.
The same pause movie needs 1,386,288 outline bytes eagerly, versus 6,192
additional outline bytes lazily (nine drawn glyphs); font records themselves
remain 189,750 bytes per player. Text measurement now borrows serialized
bounds: zero outlines are decoded during this movie's construction.

These allocation figures are requested Rust heap bytes, not Windows working
set, GPU memory or the complete original tags (which also include gameplay
and effect families not implemented by the rewrite). Acquisition CPU numbers
above are diagnostic dev-profile measurements; release timings remain separate.

Decal animation is a missing rendering path, not a measured memory saving.
The existing sampled animation and pool binding need authored descriptor
acquisition plus the world FX scheduler/renderer connection. No claim of
complete decal support is made by a declaration-reference cache alone.

Font atlas budget audit: `14002c440` caps each side to the already loaded
movie's stage size before power-of-two rounding, then doubles above 1920x1080
except declaration names containing case-sensitive `bnet` (`140142690`).
The runtime now passes existing player metadata, without rereading Iggy bytes.
The R8 format change saves 75% at the same atlas dimensions; the stage cap is
an additional fidelity correction, not included in that saving. The installed
pause movie retains 252 vertices, 16 batches, nine lazy outlines and pixel hash
`4ea5cd8f67a25f61083190ac3c60c2d90dd988fba7b29265d33c97aaf04a3a3c`.

WORLD query ownership prediction: sunlight and local shadows require two
identical immutable trees; projected decals add a third consumer. Sharing the
WORLD resource's lazily prepared `295fa0` tree predicts 2 -> 1 constructions
without decals and 3 -> 1 with decals, without changing bounds or predicates.

WORLD query acquisition, same debug-allocation build, installed EotW WORLD:
200 factory calls, three retained consumers: 3 -> 1 distinct trees,
13,864,920 -> 4,622,536 live requested bytes (66.7% reduction, as predicted).
Acquisition: 22,974,652 -> 118,445 microseconds over 200 requests (dev profile,
not a release timing claim). Both modes: 200 queries, 11,949 hits and hash
`73b8970a1d4d2c0890e42fd88ea60975f1631bef2679a6103e42828e7a935bc1`.
