# Suspected defects in the original game

Behaviour of the original executable or shaders that looks unintended. The
rewrite reproduces each one faithfully (parity first); this list is for a
later, deliberate fix pass. Each entry names the evidence and what a fix
would change, so it can be made opt-in without guessing.

## Rendering

### Vegetation sun shadow: "thickness" distance is meaningless (arkVegetation)
- Source: `BRDF_vegetation` (`generated/decls/renderprog/brdf_vegetation.decl`)
  attenuates shadowed light by `exp(-transmittanceParms.w * isBehindCaster *
  distance(receiver, caster))`, where the caster position comes from
  `GetShadowMapZandPos` (`translucency.decl`).
- Defect: `lighting.renderprog` sets `m_ShadowSetup = 0` for the parallel
  light but never assigns `light.m_shadowInfo`. The compiled leaf shader
  (eotw capture, event 54031) uses the cascade's `cb_shadowprojections` rows
  for both world-to-shadow and shadow-to-world, so the "caster" is the forward
  matrix applied to (u, v, mapZ). The measured distance at a leaf was 164.7
  units, so any leaf behind a caster gets `exp(-82) = 0`: a hard on/off test
  instead of the intended soft attenuation with depth into the crown.
- The same term for shadowed local lights uses each record's real
  `mShadowToWorldMat` (same shader, instructions 786-814), producing a true
  distance; only the parallel light loses its inverse.
- Fix: pass the cascade's real inverse as `mShadowToWorldMat`.
- Rewrite: `parallel_thickness_visibility` in `assets/shaders/dh2_parallel_receiver.wgsl`.

### Vegetation sun shadow: no bias, bilinear depth (arkVegetation)
- The same term reads the shadow map with `$bilinearSampler` and compares
  `shadowCoord.z` with the filtered depth without any bias, so a leaf is tested
  against an average including its own depth: speckled self-shadowing.
- The second `GetShadowMapZandPos` overload (same file) uses `$pointSampler`
  and a 0.005 normal offset, which suggests the first one's sampling is not
  deliberate.
- Fix: point sampling plus the shadow bias the PCF path uses.

### Vegetation sun shadow: PCF only gates the light (arkVegetation)
- `ComputeLighting` proceeds when `m_shadow > 0` (PCF times cloud shadow) but
  then replaces it with the thickness term, so a partly shadowed leaf is not
  dimmed by its PCF value.
- Fix: multiply the thickness term by `m_shadow`.

### Vegetation area-light translucency reflectance (arkVegetation, translucent variant)
- `ComputeLighting` uses `Rho_t = 1 - (1 - F) * DiffuseColor`, the comment's
  "complement of the diffuse reflectance". `ComputeLightingArea` computes
  `Rho_t = INVPI * (1 - (1 - F)) * diffuseAlbedo`, i.e. `F * albedo / pi`,
  which is not that complement and differs from the point-light term.
- Status: not yet reproduced (translucent vegetation is unported; only the
  two Dust District root sprite materials enable translucency).

### Opaque soft-contact particles discard themselves (particle/blood, particle/liquid)
- Source: the `particle/blood` and `particle/liquid` colour programs multiply
  alpha by `SoftContact` before `ClipAlpha` unless `$isZPrePass`. For a
  non-isAlpha material the colour program reads `ro_fx_depthfull`, which is
  copied (Karnaca fish-market capture, event 5051) after the zPrePass in
  which the same particle wrote its own depth (events 4383-4425).
- Defect: the scene depth equals the particle's depth, so the soft-contact
  factor is about zero and every lit-pass pixel is discarded. The world
  behind already failed against the particle's prepass depth, so the lit
  target keeps its cleared zero there: measured 0 at all sampled blood-drop
  pixels after events 69112-69154, while their neighbours are lit. Blood
  drops therefore appear as black (later fogged) silhouettes.
- Fix: read soft contact from a depth copy taken before the opaque particle
  zPrePass, or disable soft contact for non-isAlpha particle materials.
- Rewrite: `render_opaque_particles` in
  `src/rendering/bevy_backend/camera_scene.rs` uses the fog's opaque-depth
  snapshot, recorded after the particle zPrePass, as the original does.

### Moving FX systems keep stale shadows in cached local-light faces
- Source: FX particle systems are static-bucket model casters (201480;
  no FX path sets +0x18bc bit1). Local-light faces cache their static
  layer keyed only by the light pose (2cd500 tolerance), and the cache is
  invalidated only on dynamic-caster transitions (523af0, StartEntity /
  StopEntity).
- Defect: a moving particle system (stink bugs, falling leaves) is drawn
  into a cached face once, when the entry is built, and its local-light
  shadow stays there while the particles move on.
- Fix: mark FX systems as dynamic casters, or invalidate model entries when
  a static owner's caster record changes.
- Rewrite: `LocalStaticInputs::particles` in
  `src/app/local_shadow_updates.rs` snapshots the surfaces with the entry.

## Engine

### Image format normalization reads an uninitialized local
- `FUN_1402c2ef0`: on a successful conversion that writes no mapped format,
  the result is read from an uninitialized stack variable.
- Rewrite: `normalize_image_format` in `src/assets/texture.rs` rejects that
  branch instead of reading garbage.

### Lean recentering timer is never reset
- The lean state's recenter timer starts at Activate and is never reset, so
  the 0.1 s recentering delay applies only after the first activation.
- Rewrite: `src/gameplay/player/states.rs` (lean update) keeps it.
- Needs confirmation in the running game before treating it as a defect.

### Particle attractors never read m_velocityOverCycle
- `FUN_1401cde40` samples the attractor's `m_velocity` (+0x230) twice, once
  at the attractor's time and once at the cycle fraction (confirmed in the
  disassembly: both calls use `lea rcx, [rbp+0x230]`); `m_velocityOverCycle`
  (+0x348) is parsed but never sampled. Its position counterpart uses
  `m_positionOverCycle` (+0x118) as expected.
- Rewrite: `AttractorState::prepare` in `src/gameplay/particles/attractor.rs`
  keeps the double `m_velocity` sample.

### Trail segment motion reads the trail-life gravity
- `FUN_14020c620` samples `m_trailLifeCurves.m_acceleration.m_gravity`
  (+0xac8) at the segment's age; `m_segmentLifeCurves`' gravity (+0x1120,
  default 1 from 14020ee80) is never read (confirmed in the disassembly).
- Rewrite: `TrailRecord::move_point` in `src/gameplay/particles/trail.rs`
  keeps it.

## Particle ribbons add no bounds

140207230 measures the ribbon strip's bounds (1402054c0 and 140204950) into
a stack copy and never stores it; the emitter bounds (+0x140) it resets
stay empty, so 1402007b0 never merges a ribbon into the system bounds. The
rewrite keeps this (Ribbon::build returns no bounds).

## Havok particle renderer seeds are never written

arkHavokParticleStageSimulator sizes a per-body seed array at +0x340
(1401dfc20, 140384fe0) but nothing writes it. The sprite and mesh updates
(1401e83f0) and draw publications (1401e5440, 1401e4f50) read their seed
from it, so they see whatever the allocation held. The rewrite uses zero
(`gameplay::particles::havok_stage`, marked approximate).

## FX references to declarations that are not installed

`models/effects/gui/gamma_screen.fx` names the quad material
`scrquad_gamma_screen_01_mask.material`, and
`howling_bolt_low_frequency.fx` names the explosion
`explosion_low_frequency_large_player.explosion`; neither declaration is
installed. The decl manager builds them from their default definitions:
the material from 1402d7600/1402d4710 (`default.material`, arkDefault with
the stripped material path as its diffusemap), the explosion from the
generic 022c70 `{ }` over constructor 7e3560's defaults. The rewrite does
the same (`material::declaration`, `explosions::Declaration::acquire`).
The gamma mask quad then draws arkDefault in the AFTER_TONEMAP quad pass
(31da70 draws each quad with its material's +0xa0 program); the rewrite's
screen-quad pass has no arkDefault draw, so that menu effect stays refused.
