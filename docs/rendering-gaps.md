# Rendering gaps

Rendering paths the rewrite does not draw yet, ranked by how much installed
content they block, with what is known and the next step. Remove an entry
when it is implemented.

Program census (2026-11-10): `mainProgram` of every installed material
declaration. Particle programs connected: `particle/add_default`,
`dist_blur`, `scrolling_water`, `liquid`, `dissolve`, `add_liquid`, `water`.

| Gap | Installed content | Status |
|---|---|---|
| `arkhair` | 134 materials | none: character hair program; needed once characters render |
| `particle/blood` | 19 materials (combat blood, camera blood, eotw fish blood) | blocked: see below |
| `arkeyeball` | 23 materials | none: character eye program |
| Particle surfaces casting shadows | eotw stink bugs (`bug01`), `dust_scorie` | blocked: see below |
| World FX target/tag pose producer | eotw world FX | none: `world_particle_state` refuses targeted FX |
| `arkKisculeNodeEntitySpawnerControl` | eotw outsider shrine FX | none |
| `particle/fire`, `particle/filing_container` | 2 and 8 materials | none |

## Particle declaration census (2026-11-10)

All 8,909 installed FX declarations loaded through `WorldFxSources`
(local census `examples/fx_failure_census.rs`; `examples/` is not tracked): effects=8909 failed_effects=10 particles=2171 failed_particles=1233.
Each particle declaration fails at its first unsupported field, so counts
below a top reason can hide further gaps. Since this table: flipbook and
radial-ramp opacity, bare rotation orders, clipping planes, stage
sorting, stage-to-stage spawning, attractors, stage trails and
declaration trails are connected; 954 particle declarations still fail.

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

## Particle beams (`m_beams`), next

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
