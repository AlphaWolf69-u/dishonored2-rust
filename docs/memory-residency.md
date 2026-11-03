# Memory residency audit, 2026-11-03

The comparison reference is `tests/fixtures/original_eotw_budget.txt` and its
raw per-tag capture, `artifacts/parity/original-eotw-memory-tags.tsv`.
The original's Edge of the World snapshot reports 8,626 MiB private commit
and 3,897 MiB working set. These counters describe different things;
private commit includes pages which are not currently in RAM. The gameplay
overlay labels both separately.

A fresh rewrite run before this change, with allocation attribution enabled,
reported 6,228 MiB private and 3,122 MiB resident after streaming settled.
This is one level/view, not evidence about long traversal or every level.
The historical 10,495/6,724 MiB rewrite load snapshot predates the staging,
mesh residency and resource-sharing fixes and is not the current baseline.

Largest measured retained allocations included about 199 MiB of compressed
sound media, a 144 MiB collision triangle allocation, 89 MiB of decoded menu
bitmaps, and a 73 MiB visibility Tome. The original's corresponding tag
budgets include WWISE 240,890,571 bytes, HAVOK 103,479,168 plus TRIANGLES
50,402,432 bytes, IGGY 86,057,717 bytes, and UMBRA_TOME 77,553,524 bytes.
Tag boundaries differ: a bitmap subtotal is not the whole IGGY budget.

The world CPU meshes and 227 MB per-vertex probe serialization are already
released after GPU extraction. Only eight tiny mutable/procedural meshes and
19 MiB of CPU shader-buffer payloads remained. Removing these resources a
second time cannot account for a large saving.

## Changes and predictions

- Collision index: retain exact triangles/bounds and the existing candidate
  filter/sort order. Release triangle spare capacity (about 38 MiB for
  1,550,597 triangles) and replace separately allocated, over-capacity usize
  cell lists with contiguous exact-sized u32 lists. The expected list saving
  is the old capacities times eight minus the used count times four, plus
  the smaller cell descriptors; production diagnostics report both totals.
  Query costs are compared in the same release executable using
  `DH2_DEBUG_COLLISION_EXPANDED` and `DH2_DEBUG_COLLISION_TIMING` during the
  existing walking benchmark. No triangle decompression moves onto queries.
- UI bitmaps: `1402e2c00` maps raw format 13 to DXGI BC7_UNORM and passes the
  installed blocks directly to CreateTexture2D. Retain/upload those same
  blocks instead of CPU RGBA decode. Prediction: 75% less retained payload
  and bitmap GPU storage for aligned BC7 images, with decoding work removed.
  `DH2_DEBUG_UI_EXPANDED_BITMAPS` selects the former representation in the
  same executable for comparison. Existing mip selection is unchanged.
- Encoded sound: the loaded bank allocations alone account for about 199 MiB;
  prepared ADPCM/Vorbis sources retained another roughly 208 MiB of encoded
  payloads. `1410f1910` reuses the loaded Sound, and `1411284e0`/`141122400`
  publish its source descriptor rather than another copy per event. Share
  byte ranges of the retained bank through preparation and codec cursors.
  Expected saving: approximately 190–208 MiB where prepared sources overlap
  the resident bank DATA. Package-only sources remain resident once. Range
  indexing does not change packet/block offsets, bytes, precision, or reads.
  `DH2_DEBUG_SOUND_COPIED_MEDIA` restores the copied representation for A/B;
  physical allocation identities distinguish shared views from duplicate
  ownership. Optional `DH2_DEBUG_SOUND_HASHES` captures actual source hashes
  and playback metadata. Competing parallel preparations publish the same
  cached identity; losing preparations no longer escape into event graphs.

## Initial release measurements

Same executable, Edge of the World walking benchmark, expanded then packed:

| Owner | Before | After | Saving |
|---|---:|---:|---:|
| Collision triangles/cell index, accounted payload | 192,644,480 B | 128,195,648 B | 64,448,832 B |
| Cached UI bitmap payloads | 125,255,232 B | 31,313,808 B | 93,941,424 B |

Both collision modes have 1,550,597 triangles and 80,107 occupied cells.
Collision accounting includes vector capacities and cell entries at the
map's logical capacity; hash-table control/bucket slack and allocator
headers are excluded. It conservatively understates the smaller table's
actual allocation saving slightly.
The packed-cell and spare-capacity saving matches the layout prediction;
bitmap storage falls exactly 75%. Bitmap resource loading took 347.398 ms
versus 62.959 ms. Collision-query cumulative diagnostic snapshots averaged
1.446 versus 1.364 microseconds per call (81 versus 69 snapshots, each after
at least 512 real gameplay queries). Reverse-order repeat: packed 1.337,
expanded 1.350 microseconds/query (70 snapshots each). Repeat bitmap loading
took 58.202 versus 331.532 ms. All four runs exited successfully without GPU
validation errors. These are owner allocations, not a claim that process
working set falls by the same amount immediately.

## Shared encoded audio

Same development executable, copied/shared/shared/copied order, actual
Edge of the World camera sweep with audio enabled:

| Encoded allocations, including resident banks | Copied | Shared |
|---|---:|---:|
| Unique bytes after level sounds are prepared | 428,652,811 | 211,296,428 |
| Source backing allocations | 9,047 | 18 |
| Sources referring directly to bank allocations | 0 | 9,031 |

The encoded backing-byte saving is 217,356,383 bytes (207.287 MiB), within
the predicted 190–208 MiB. The other two changes save at least 151.053 MiB:
combined, about 358.340 MiB of live owner storage is eliminated.

All 9,047 source byte signatures, channels, sample rates, frame counts and
loop points match; neither run has extra/missing sources. Packet/block decode
and the mixer are unchanged; source views retain the same encoded bytes.
The 1,661-event preparation wall time was 348.232/335.139 ms copied versus
260.617/278.867 ms shared. This is parallel startup wall time, not sampled
CPU time or a claim about frame-rate improvement. The subsequent 33-event
batch remained 6–8 ms.

Settled working set was 3,136/3,172 MiB copied versus 2,920/2,919 MiB shared;
private commit was 6,894/6,908 versus 6,651/6,645 MiB. The means differ by
234.5 MiB resident and 253 MiB private. These process counters include
allocator/driver variation; the exact reproducible saving is the owner byte
count above. Source signatures and measurement summaries are preserved in
`artifacts/residency-audio-summary.json`.

## UI lifetime cleanup

The GPU bitmap generation cache already re-uploaded each newly opened movie.
Glyph IDs, however, are globally unique and the old pruning rule retained
pages absent from the current movie forever. Closing now releases bitmap and
glyph textures once, using the existing uploaded-list marker. No reusable
next-open bitmap/glyph upload is lost: a new movie already had a new bitmap
generation and new glyph IDs. Shared ramps, render targets and programs stay
cached. This prevents an additional atlas page per repeated menu open from
accumulating; no unmeasured process-byte saving is assigned to this cleanup.

## GPU composition and build verification

Device-bracketed D3D12 captures of the production pause renderer contain
98 UI draws with the identical ordered vertex counts in both modes. All
41 library bitmap textures change from RGBA8_UNORM to BC7_UNORM; their
captured payload falls from 29,787,968 to 7,446,992 bytes, exactly 75%.
The 1,048,576-byte R8 glyph page has the same hash in both captures. There
are no extra UI draws and no change to glyphs or draw geometry. No images
were visually inspected. The first capture attempts selected the unrelated
D3D11 presentation device and were rejected; the accepted captures use the
existing `debug_graphics_capture` device-bracketing path.

The game development build, `cargo check --features game --bins`, and
`cargo check --no-default-features --lib` pass. No tests were written or run.
The main-branch release build completed successfully at source commit
`119c636`; its result is recorded in
`artifacts/residency-main-release-build.result.json`.

## Walking CPU regression measurement

Actual pre-change release executable (a6c6418) versus the complete memory
change release executable (119c636), not a development build or just the
collision diagnostic switch. Executable SHA-256 values:

- Before: `522163ee4688f5fd46578fbbea5d9cedfe9d6c2fb36cdac4195dc25571e9e610`.
- After: `eb5955316f9cd2eb24b2eb03aef8ac0f93c1ba025ac13005bfda1f3cfff60438`.

Both use the same installed resources, rewrite home, original settings,
1920x1080 output, uncapped rendering and EotW walking route. Audio is enabled
and the normal player controller walks there and back between game-space
positions (-30.81336, 192.69846, 2.94306) and
(-30.81336, 182.69846, 2.94306). The separate camera-audio diagnostic is
disabled because it bypasses player walking. No pause menu is opened.

Order: before/after/after/before. Each CPU interval starts 10 seconds after
complete-rendering readiness and lasts 12 seconds. `GetProcessTimes` and
`GetThreadTimes` capture active kernel+user CPU time, including compute,
rendering, audio, IO and driver threads. The existing `DIRECTIONAL_FRAMES`
counter counts completed directional submissions, one per render frame on
this sunlit route; its data-symbol address is resolved from the matching PDB
and read without modifying/suspending the game. This counts render work
directly instead of depending on the display-present stream. No stack
sampling runs during the accepted measurements.

| Run | Rendered frames | All-thread CPU ms/rendered frame |
|---|---:|---:|
| Before 1 | 1,190 | 42.266 |
| After 1 | 1,237 | 42.555 |
| After 2 | 1,237 | 42.618 |
| Before 2 | 1,221 | 44.533 |

Frame-weighted comparisons (4,885 measured render frames):

| Metric | Before | After |
|---|---:|---:|
| All-thread active CPU, ms/rendered frame | 43.414 | 42.587 |
| Main-thread active CPU, ms/rendered frame | 0.907 | 0.903 |
| Compute workers active CPU, ms/rendered frame | 37.446 | 36.852 |
| Audio callback active CPU, ms/wall second | 165.361 | 159.502 |
| Main frame wall span, ms (includes waits) | 7.574 | 7.623 |
| Render frame wall span, ms (includes waits) | 8.005 | 7.766 |
| Render submission interval, ms | 9.955 | 9.701 |

All-thread CPU time is summed across parallel threads, so it can exceed a
frame's wall duration. The aggregate CPU/frame mean is 1.9% lower, but the
before runs vary by 5.4%; this does not establish a significant CPU speedup.
The small main-wall-span change (+0.049 ms) is within its before-repeat range
(7.510–7.637 ms), while main-thread active CPU is unchanged. No measurable
walking CPU regression was found. This comparison covers this route and
active audio; it is not a claim about every level or menu interaction.

Raw counters, frame-counter endpoints, thread names, CPU phase readings,
configuration and hashes are preserved in
`artifacts/parity/residency-counter-v3-*.json` and
`artifacts/residency-walk-cpu-comparison.json`. Earlier ETW/PresentMon
calibration captures are excluded: release function attribution was
untrustworthy and the display stream did not reliably cover the counter
interval. No tests were written or run for this measurement.

Measurement logs live in `artifacts/residency-*.log`; no game bytes are
included in this document or committed.
