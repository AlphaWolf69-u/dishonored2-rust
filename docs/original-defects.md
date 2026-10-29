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
