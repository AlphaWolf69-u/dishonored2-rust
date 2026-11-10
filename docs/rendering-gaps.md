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
sorting, stage-to-stage spawning and attractors are connected; 1,054
particle declarations still fail.

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

## Particle trails (`m_trails`), in progress

Stage trails (arkParticleTrailDesc, 0x15b0 bytes, list at stage +0x1938;
183 in 134 installed declarations) and declaration trails (arkTrailDesc,
list at system +0x39a; 60 in 42 declarations). Both still rejected by the
reader until simulation, strip building and drawing all exist.

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
