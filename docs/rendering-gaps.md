# Rendering gaps

Rendering paths the rewrite does not draw yet, ranked by how much installed
content they block, with what is known and the next step. Remove an entry
when it is implemented.

Program census (2026-11-10): `mainProgram` of every installed material
declaration. Particle programs connected: `particle/add_default`,
`dist_blur`, `scrolling_water`, `liquid`, `dissolve`, `add_liquid`, `water`,
`fire`. The two left (`blood`, `filing_container`) are opaque.

| Gap | Installed content | Status |
|---|---|---|
| `arkhair` | 134 materials | none: character hair program; needed once characters render |
| `particle/blood` | 19 materials (combat blood, camera blood, eotw fish blood) | blocked: see below |
| `arkeyeball` | 23 materials | none: character eye program |
| Particle surfaces casting shadows | eotw stink bugs (`bug01`), `dust_scorie` | blocked: see below |
| World FX target/tag pose producer | eotw world FX | none: `world_particle_state` refuses targeted FX |
| `arkKisculeNodeEntitySpawnerControl` | eotw outsider shrine FX | none |
| `particle/filing_container` | 8 materials | blocked with `particle/blood`: opaque (`blend ONE ZERO`, sortCoverage, zPrePass motion vectors) |

## Particle declaration census (2026-11-10)

All 8,909 installed FX declarations loaded through `WorldFxSources`
(local census `examples/fx_failure_census.rs`; `examples/` is not tracked): effects=8909 failed_effects=10 particles=2171 failed_particles=1233.
Each particle declaration fails at its first unsupported field, so counts
below a top reason can hide further gaps. Since this table: flipbook and
radial-ramp opacity, bare rotation orders, clipping planes, stage
sorting, stage-to-stage spawning, attractors, stage trails and
declaration trails, beams and ribbons are connected; 846 particle
declarations still fail (15 at beams whose end uses the FX target
parameters, below).

| Count | First failure |
|---|---|
| 578 | particle: unresolved particle declaration producer m_havokStages |
| 77 | particle: unresolved particle spawn producer m_physicsShapesSpawn |
| 75 | particle: unresolved particle declaration producer m_beams |
| 73 | particle: unresolved particle clipping field m_planes |
| 55 | particle: unresolved particle stage producer m_attractors |
| 55 | particle: unresolved particle stage producer m_sorting |
| 52 | particle: unresolved particle stage producer m_trails |
| 47 | particle: unresolved particle parent-stage field m_stageName |
| 46 | particle: unresolved particle declaration producer m_occlusionKill |
| 35 | particle: unresolved particle declaration producer m_trails |
| 27 | particle: inline mip chain size mismatch |
| 26 | particle: unknown particle mesh rotation order YXZ |
| 24 | particle: unresolved particle declaration producer m_ribbons |
| 15 | particle: opaque particle pixel-lighting inputs are not connected |
| 12 | particle: unresolved particle declaration producer m_occlusionQuery |
| 11 | particle: unresolved particle spawn producer m_velocityCycleOrientation |
| 9 | effect: unknown sub-FX field m_name |
| 5 | particle: unknown particle mesh rotation order ZYX |
| 4 | particle: unknown field rotation order YXZ |
| 4 | particle: unresolved particle parent-stage field m_initialVelocity |
| 3 | particle: unknown particle mesh rotation order ZXY |
| 2 | particle: FX array index N exceeds declared count |
| 2 | particle: particle program particle/fire is not connected |
| 2 | particle: unknown particle mesh rotation order YZX |
| 1 | effect: sound declaration has no engine ID |
| 1 | particle: unknown particle mesh rotation order XZY |
| 1 | particle: unresolved particle declaration producer m_fxEvents |
| 1 | particle: unresolved particle parent-stage field m_lifeRestriction |

## particle/blood (opaque pixel-lit particles)

`particle/blood` shares `particle/water`'s fragment (connected) but defaults
to `blend ONE ZERO`. Such a particle material is not `isAlpha`: the original
draws particle renderers as ordinary dynamic model surfaces of their
material (1401fbad0), so an opaque one goes through the material's zPrePass
and main passes. Its sun shadow then comes from the screen-shadow mask built
from that prepass (`parallel_depth_visibility` route 0 reads
`screen_visibility`), and its AO from the opaque inputs.

The rewrite draws particles only in the FX colour pass after the opaque
scene, without depth writes, so `ParticleMaterial` refuses opaque pixel
lighting. Next step: confirm in an original capture with blood on screen
which passes draw a `particle/blood` renderer (expected: depth prepass with
motion vectors, then the lit opaque target), then draw opaque particle
renderers in the depth prepass and the opaque pass with the water fragment.

## Particle surfaces casting shadows

The FX world is itself a render model: FUN_1401fbad0 fills its surface list
(+0x1c00, one 0x80-byte record per renderer) and ends with the model's
surface-finishing virtual (+0xb8). A particle surface therefore reaches the
shadow views like any dynamic model surface, subject to its render entity's
flags and the material's `noshadow`. The 2026-10-28 eotw capture has no
particle draws in its shadow passes, but no stink bug or scorie emitter was
near that view. Next step: an original capture with a stink bug (eotw) in a
shadow-casting light, then the census of shadow-pass draws using
`ro_directinstancebuffer`; if present, add the proxy surfaces to the
rewrite's dynamic caster list with their instance records.

## Not gaps

World-material sprite renderers: the original publishes sprite and mesh
renderers as the same surface record, so a sprite with a world MAIN program
would draw through its material's passes. No installed content does this:
all 8,909 installed FX declarations (and every level's world FX) contain 0
sprites with world materials; the 56 world-material particle surfaces are all
meshes, which `particle_meshes` already draws.

## Stage-to-stage spawning (`m_spawnStage`), connected

`arkParentSpawnStage` sits at stage descriptor +0x200 (m_spawn +0x1d8;
defaults from 1f8b80). 140201760 links each enabled stage to the named
stage (+0x110). For every particle, 1f3f10 runs after 1dbbe0's shape
sampling: it evaluates m_lifeRestriction as a Catmull-Rom spline over key
values, lets 1f4930 probe the parent's slots four times with the parent's
seed for a live particle in that age window, then adds the parent's position
and its velocity scaled by m_initialVelocity and redirected through 1dcf60
by m_velocityRedirectionAngle. Without a match the emitter origin is added,
and m_spawnOnlyFromStage marks the particle dead by setting its age to
FLT_MAX. Installed data: 120 m_stageName references, 71 name a stage of
their declaration, 49 name stages that no longer exist (no link, ordinary
emission), none name their own stage.

## Particle attractors (`m_attractors`), connected

arkAttractorDesc (0xd88 bytes, defaults 1cd210/1cd520) is read whole. Each
stage keeps one state per attractor: 1cdff0 resets it at stage creation
with a copy of the stage seed, 1ce190 moves it once per simulation chunk
after the field preparations (position, velocity integral, random offset
that glides to a new random target every m_randomPeriod, cut-off, target
radius and strength), and the integration loop adds 1cda40's pull scaled by
m_strengthOverLife to the particle's accumulated velocity before each
substep. Installed data: 197 attractors in 111 particle declarations.

## Particle trails (`m_trails`): connected

Stage trails (arkParticleTrailDesc, 0x15b0 bytes, list at stage +0x1938;
183 in 134 installed declarations) and declaration trails (arkTrailDesc,
list at system +0x39a; 60 in 42 declarations). Stage trails are simulated
(gameplay::particles::trail), built into strips each frame within the
renderer budget and drawn through `particle_strip_vertex`; examples/
trail_smoke.rs drives all 82 loadable installed stage trails (vector-
oriented ones through 1402100f0/14020f710/1402104b0) for 120 frames without
errors; declaration trails (gameplay::particles::trail_system) run for
all 44 installed ones, including trail-only systems, over 180 frames
without errors. Not yet seen on screen. Known differences: TEXCOORD5 (left
unwritten by the original, zero here), declaration-trail velocity
inheritance (1401db7b0 is sampled into +0x94 but no trail code reads it, so
it is not ported), and the system's restart path does not reset
declaration trails. Trail strip bounds enter the system bounds (culling and
fog selection) from the last publication: the original merges stage trail
renderer bounds into the stage box (1401f1c40) and declaration trail bounds
right after their build in the same update (140200bf0), so declaration
trails lag one frame here; stage-local trail boxes are moved to world space
on their own rather than inside the stage box (approximate).

Descriptor (reflection, defaults 1401facf0): m_name +0, m_enabled +8 (1),
m_worldspace +9 (1), m_trailLife +0x10 (0.5), m_material +0xd8 (required:
1401fada0), m_flipBook +0xe0, m_mapping +0x1c0 (arkStripMapping: axis 0,
1.0, 0), m_spawn +0x1d0 (spawn_t: initialLocation +0, initialVelocity
+0x118, baseColor +0x230, initialColor +0x240, initialAlpha +0x350,
initialWidth +0x418, minimalEmitterMove +0x4e0, minimalParticleDistance
+0x4e4, interpolationDistance +0x4e8), m_offsetOverLife +0x6c0,
m_trailLifeCurves +0x7d8 and m_segmentLifeCurves +0xe30 (life_t:
velocityOverLife +0, acceleration_t +0x110 {value, worldspace +0x110,
linearAcceleration +0x118, gravity +0x1e0}, color +0x3b8, alpha +0x4c8,
width +0x590; 14020f0f0 trail defaults alpha keys 0/0.2/0.8/1 = 0,1,1,0;
14020ee80 segment defaults alpha 0/0.1/0.5/1 and gravity 1),
m_tesselation +0x1488 (segments 10, isLengthScaled true, maxSegments
256), m_orientation +0x1498 (isVectorOriented false, vector (0,0,1)).

Runtime map (particle variant of a template pair): renderer 1402107b0
(stage +0xd0 list; 0x168-byte record per particle slot, 140210980),
budget r_prt_particletrailrenderer_budget; pose 140211a90; spawn
1402117d0 -> 14020dd80 (14020dc10, 14020bec0, 14020dd00); per-substep
140211bc0 -> 14020dea0 -> 14020cfc0 (14020dff0, 140209de0, 14020ba40,
14020c620, 14020d7c0, 14020eb00); geometry 140210e90 -> 14020d5d0 ->
14020a720 (idTriangleDrawVerts<ushort>); surface record 140210e30 from
1401fbad0. The particle renderprog draws them through its
non-`$useDirectInstancing` branch (vertex texcoord4, color, texcoord5).

Trail record (0x168): +4 trail life (m_trailLife sample), +8 seed, +0xc
initial location (raw m_spawn.m_initialLocation value), +0x18 colour (baseColor x
initialColor, alpha x initialAlpha), +0x28 width, +0x2c flip bits
(14020dd00), +0x30 pose interpolator (1401d9xxx, same as EmitterPose),
+0xa0 time of last point, +0xa4 emitting, +0xa8/+0xac interpolation
distance and its square (FLT_MAX when <= 0), +0xb0 point times,
+0xc8 point positions, +0x108 per-point base velocity (m_initialVelocity),
+0x118 per-point accumulated velocity, +0x128 per-point orientation
(only with m_isVectorOriented), +0x138 trail time, +0x13c cycle fraction,
+0x140 flipbook frame, +0x144 seed, +0x148 length, +0x14c segment count,
+0x150..+0x164 bounds.
Per substep (14020cfc0): flipbook advance (14020eb00), drop points older
than trail life keeping one (14020d7c0, only with more than 3 points and
once the oldest exceeds 1.2x life? DAT_141ac8fd8), trail-life motion sample
(14020ba40), per-point motion (14020c620: segment-life velocity over life,
accelerations, gravity), new point when emitting (140209de0: needs
m_minimalEmitterMove since the last pose and m_minimalParticleDistance from
the last point; subdivides by m_interpolationDistance along the pose
interpolator), length and bounds by Catmull-Rom over 2n-3 samples
(14020dff0), segment count (14020d6f0: length x segments + 0.5, at least
1, capped by maxSegments when length scaled).
Strip (14020a720): segment count + 1 vertex pairs from max(time - life,
first point time) to the newest point; colour/alpha/width from trail-life
curves at the normalized age; centre and side from 140210240/1402100f0 and
14020f940/14020f710; vertices written by 140212f90 (40 bytes each),
strips joined by duplicated vertices; bounds grow by the widest width.
Trail vertex (140212f90, 40 bytes, pair per step): +0 position (centre -/+
width x side), +0xc half2 uv from the strip mapping (140208ef0: m_mapping
axis/mode/scale with flip bits, stepped by 140208eb0), +0x10 two halves
(mapping +0x28: half(0), half(random) from the seed), +0x14 packed
signed-byte normal (frame row 9..11), +0x18 packed tangent (mapping basis x
frame, sign in w), +0x1c packed colour (140164340), +0x20/+0x24 untouched.
Side frame (14020f940): forward = normalized centre-line tangent, view =
normalized (eye - centre), side = normalize(forward x view), normal =
side x forward; +0x30 |forward.view| feeds 14020f6f0's fade
min((1 - dot) x 20, 1) on width. Renderer sets vertex format 4 (+0x148)
with index size 2 (+0x14c).
Vertex format 4 (registry 0x1432d55a0, element list +0x178, built by
1402d0dc0; 8-byte records: format bits, semantic, index, offset, slot; D3D
layout by 1402aba10): POSITION0 f32x3 @0, TEXCOORD0/1 @0xc, TEXCOORD4
f16x4 @0xc, NORMAL snorm8x4 @0x14, TANGENT snorm8x4 @0x18, COLOR unorm8x4
@0x1c, TEXCOORD5 f16x4 @0x20 (trails leave it unwritten). The particle
renderprog's non-instanced vertex shaders read VERTEX_POSITION, COLOR,
TEXCOORD4 and, by permutation, NORMAL, TANGENT, TEXCOORD5 (installed
shader-cache census, examples/vertex_input_census.rs). Strips are drawn
from the shared index buffer 1402d1ae0(registry, 2).
Segment motion (14020c620) samples the trail-life gravity (+0xac8) at the
segment age; the segment-life gravity (+0x1120, default 1) is never read.

Declaration trails (arkTrailDesc, next): system list +0x39a, constructor
140211de0, emission 140212760 (rate from desc+0x28/+0x2c and curve +0x30
at the cycle time, cycle clock 1401d9b70/1401d9c20 with desc+0x1e0/0x1e4,
one record per spawn via 140212ae0 -> 14020de30/14020c270), burst
1402120f0, update 140212c70 (14020df10 returns false to retire a record),
strip 140212410 -> 14020b0b0 (the arkTrailDesc copy of 14020a720). The
shared record functions are the template twins of the stage-trail ones
(c270/bec0, d160/cfc0, a280/9de0, bc80/ba40, caf0/c620, b0b0/a720) and read
the arkTrailDesc offsets (spawn_t +0x3a8, etc.), which must be mapped per
call before reusing the stage-trail port.

## Particle beams (`m_beams`), connected

219 beams in 126 installed declarations (99 first failures). arkBeamDesc
(reflection): m_name +0, m_enabled +8, m_enableWind +9,
m_enableVelocityInheritance +0xa, m_worldspace +0xb, m_timings +0x10
(arkStandardCycles + warmup/delay/duration), m_material +0x1e8, m_flipBook
+0x1f0, m_mapping +0x2d0, m_spawn +0x2e0 (spawn_t: m_startPosition +0
position_t {initialLocation +0, initialVelocity +0x118, spawnMesh +0x230,
trackOrigin +0x580}, m_endPosition +0x588 (position_t + useTargetParameters
+0x588, useTargetAxis +0x589, trackTargetParameters +0x58a), baseColor
+0xb18, initialColor +0xb28, initialAlpha +0xc38, initialWidth +0xd00),
m_beamLifeCurves +0x10a8, m_startPointLifeCurves +0x1700,
m_endPointLifeCurves +0x1d58 (life_t as trails), m_controlPointsExtended
Parameters +0x23b0 (m_trackPositions +4), m_controlPoints +0x23b8
(arkControlPoint, 0xae8 bytes: initialPosition +0, radius +0x118, mode
+0x1e0, randomizePeriod +0x1e4, randomizePosition +0x1ec, radialVelocity
+0x1f0, inheritedVelocity +0x2b8, initialVelocity +0x380, lifeCurves
+0x490), m_reprojectControlPoints +0x23c8, m_tesselation +0x23cc, m_noise
+0x23d8 (perlin_t {mode, octaves, persistence, frequency}, frequency
+0x10, scale +0x14). Defaults 1401cfe10: two control points (1401d09c0),
tessellation 10/length scaled/256, perlin mode 0, 2 octaves, persistence
0.9, frequency 100, noise frequency 10, scale 0. Runtime: list +0x398,
constructor 1401d7be0 (material check 1401d0ff0), frame update 1401fde70
(1401d8b20, 1401d9180, 1401d97b0/1401d9630 -> 1401d5e00 beam update,
1401d5870, 1401d5390, 1401d6f70), strip 1401d8d50 -> 1401d32e0, bounds
1401d9080 merged into the system bounds after the build.

Beam emitter (1401d7be0, 0x1e8-byte object): same shape as the
declaration-trail emitter: seeds +0x10/+0x14 from the system seed, rate
accumulator +0x18 (FLT_MAX), delay, records of 0x198 bytes (at most 64,
free list descending), budget min(1402134d0, r_prt_beamrenderer_budget x
4096). Frame (1401fde70): 1401ceeb0 refreshes the target parameters
(+0xb0); delay 1401d8c70; clock-only 1401d9900; otherwise wind 1401d99f0
and inheritance 1401d9970, then 1/60 s / 4 % / cycle-end slices without a
pose interpolator: update live beams 1401d97b0, 1401ced10, then rate
1401d9180 or burst 1401d8b20 (spawn 1401d9630 = 1401d5870 then 1401d5e00);
after the slices the strip 1401d8d50 and bounds 1401d9080 into the system.
Beam record (1401d5870/1401d5e00): +0 point curve (start, control points,
end), +0x58 base velocities, +0x68 velocities, +0x78/+0x88 control-point
randomize timers, +0x98 normalized age (1401d5180's 1/life at +0x130),
+0x9c flipbook frame, +0xa0 time, +0xa4 and +0xb0 the owner pose of the
last update (14010ee70), +0xd4/+0xe0 the target pose, +0x104/+0x110 the
spawn-time end points (m_reprojectControlPoints, 1401d47b0), +0x11c colour,
width and flips (1401d5180), +0x160 length and +0x164 segments
(1401d52d0 after 1401d6f70), +0x180 bounds. Update order: age (retire past
1 unless infinite), flipbook (1401cfde0), start/end tracking of the owner
or target pose, wind on the points, reprojection, control-point
re-randomization (1401d21b0), point accelerations from beam/start/control/
end life curves (1401d30f0), velocities from velocityOverLife, length.

Beam spawn (1401d2a70): n = control points; the point curve gets n + 2
keys at times k/(n+1) (start, controls, end); base velocities (+0x58) zero
at both ends; start point from m_startPosition through 1401d0d60 (+ the
emitter position when world space); end point from m_endPosition through
1401d0d60, or with m_useTargetParameters from the target pose (+0x48 when
m_useTargetAxis) plus the target position; each control point placed by
1401d21b0(.., first = true); velocities (+0x68) zeroed; then every point's
inherited velocity (1401db630) is added to its base velocity.
Control point placement (1401d21b0, also at each m_randomizePeriod expiry
with first = false): timer reset; new period from the randomize range
(140001050) when both endpoints > 0, else 0. Anchor = lerp of the end and
start points at the key time (each component flushed below 1e-18), or the
target's per-point anchor (+0xc8) when the target supplies points. The
direction is three random signed units on the first placement, otherwise
the current offset from the anchor; on the first placement the position is
anchor + m_initialPosition (1401f6ca0) + direction * radius. RADIAL
CONSTRAINED: base velocity = (anchor + random unit direction * radius) -
position, divided by the period when positive (zero otherwise). FREE: base
velocity = unit direction * m_radialVelocity + lerp(start, end base
velocity) * m_inheritedVelocity + m_initialVelocity. Several scalar samples
lost their arguments in the decompile and need the disassembly.

Reprojection (1401d47b0, m_reprojectControlPoints, beams with 3+ points):
unless both end points stayed within 1.19e-7 of the stored ends, the
interior points are carried along by the rotation (14010e5e0, axis-angle
from the acos 1411afeb4 of the old and new end-to-end directions, skipped
below DAT_141ab3b04) plus the projection onto the new chord scaled by the
length ratio; base and accumulated velocities rotate with them; the ends
are then stored again. Point motion (1401d30f0) per point: acceleration =
life_t.m_acceleration value (rotated by the frame unless world space) +
(0, 0, gravity x scale x -9.8) + the beam-life wind/acceleration term,
plus the normalized current velocity times (linear acceleration + the
beam-life linear term). 1401d5e00 applies it as velocity += acceleration
x delta, then position += (velocity + base velocity x beam colour?
...velocityOverLife products) x delta for the start point, control points
(with the wind term) and end point. Flipbook rate 1401cfde0 = m_flipBook
rate (+0x208) sample x delta unless static (1401d0fe0: +0x1f8 type 0).

Beam strip (1401d8d50 -> 1401d32e0): the trail budget helpers again
(base 2 per join, 2n+2 per beam, factor from the renderer budget), bounds
merged per built beam, vertex count stored. Per beam: strip mapping
(140208ef0) with the record's length and flip bits; colour = beam-life
colour/alpha at the record age x the record colour x tint, width =
beam-life width x record width, and 1401d5390 fills the per-point colour
(+0x140) and width (+0x150) lists. segments + 1 samples at t += 1/segments:
Catmull-Rom centre (weights as 140210240) and tangent (1401d1500) over the
point curve, owner transform when local (1401d5ce0), frame 1401d4580 from
the view, colour by 1401d1390 and width by 1401d1c30 at the interval with
the same weights; when m_noise.m_scale > 1.19e-7, Perlin noise 14010e2e0
(octaves, persistence, perlin frequency, at noise frequency x record +0x14
and a running parameter advanced by 1/segments, x length in LENGHT mode)
offsets the centre along the frame's side by scale x noise x width; the
bounds grow by the widest point width x (1 + noise scale). Vertex pairs as
trails; TEXCOORD4.zw = (half(frame), half(random from +0x12c)). Strips
are joined by duplicated vertices before and after (param_9/param_10).
Still to read: 1401d5390, 1401d1390, 1401d1c30, 1401d4580, 1401d5ce0,
14010e2e0, 1401d6f70 (length).

Per-point colour and width (1401d5390, at the record age): start point =
m_startPointLifeCurves colour x beam colour, alpha x beam alpha, width x
beam width; each control point likewise from its own m_lifeCurves (alpha
and width as range x curve with two draws each); end point from
m_endPointLifeCurves. Strip interpolation: colour 1401d1390 (vec4
Catmull-Rom over the colour list, control fetch 1401d1e80), width
1401d1c30 (scalar control fetch with the usual end extrapolation). Beam
frame 1401d4580: unit tangent, side = normalize(tangent x (eye - centre))
written as (t.z(e.y-c.y) - t.y(e.z-c.z), ...), normal = side x tangent
normalized; no edge-on fade (unlike trails). Local beams: 1401d5ce0
transforms centre (with translation) and tangent by the owner matrix.
Perlin 14010e2e0: octave sum of 14010dff0 noise at (seed, frequency x t)
with amplitude persistence^k, each octave added as (1 - |sum|) x noise x
amplitude. Position placement 1401d0d60: location and velocity through
1401f6ca0, plus the spawn mesh sample (1401f9490) when it has a mesh.
Still to read: 14010dff0 (noise), 1401d1e80, 1401d6f70 (length).

Beam length (1401d6f70): bounds start from the first and last points; no
points gives 0; two points the straight distance (sqrt); otherwise 2n-3
Catmull-Rom samples (weights and P1 w1 + P0 w0 + P2 w2 + P3 w3 as
14020dff0) at t = k/(2n-1), k = 1..2n-3, accumulating (1/sqrt(max(d2,
FLT_MIN))) x d2 from the first point and finally to the last point,
growing the bounds with each sample. Segment count 1401d52d0: below 1e-4
both length and segments are zeroed; length scaled: max(1, floor(length x
segments + 0.5)) capped by maxSegments; otherwise floor(segments + 0.5).
Runtime: `gameplay::particles::beam` (records) and `beam_system`
(emitters), created before the declaration trails and simulated between
the stages and the trails, as 140201760 and the system frame order do;
strips share the trail vertex format, mapping and vertex pairs. The
1401d21b0 samples whose arguments the decompile dropped use the cycle
fraction and the emitter cycle seed (disassembly); its period draw is
140001050(min, max, f) = (1 - f) min + max f. The noise permutation window
is owned by the catalog (`Catalog::beam_noise`), loaded on the first
definition with a noisy beam and held by those definitions' systems.
`examples/beam_smoke.rs` drives all 86 loadable installed beams for three
seconds without failures or non-finite vertices. Not verified on screen.

FX target parameters (system +0x1d10): 140227b90 copies the FX record's
target (+0x80 position, +0x8c axis; 14021dc10 initializes the origin and
identity) into the system on every update. arkCpntFX writes the record
target: 1404b1d80 at creation and 1404b2200 -> 1404b20b0 every tick while
the target entity exists (1406dc560), using 1404b1880 (the target's world
pose, or its target tag/joint through 1404b19b0). The rewrite's world FX
host resolves the same pose through `WorldEffectFrames::resolve`; named
entity tags and joints still wait for animated model poses there. A beam
without a target aims at the origin, as the original's default record does.
The packet's target point list (+0x70), which 1401ce790 turns into
control-point anchors, has no writer in arkCpntFX and stays empty. No
installed beam position has a spawn mesh; one would be refused.

## Particle ribbons (`m_ribbons`), connected

38 declarations' first failure. arkRibbonDesc (0x13b0 bytes, declaration
list +0x5f8, system list +0x39c; enable check 1402042b0, constructor
1402042c0, frame update 1402007b0 between the trails and the end of the
system frame). Reflection: m_enabled +0, m_enableWind +1,
m_enableVelocityInheritance +2, m_worldspace +3, m_main +8 (m_name),
m_timings +0x10 (timings_t: m_cycles +0, m_warmup +4, m_delay +0xc,
m_duration +0x14, m_rate +0x20, m_life +0xe8), m_spawn +0x100 (spawner_t:
m_initialLocation +0, m_initialVelocity +0x118, m_spawnMesh +0x230,
m_velocityMesh +0x580, m_velocitySprayCone +0x7b8), m_velocityOverLife
+0x980, m_accelerationOverLife +0xa90 (acceleration_t: m_value +0,
m_worldspace +0x110, m_gravity +0x118), m_linearAccelerationOverLife
+0xc70, m_material +0xd38, m_flipBook +0xd40, m_mapping +0xe20,
m_baseColor +0xe2c, m_cycleLifeCurves +0xe40 and m_segmentLifeCurves
+0x10e0 (life_t: m_color +0, m_alpha +0x110, m_width +0x1d8),
m_section +0x1380 (m_startFromOrigin, m_minimumParticleLife +4,
m_maximumParticleLife +8), m_tesselation +0x138c (segments, length
scaled, max segments), m_noise +0x1398 (perlin_t {mode PERCENT/LENGTH,
octaves, persistence, frequency}, frequency +0x10, scale +0x14).

Runtime: one ribbon per emitter. Points of 0x34 bytes (+0x10 list):
position +0, age +0xc, base velocity +0x10, accumulated velocity +0x1c,
velocity +0x28. Constructor: two system seeds (+0x20, +0x24), flips and
initial flipbook frame from the system seed, 1/cycle duration (+0x64),
the point capacity 140203ae0, step limit min(1/60, 1/(0.001 x max rate x
2)) (+0x60), vertex budget 2 x maxSegments + 2. Frame (1402007b0): delay
1402063f0, clock-only 140208910, wind 1402089e0, inheritance 140208970,
then the usual 1/60 s / 4 % / cycle-end slices: points 1402086f0 (age by
1/cycle duration, flipbook 1401f7230, 140208270 per point, then points
past their life dropped from the front) and emission 140207d20 (rate as
1401d9180, spawn 140207f70: append or reuse the last point, spawn
position/velocity 140204080 from spawner_t, moved to world for world
ribbons, inherited velocity 1401db630, then one 140208270 step). Point
step 140208270: linear acceleration along the velocity, acceleration
(rotated unless world) with gravity x -9.8 on z, velocity over life times
the base velocity, wind over life, position += velocity x delta. Strip
140207230: 140204df0 builds the point curve, section clamps to
[m_minimumParticleLife, m_maximumParticleLife], segments 140206ed0,
140204950, build 140206420 (strip mapping 140208ef0, 1401d1390/1401d1c30
interpolation, 1401d5ce0, noise 14010e2e0, as beams).

Runtime: `assets::fx::particle_ribbon` (reader, defaults 140203b70 and
140204000) and `gameplay::particles::ribbon`, created after the
declaration trails and simulated after them, drawing through the trail
strip pipeline with the catalog's noise table. The point curve
(140204df0) keys an origin point (section m_startFromOrigin, on by
default) and the points from newest to oldest at their ages; every key's
colour and width use the same seed copy. 1402054c0 measures 2n - 2 samples
from the section start, never reaching the end. 140207230 grows the
strip bounds on the stack only, so ribbons add nothing to the system
bounds (original). 1c4b50's force-simulate flag now also covers beams,
declaration trails and ribbons. `examples/beam_smoke.rs` drives the 18
loadable installed ribbons: 17 draw, one needs the owner's published
motion for velocity inheritance. No installed ribbon has a spawn or
velocity mesh; one would be refused. Not verified on screen.

## Particle occlusion queries (`m_occlusionQuery`), connected

12 declarations' first failure (light glows: search lights, watchtower,
Wall of Light). occlusionQuery_t (declaration +0xe8, 0xd0 bytes):
m_enabled +0, m_radius +4, m_offset +8, m_queryVisibility +0x10 (size
curve +0x10 and alpha curve +0x70, keyed by the query result). Each frame
1401ffbc0 sets the system query visibility (+0x1d98) to 1, and when the
query is enabled and the system draws, stores the query sphere (system
position, radius, offset, averaging on) and sets +0x1d98 to the last
result (+0x1bcc); the size and alpha curves at that result scale the
system size (+0x1de0) and alpha. The sprite/mesh m_queryVisibility curves
(already read, `SpriteView::with_query_visibility`) take that value; the
rewrite still passes 1.

Renderer side (14031d7e0): one query manager of 256 slots (0x24 bytes:
result +4, radius +8, offset +0xc, position +0x10, state +0x20 (0 free,
2 pending, 1 issued), averaging +0x21/+0x22) allocated by 140292cc0 and
toggled by 140292f80; 140292fb0 fills a slot; 140292d10 returns the
result, the average of the last six (140292d80) when averaging. Draw
1402926d0: per issued slot, project the centre (view +0x7450 rows,
+0x73c0 for w), screen radius from the radius (or radius x 0.01 when
+0x22), result = samples passed (previous query, +0x24000 table) / (pi r^2
x viewport x 0.97 x scale) x 1.4 - 0.4, clamped to [0, 1]; then a GPU
occlusion query (140293030/140293160) around a draw of
`OcclusionQuery_1.material` (depth tested) or `OcclusionQuery_0.material`
with parameters `OcclusionQuery/Transform1` (radius, radius x aspect) and
`OcclusionQuery/Transform2` (projected centre). Installed:
generated/decls/material/occlusionquery_{0,1}.material.decl,
renderparm/occlusionquery/transform{1,2}.decl and renderprog/occlusionquery.
The query geometry is the shared unit quad (corners at +-1): 1402926d0's
normalization is the quad's pixel area (r^2 x width^2) x 0.97 x the
resolution scales, no pi. Program `renderprog/occlusionquery`: position
= x * Transform1.xy + y * Transform1.zw + Transform2.xy, z = Transform2.z;
blend ONE ZERO, no depth write, depth test GEQUAL (reversed Z), no cull,
colour only with DebugDraw (r_showOcclusionQuery). 1401ffbc0 also scales
the system's distance size (+0x1de0) and distance alpha (+0x1de4) by the
declaration's size/alpha curves at the result, before 1401ff460 culls.
The vendored DX12 backend now creates counting occlusion queries (wgpu-hal
PATCHES.md). Remaining: the declaration reader, the per-system query
state, the query manager and a depth-tested query pass with resolve and
delayed readback.

Runtime: `rendering::occlusion_query` (the 256-slot manager, shared by
systems and the pass; a system's slot is freed when it stops querying or
is dropped) and `bevy_backend::occlusion_query_pass` (depth-only unit
quads in counting queries against the scene depth, `assets/shaders/
dh2_occlusion_query.wgsl` ported from renderprog/occlusionquery, resolved
and read back asynchronously). Systems apply the result as 1401ffbc0 does:
the sprite/mesh query visibility and the distance size/alpha factors for
culling and drawing. bevy_backend deviations: each system requests and
reads its slot during its own update rather than in 14031d7e0's loop, and
counts arrive one or more frames later than the original's next-frame
read. r_showOcclusionQuery's debug colour draw is not connected. FX
census 844 -> 833. Not verified on screen.

## Particle FX events (`m_fxEvents`), mapped

96 installed declarations carry m_fxEvents (3 fail on it first; the rest
stop earlier). Declaration +0x608: arkFXEventDesc entries (0x58 bytes:
m_event +0 atom, m_enabled +8, m_interval +0xc, m_fx, axis mapping with
m_axisMode +0x18, m_velocityAxis, m_reverseVelocityAxis, m_viewAxis,
m_reverseViewAxis, offset/rotation). 140201760 creates an
arkFXEventInstance (1401da2c0) per enabled entry with an FX (1401da0d0)
only when the system has an FX owner (+0x1c78); 1401daf50 advances each
instance's interval timer; 1401fc020 hands the stage events collected at
system +0x1cf0 to 1401da820, which matches names, orients the spawn from
the event velocity/view axes (1401da330) and asks the owner to spawn the
FX. The rewrite needs a runtime FX spawn request from particle systems
to its world FX owner before this can be ported.

## Havok particle stages (`m_havokStages`), mapping

682 declarations' first failure (debris, chunks, rolling stones,
breakables, leaves). arkHavokParticleStageDesc (0x1d20 bytes, declaration
list +0x5c8, system list +0x396; constructor 1401dfc20, frame update
1401fef80). Reflection: m_enabled +0, m_enableWind +1,
m_enableVelocityInheritance +2, m_main +8 (m_name, m_boundsMargins +8,
m_enableAngularMotion +0x20 of main_t... see registration),
m_enableCollisions +0x20, m_collisions +0x28 (arkCollisionMaterial
+0x28 within, m_accuracy +0x410, m_layer +0x414), m_cycleSimulation +0x440
(simulation_t: m_gravity, m_linearDamping, m_linearClamping,
m_angularDamping, m_angularClamping, 0xc8 each), m_spawn +0x828 (spawn_t,
0x1088: the standard cycle fields, m_delay +0x1c0, m_duration +0x1c8,
m_useEventLocation +0x1d0, m_eventVelocity +0x1d8, ...),
m_spawnCollisions +0x18b0, m_wind +0x18c8 (m_dragOverCycle,
m_dragOverLife), m_vectorFields +0x1a58, m_accelerationVectorFields
+0x1a68, m_attractors +0x1a78, m_accelerationOverLife +0x1a88,
m_linearAccelerationOverLife +0x1ba0, m_events +0x1c68 (m_birth,
m_collision, m_oceanCollision, m_stop, m_death, m_life),
m_sorting +0x1cc8, m_constantSeed +0x1ccc, m_sprites +0x1cd0, m_meshes
+0x1ce0, m_trails +0x1cf0, m_collisionConvexMeshOffset +0x1d10,
m_maxBoundingRadius +0x1d1c. The sprite, mesh and trail renderers are the
ordinary stage ones.

Runtime: 1401fef80 is the stage frame (emission 1401e6570 with the
standard spawn helpers, particle update 1401e83f0 with the sprite and
trail advances, bounds widened by the collision radius x distance size).
The particles themselves are simulated by arkHavokParticleStageSimulator
(constructor at chunk_000086 2886), an arkPhysicsFXPipeline (14008b560,
base 140ff1be0) registered with the physics world (+0x148338), whose
collision shape 1401df6a0 (UpdateCollisionShape) is a convex hull of the
particle mesh (140b88xxx hull builder) recentred on its centroid.
arkHavokParticleSimulatorEffect (1401db280) feeds per-particle effects.
The pipeline is Havok FX (hkfx, the Arkane 2016_1_0 drop: hkfxPipeline,
hkfxBodyAllocator, hkfxPipelines::Debris/Particles, hkfxEffects::
LinearMotion, AngularMotion, WorldCollide, PhysicsCollide, SolveContacts,
SolveConvexContacts, ...). The simulator's secondary interfaces are its
callbacks: 1401e6240 (+0x118 vtable) records each world contact (0x24
bytes) for the collision events, 1401e63f0 (+0x110) runs after each step
(1401e7d40: bounds, contact and stop/death events, freeing bodies through
the hkfx allocator). Porting the stages means porting the hkfx Debris
pipeline and its effects against the rewrite's hknp world: a middleware
port of its own, not a rendering path.

### Havok FX pipeline, mapped for the port

Descriptor defaults: 1401dd300 (idList element constructor, called by
1409b????'s resize at chunk_000483:5008) then 1401de940: m_enabled 1,
m_enableCollisions 1 (so 1073 of the 1075 installed stages collide),
m_boundsMargins 0.1, curves at +0x50/+0x118/+0x1e0 = 0, +0x2a8/+0x370 = 12,
simulation_t gravity 1, linearDamping 0, linearClamping 0x5f7ffff0,
angularDamping 0, angularClamping 0x5f7ffff0. Reflection (chunk_001017
3060-3900): main_t {m_name +0, m_boundsMargins +8}; collisions_t
{m_collisionCounter +0, m_killOnCollision +8, m_stopOnCollision +9,
m_sleepOnCollision +0xa, m_killOnOceanCollision +0xb, m_selfCollisions
+0xc, m_mesh +0x10, m_boundingBox +0x18, m_radius +0x1c,
m_enableAngularMotion +0x20, m_material +0x28 {m_friction, m_restitution,
m_stickiness, m_restitutionThreshold, m_frictionThreshold, 0xc8 each},
m_accuracy +0x410, m_layer +0x414}; spawn_t also holds
m_physicsShapesSpawn +0x598, m_initialAngularOrientation +0xd90,
m_initialAngularVelocity +0xea8.

Simulator (1401dfc20, arkHavokParticleStageSimulator : arkPhysicsFXPipeline
: hkfxPipeline). Effects added in order (140ff2750): LinearMotion (inline,
always), AngularMotion (mesh with angular motion, or +0x48), arkVectorFields
(vector/acceleration fields), arkAttractors, arkAccelerations (acceleration
over life not constant), arkWind (system wind and m_enableWind); with
collisions: SpatialPartition (inline), PhysicsCollide, SolveSphereContacts
(no convex mesh) or SolveConvexContacts (140fbda60 with the hull), and
SolveSelfContacts when m_selfCollisions; Integrate (inline) last.
Registration: 1400957a0 (world +0x148348, stepped by 1400908c0 through
140ff71c0 with the pipeline's own timestep), 140095840 (post-step listener).

Step (CpuBackend slot 3, 140ff4380): bookkeeping 140ff2c10/140ff2620/
140ff2730/140ff2f40; every enabled effect's slot 15 (begin, dt); stage
lists of effects with type 1 (unary) or 2 (compute) and stage count >
s; consecutive unary effects run together per body chunk (prepare slot 18,
per chunk each effect's slot 19 in order, finish slot 20); a compute
effect runs its slot 17; every enabled effect's slot 16 (end); step count
+0x38, time +0x3c, post-step signal +0xb8+8. Types: LinearMotion and
AngularMotion 0 (channel data only), the rest 1 with one stage,
SolveSelfContacts 2. Per body the order is therefore fields, attractors,
accelerations, wind, spatial partition, collide, contact solve; then self
contacts; then Integrate.

LinearMotion: 0x20 bytes per body, channels "LinearMotion/PositionAndAge"
and "VelocityAndScale"; allocation 140ff8720 writes (0,0,0,0),(0,0,0,1).
Integrate (140ff89d0; fields gravity +0x30, linearDamping +0x40,
linearClamping +0x44, angularDamping +0x48, angularClamping +0x4c, maxAge
+0x50, rmaFrequency +0x54, outOfDomainHandling +0x58 (CLIP 0, TRASH 1),
secondOrderAccuracy +0x5c, aging rate +0x60; per-body 8 bytes +0x78 of two
packed vectors): slot 19 140ff8cd0 -> 6-DoF 140ff95a0 with AngularMotion,
else 3-DoF 140ff9fd0: bodies older than maxAge are trashed (140ff2550);
outside the pipeline domain (+0x70/+0x80, +-4096 around the origin) they
are clipped (velocity zeroed, ageing continues) or trashed; inside, the
velocity's running average is packed, the position advances by v dt
(second-order variant with the previous packed velocity), age += rate dt,
velocity = v pow-damped + projected gravity dt, clamped to the linear
clamp, and the optional plane lock (+0x90) is applied.

Port status (`gameplay::havok_fx`): reader `assets::fx::particle_havok_stage`
(all but the m_physicsShapesSpawn stages load); allocator, pipeline
allocate/trash/flush, LinearMotion/AngularMotion channels, packed velocity
history and Integrate 3-DoF/6-DoF are ported. 6-DoF (140ff95a0) per body:
plane projection; angular step w kept along the plane normal, q' =
normalize(q + (dt/2 w) q), w x pow(1 - angularDamping, dt), clamped to
angularClamping or pi/dt; then the 3-DoF linear step; a clipped body also
loses w. Remaining, in order:
- Simulator glue (1401dfc20): spawn 1401e7040 -> pending records +0x3e8
  (0x60 bytes: position, life, velocity, seed, colour, cycle, quaternion
  +0x40, angular velocity +0x50); post-step 1401e7d40: 1401e41e0 allocates
  them (140ff2200 with stealing; 1/x at +0x330; renderer births 1401ee5f0/
  1401eb4c0/1402117d0), 1401e78b0 samples simulation_t into Integrate
  (world gravity +0x970 x gravity curve; damping clamped to [0,1], clamps
  to [0, 0x5f7ffff0], angular clamp x 14228a75c) and the contact material
  (1401e1d10), then per allocated body: collision reactions (kill/stop/
  sleep, counter, events; solver +0x50 per-body contact count), normalized
  age (infinite life wraps), render state +0x310, velocity +0x320, ocean
  check, live list +0x360 or dead list +0x3d0; death events; 1401e3ff0
  frees the dead.
- ark effects (unary slot 19): arkWind 140216900, arkAccelerations
  1401ccb30, arkAttractors 1401ce600, arkVectorFields 1402165f0.
- Collisions: SpatialPartition (begin 140fd72f0, end 140fd75f0; empty per
  body), PhysicsCollide (begin 140fbf850 "Get new dynamic hits" queries the
  broadphase with the partition root bounds; per body 140fc1370, 11k
  decompiled lines, calls 140cada50 closest-point preparation and
  140c85490; end 140fc01d0), SolveSphereContacts (slot 19 140fb9c20),
  SolveConvexContacts (140fbb350, hull from 1401df6a0), SolveSelfContacts
  (compute slot 17 140fd33d0).
- Connected (`gameplay::particles::havok_stage`): emission, spawn records,
  pipeline step with Integrate every frame (the physics world steps it
  whether or not the system is simulated), post-step, sprite/mesh births
  and updates, draws through `rendering::particles::publication::DrawStage`
  with the AngularMotion basis, system bounds. Refused by name until
  ported: collisions (482 declarations), collision hulls, fields,
  attractors, accelerations, wind, trails, events, spawn queries, ocean
  kills. Havok sprites with world materials (program arkdefault, 73) are a
  separate draw gap.
- PhysicsCollide begin (140fbf850, "Get new dynamic hits"): broadphase
  query (140c894b0) with the partition root bounds for dynamic world
  bodies (flags 2 and 8); each body's world AABB (140c85490) queries the
  particle partition (140fd8390) and, when the body's speed (140c85980) is
  below +0xac, its overlapping particles (inside the swept particle AABB
  grown by the pipeline radius x 2) become dynamic neighbour entries in the
  +0x128 structure. Static geometry is handled per body (140fc1370: "Static
  CD" with hkfxCacheSharing and "Static cache limiting", "Dynamic CD", "CD
  pass", "CCD pass").
- WorldCollide (PhysicsCollide's base, constructor 140ffb110, class id
  0x68 the solvers look up): per body a static and a dynamic contact,
  channels "WorldCollide/StaticContact/Plane", "/LinearVelocity",
  "/AngularVelocity", "/COM" (0x40-byte records at +0x30, dynamic at
  +0x50) and two floats each (+0x40 / +0x60, -1 = none); allocation
  140ffb2e0 zeroes the records and sets the floats to -1. The solvers
  (SolveSphereContacts 140fb9c20) blend whichever of the two exist.
- PhysicsCollide per body (140fc1370): the game's configuration takes the
  first CD variant (+0x70 short 0; lines ~1199-5300 of the decompile) then
  the CCD pass (~9514-end). Static contacts come from hkfxClosestPoints
  queries against the world (140fe3be0 -> world+0x478 query interface slot
  +0xb0) with a per-body cache (hkfxCacheSharing, static cache limiting
  140fd1880/140ffbed0); dynamic contacts from the neighbour entries and
  140cada50 closest points; the CCD pass casts rays through slot +0x90
  (hkfxShapeTagCodec 140fd1e30). The rewrite's player answers the same
  world+0x478 queries geometrically against `navigation::CollisionWorld`
  (`app::player::GameCollision`).
  Each contact's two floats are its friction and restitution: the
  particle's solver material combined with the hit shape's hknp material
  (world material library +0x988, 0x70-byte entries: half-precision
  dynamic/static friction +0x32/+0x34 (static when the contact counter is
  below 3), restitution +0x36, combine policies +0x38/+0x39 = geometric
  mean, min, max, average; a missing or negative-weight material keeps the
  particle's own values). Static plane = hit normal and -(n . point);
  140fc1370:3588-3640, dynamic at 3103.
  Section map of 140fc1370 (variant A): Dynamic CD ~1353-3320, Static CD
  3329-5224 split by the collide accuracy (+0x28, 1401df5a0: LOW 2,
  MEDIUM/HIGH 1, GROUND_TRUTH 4; installed stages use 1 and 166 use 4):
  mode 4 (3383-3680) re-queries every step, mode 1 (3681-5224) goes through
  the neighbour cache and static cache limiting; CCD pass ~9514-11227.
  Mode 4 per body: displacement d = |dt v|, query distance
  clamp(d x +0x9c + r x +0x98 + r, +0x7c, +0x80) with r the body scale x
  pipeline radius (+0x34; per-body partition sizes when the partition
  keeps them); the cache entry is reset and its AABB centre/size written;
  140fe3be0 queries closest points of the particle shape (+0xf8) at the
  body position with filter (+0x128 +0xc0..+0xd8); the closest hit becomes
  the static plane (normal, -(n . p)), the hit body/shape key is cached
  and the material combination fills the friction/restitution pair.
  Port route: sphere particles query the published physics snapshot
  (`world_point_query::PointQuerySnapshot`) with the ported point query
  (`WorldPointQuery::hits_with_distance`, signed narrowphase distances),
  which needs the query distance passed to its broadphase visit (it uses 0
  today). Convex-hull particles (678 installed stages) need a hull versus
  world closest-point query that the rewrite does not have yet. The
  pipeline step runs in `ParticleSystem::update`, which does not receive
  the snapshot; it has to be handed in there.
  Revised route: the retained query geometry covers compounds and convex
  polytopes but not hknpCompressedMeshShape (level geometry), so static
  sphere queries run the ported GSK point kernel
  (`convex_support::distance::query_point`, 140d10960: closest point,
  normal, distance with query radius and threshold) on each nearby
  triangle of the level `navigation::CollisionWorld` (view coordinates;
  the player's `GameCollision` converts). The closest hit wins as in
  hkfxClosestHitCollector. Contact material: friction is the hit hknp
  material's dynamic friction (half +0x32), or its static friction (+0x34)
  when the solver state is at least 3 and static exceeds dynamic, x 2.4
  (141af7790); restitution +0x36; still to trace: the arkPhysicsMaterial
  -> hknpMaterial conversion that sets the combine policies (+0x38/+0x39)
  and the +0x3c weight. Mode 4 then projects the cached plane off the
  pipeline's plane lock (+0x90) into the static contact; static contacts
  keep zero velocities and centre of mass.
- Connected collisions (`havok_fx::physics_collide`, `solve_contacts`,
  `particles::havok_stage::Collisions`): per step, mode 4 static CD for
  every body, SolveSphereContacts, Integrate, then the CCD pass's static
  clamp (9537-9778: a body integrated beyond the sphere of its last query,
  R' = max(R - r/2, 0) without a hit or when R < the cached plane distance
  of the query centre, else max(0.7 x that distance, max(1.19e-7, R)), is
  put back on that sphere; skipped for solver state 0x80). The query runs
  on the level map in view axes (game (x, y, z) -> (x, z, -y)). Post-step
  reactions (1401e7d40): with kill/stop/sleep, a collision event or a
  collision counter, a body with solver state > 0 reacts; at states up to
  3 (kill) or 2 it first fires the collision event and decrements its
  counter (+0x350, drawn at birth in [min, max] from the spawn seed, sized
  only when both bounds are non-zero), reacting only when it reaches 0;
  kill kills, stop zeroes linear and angular velocity (stop event below
  state 3) and sleep sets state 0x7f. Census 765 -> 752.
  Not ported: collide modes 1/2 (accuracy LOW/MEDIUM/HIGH, 467
  declarations: cached static CD 3681-5224 with hkfxCacheSharing and
  static cache limiting), the dynamic CD and the CCD's dynamic part
  (moving world bodies: the map holds only static triangles, so moving
  doors and props give no contacts), hulls and SolveConvexContacts,
  SolveSelfContacts, collision/stop events.
- Ported (`physics_collide::static_cached`): modes 1 and 2 for sphere
  bodies, with the collector (140fe31e0) keeping the nearest and
  second-nearest leaf distance (a compressed-mesh quad is one leaf: the map
  links its two triangles, `CollisionMap::primitive`); the safe radius is
  the second-nearest distance, limited for a face contact (|f|^2 - (n.f)^2
  <= 1e-4 |f|^2) to the nearest edge line (RCPPS, no refinement), else the
  plane gap; speeds in the reuse test use raw RSQRTPS. Cache sharing has no
  effect on a static-only world (a static hit sets flag 0x20, which the
  sharing filter rejects, and an empty cache matches only bodies with a
  dynamic neighbour), nor do the moved set and static cache limiting (the
  +0xd8 list of moving body AABBs, 140fd1880). `havok_fx::spatial_partition`
  ports the partition's list order (sphere bounds 140fd86b0, transfers,
  hkcdSimdTree mode-0 build order 140c1fb80) for when moving bodies exist.
  Census 752 -> 670.
- Havok trails (connected): one renderer per enabled trail with a
  material (1401fada0, 1402107b0, no world transform); births 1402117d0 at
  the spawn record position with the identity basis after the mesh
  births on the same seed chain; updates 140211bc0 per live body at its
  render position and normalized age with the system fraction; drawn
  (1401e5980) over the post-step live list. The inherited-velocity
  argument (+0x63c) the simulator passes is unused by the trail code
  (14020dd80, 140209de0). Census 670 -> 530.
- Measured in the running original (2026-11-14, break after the mode-1
  query at 0x140fcdcb1): the static hit distance is the centre-to-surface
  distance |p - q| of the query point (collector +0x50; second best +0xb0
  as decoded), so the port's point query is the original's.
- Hull build ported (`havok_fx::convex_hull`): bit-exact, vertex order and
  centroid included, against 25 hulls read from the running original's
  descriptors (`examples/hull_oracle.rs`); the other 41 need the decimation.
- Collision hulls ported (`havok_fx::convex_hull::collision_shape`): the
  3D build, buildIndices faces, the quadric decimation with its OBB-based
  tolerance (140c0d380 3D path, planar build 140b884e0), ensureContainment
  and the centroid reproduce all 66 hulls read from the running original
  bit-exactly (tests/fixtures/particle_collision_hulls.txt, checked from
  the installed models by examples/hull_oracle.rs). The 4x4 inverse was
  generated from 140adf2d0's instructions by a symbolic SSE evaluator;
  both hull inverses match the installed functions on 800 probe cases.
- Convex contacts, next: SolveConvexContacts (140fbaf60) keeps one 0x120
  shape record per stage (140fbdea0: at most 16 hull vertices re-hulled,
  m_boundingBox replacing them by the 8 OBB corners, recentred on the new
  centroid; +0x100 inscribed radius max(1e-5, min face distance), +0x104
  outer radius max(inner, max |v|)); 140fbdc80 then sets the pipeline's
  +0x30 to the inscribed and +0x34 to the outer radius (sphere stages
  keep m_radius in both). The per-body solve 140fbb350 (2643 lines) reads
  both WorldCollide contacts, the AngularMotion orientation, and re-queries
  the hit bodies with the hull (140ffaaf0 / 140ffae70: convex closest
  points and casts through the world).
- Collision hulls (227 declarations), decode notes: 1401df6a0 builds the hull of
  m_mesh's particle-model vertices with hkgpConvexHull (build 140b88330 ->
  140baba50 on a quantized domain 140b9c1e0; when the hull has more than 16
  vertices, decimate 140b896a0(hull, 0x5f7ffff0, 16), 140b9ab30, decimate
  again; then mass properties 140b900b0/140b90100 and the centroid
  140b91880, which is subtracted from the vertices (+0x1d00) and stored at
  +0x1d10; failure aborts with "UpdateCollisionShape() failed"). Of the 165
  distinct collision meshes most have more than 16 distinct vertices, so
  the decimation is on the path for nearly all. Then the convex collide
  (the flag-4 hull path of 140fc1370, face planes of the hit polytope) and
  SolveConvexContacts (140fbda60 setup, slot 19 140fbb350).
- Mode 1 (cached static CD, 140fc1370:3681-5224), decoded for the port.
  Tuning (140fbea10 at +0x7c, never changed by the simulator): +0x7c min
  0, +0x80 max FLT_MAX, +0x84 FLT_MAX, +0x88 0, +0x8c 0.01, +0x90 8 cache
  sharing neighbours, +0x94 8 recent bodies, +0x98 10, +0x9c 10, +0xa0 4,
  +0xa4 1 (query budget; only < 1 limits queries), +0xa8 20, +0xac FLT_MAX,
  +0xb0 0. Per body: a cached hit (entry +0x128[0]: hknp body id, material
  key, flags) whose body is gone or in the moved set (+0xd8) wakes the
  body (state 2); asleep bodies (0x80) with a valid cache skip the pass.
  The cache sphere (+0x128[2]: centre, R) is reused while the body and its
  predicted position stay inside max(1.19e-7, R - r); otherwise neighbour
  caches are tried (cache sharing over the SpatialPartition's sorted list,
  +-1, +-2, ... up to +0x90 entries, or the last +0x94 bodies of the chunk
  when the body is not in the list) and only then the world is queried.
  A query hit stores the plane (n, -(n . p)), the hit id and key, and R:
  the query distance limited, for a triangle or quad face contact, to the
  distance from the body to the face's edge lines, and for convex leaves
  to their face planes. The contact plane comes from the cache every step;
  the material is recombined only when the hit changes. Static cache
  limiting (140fd1880, when +0xd8 changed) shrinks R away from moving
  world bodies. Needs, in order: SpatialPartition (begin 140fd72f0 pair
  pruning; end 140fd75f0 -> 140fd8d00 Initialize/Transfer/Prepare/Rebuild
  and Collide/Update closest neighbor over an hkcdDynamicTree, then group
  ids), quads kept whole in the collision map (the map splits them; the
  edge limit uses the quad's four edges), and the hit's hknp body id.
- Draw integration: the Havok stage reuses the ordinary emitter
  (1401e6570 matches `ParticleEmitter::continuous`: 0.1 minimum rate,
  1/60 or 1/(2 x max rate) slices) and the sprite/mesh renderer births and
  updates (1401ee5f0/1401eeea0, 1401eb4c0/1401ebc50, trails 1402117d0/
  140211bc0) over the live list +0x360 with the render state +0x310
  (position, normalized age) and seeds +0x340. Mesh orientation SIMULATED
  (1146 installed meshes) is the emitter basis for ordinary stages; for
  Havok bodies 1401e6f50 hands the renderers (1401ebc00) the system's
  +0x8c..+0xa0 and +0xc0 (LOD indices only, 1401ebc00). The Havok draw
  publication is its own: 1401e4930 (radix-sorted by 1401dd220/1401dcbf0
  when m_sorting is set and two or more bodies live) calls, per renderer,
  sprites 1401e5760/1401e5870 -> 1401e5440 and meshes 1401e5290/1401e5360
  -> 1401e4f50: each live body is drawn with the AngularMotion quaternion
  as its basis (140aa0390) when present, and meshes at the body position
  minus R x the hull centroid offset (1401df200); trails 1401e5980/
  1401e5ac0.

