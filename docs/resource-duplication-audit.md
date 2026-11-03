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

Release font acquisition (200 rounds): English metadata 417 us, eager 192,660 us,
lazy 27,304 us; Russian metadata 458 us, eager 200,791 us, lazy 27,926 us.
These runs use the same acquired resources and compare all decoded outlines.

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
RGBA bytes on CPU and GPU, with the same draw count. The later D3D12 capture below verifies the uploaded glyph coverage and draw
count separately from this software draw audit.

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
include both diagnostic dev-profile and main release-profile measurements.

Authored ordinary decal animation is now acquired and connected to the world
FX scheduler and receiver renderer. This closes a fidelity gap; its declaration
reference cache alone would not constitute the rendering feature. Skin-decal
FX is a separate family and remains unsupported.

Ordinary decal integration uses `arkFXDecalDesc` (reflection `143ae3730`,
constructor `217d50`): all descriptor phases, attachment axes, initial ranges
and reference alternatives are acquired through typed definitions. A level
reference cache retains shared resolved material-name sets. The 256-entry
FX family (`19a0b0`/`19f230`) remains distinct from the renderer projection
budget (4..128, `r_maxDynamicDecals`). First active application selects the
reference (`218620`), after initialization samples (`218e80`), preserving the
manager random stream. Group start, stop, clear and nested sub-FX chains now
carry the ordinary decal family. Receiver submission is connected, with
material admission through the prepared shared program table. GPU submission and animated visible output are confirmed by the capture
reported below; declaration parsing alone is not the verification.

The installed `arkDecal` program supplies the projection rules: NONE=0,
CLIP_MODEL=1, CULL_MODEL=2, DEPTH_BOX=3. The depth-box path reconstructs
receiver position and normals from scene depth; it must not be replaced by
a translucent box surface. Model/WORLD geometry paths preserve the four
resolved receiver/instancing material alternatives. Skin decals remain a
separate FX family. Renderer integration predicts no additional acquisition
reads per update: definitions and material references are prepared once.

Host submission prediction before connecting projected geometry: one bounded
FX/projection record per original slot, shared mesh assets and one renderer
proxy per material/layout, rather than entities and converted mesh copies for
every projection/receiver pair. Existing particle direct draws retain their
layout, instance count and geometry path. The direct-draw service accepts
explicit shared mesh draws and per-owner instance bindings; no CPU vertex
conversion or GPU mesh duplication belongs to the per-frame decal update.

The typed projection-program acquisition was exercised on the installed blood
WORLD material: DEPTH_BOX, FX enabled, colour-adjust RGB 0.3564, box projection
[2.5, 0.9], alpha blur 0.01, and the authored blood opacity image. Its descending
sphere-fade edges require the HLSL smoothstep polynomial, not WGSL's undefined
reversed-edge builtin. The installed PS confirms stable quadratic roots and
the smaller-root selection. Depth normal reconstruction uses right/left for U
and **up/down** for V (`MinDiff(centre, up, down)`), not screen-down V.

Installed-declaration coverage: all 25 decal declarations resolve 99 distinct
material variants through the typed projection-program reader with zero errors
(74 CLIP_MODEL, 25 DEPTH_BOX; 52 FX-opacity materials). This proves acquisition
coverage, not rendering or visibility. The new host direct-mesh path uses the
existing single direct-surface lookup per mesh command; it does not add another
map lookup to every ordinary draw. Particle publication now replaces only its
own entries so it cannot erase another surface owner's draws.

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

Receiver bounds come from the existing model owner's prepared world bounds,
including model motion and physics updates (`25da70`/`330c10`). Per-surface
`RenderInstance.bounds` are already world-space; transforming them again would
incorrectly reject receivers. Layout proxies borrow the source skin palette
and select Bevy's corresponding skinned mesh binding without creating another
skeleton or uploading its joints again. Geometry decal homogeneous-Z bias is
excluded from DEPTH_BOX, as the installed arkDecal vertex program requires.

Connected-path evidence: the installed blood_splat_01 FX is acquired through
placed-actor playback at the player's actual start transform. The live run
publishes one projection and one pair and exits normally after 379 measured
frames. RenderDoc event 21335 contains the depth-box cube draw (36 indices,
one instance), with actual scene-depth, projection/remap, opacity, material
and lighting inputs. This first capture is near activation, where authored FX opacity can still be
zero. The later capture below establishes visible RGB output.

Final D3D12 acquisition modes (same executable, installed EotW, same saved
camera): sparse and eager receiver publication both submit the depth-box cube
with 36 indices and one instance at event 22947. The sparse draw changes 1,400
HDR RGB pixels; the eager draw changes 1,139. Authored randomized scale/colour
and effect sampling times differ between the two runs, so those are not a
pixel-equality comparison. Receiver buffer bytes are 64 versus 315,392, exactly
the predicted 4,928 -> 1 record reduction. The lazy path uses the same receiver
record constructor and updates each remap alongside its record; projection
admission and draw ordering are unchanged. Only referenced records are uploaded.

The actual paused-game glyph atlas has the same normalized coverage SHA-256
in R8 and expanded RGBA:
`59abbd2bf8dd8bbdb65335fb13e909c1e7f88e7f9b37f98c7049ecdde7692d3c`.
Captured storage is 1,048,576 versus 4,194,304 bytes (75% less); both submit
98 UI draws and zero dispatches. Both use the same red-channel glyph shader
and identical active stage transform. Whole UI canvas/vertex hashes differ
between independently timed captures (animated UI and scene background), so
no whole-frame pixel equality is claimed. The deterministic installed-movie
software draw audit above verifies unchanged glyph geometry and pixel output.
No unit tests or synthetic oracle executables were written or run for this work.

The backend and existing WORLD packet builder share the `298ee0` material
admission rules, including its cursor advance over excluded receiver ranges.
The range-end summary is prepared from the original WORLD material classes;
it is not a camera-distance heuristic. This keeps absent/DEPTH_BOX material
variants and bit-4/bit-5 exclusions consistent across the host draw service
and the already ported packet construction.

After sharing the WORLD admission rules, the final game build completed and
an additional 382 measured gameplay frames exited normally with an active
projection and both a model receiver and depth-box receiver (two pairs, two
uploaded receiver records). No graphics-validation error or runtime error was
reported. These dev-profile frames are a runtime check, not a release FPS claim.
