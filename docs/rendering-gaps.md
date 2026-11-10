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
| World-material sprite renderers | Dust District leaves (`arkVegetation` sprites) | none: `ParticleMaterial` refuses non-particle programs, failing the whole FX; meshes with world materials already draw through `particle_meshes` |
| World FX target/tag pose producer | eotw world FX | none: `world_particle_state` refuses targeted FX |
| `arkKisculeNodeEntitySpawnerControl` | eotw outsider shrine FX | none |
| `particle/fire`, `particle/filing_container` | 2 and 8 materials | none |

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
