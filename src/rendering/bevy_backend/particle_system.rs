//! Particle FX instances consume their parent owner, camera and shared RNG.
//! Stage simulation and draw publication retain their slots.
use crate::rendering::particles::definition::ParticleDefinition;
use crate::{
    gameplay::{
        effects::particle::{ParticleEffect, ParticleEffectFrame, ParticlePublication},
        particles::{
            pose::EmitterPose,
            simulation::{ParentStage, StageFrame, StageSimulation},
        },
        reference_frame::ReferenceFrame,
    },
    rendering::particles::{
        clipping::{ParticleClipPlanes, ParticleWaterSurface},
        material::ParticleMaterial,
    },
};
use std::{collections::HashMap, sync::Arc};

/// A mesh renderer whose material has a world MAIN program. The original
/// draws it as an instanced model surface in that material's own passes
/// (1401fbad0 -> 140319900 -> 1402bb550), with the system's shared lighting.
pub(crate) struct WorldMeshDraw {
    pub source: String,
    pub material: String,
    pub lighting: [[f32; 4]; 9],
    pub instances: Vec<crate::rendering::particles::ParticleInstance>,
    /// The publishing system (its slot), a model owner of its own: the
    /// renderer surfaces of one system share its caster record (201480).
    pub system: usize,
    /// The system's model bounds (1c3890's authored root plus 201480's
    /// generated bounds), its caster sphere source.
    pub system_bounds: [[f32; 3]; 2],
}

/// Inputs supplied by the actual render-view publication.
pub(crate) struct ParticleDrawContext<'a> {
    pub view: &'a crate::rendering::particles::SpriteView,
    pub camera: crate::rendering::particles::view::ParticleCamera,
    pub view_z: [f32; 4],
    pub probes: Option<&'a crate::probe_lighting::ProbeLightingSet>,
    pub alpha_scale: f32,
    pub cvars: &'a crate::platform::cvars::CVars,
    pub transparency: &'a crate::rendering::particles::transparency::ParticleTransparency,
    pub areas: &'a [super::fog::AreaFogFrame],
    pub world_images: &'a HashMap<String, Vec<Arc<crate::rendering::particles::image_resources::ParticleImage>>>,
}

pub(crate) struct ParticleSystem {
    pub effect: ParticleEffect,
    /// The immutable declaration remains leased while render-world particles
    /// outlive their initiating FX. Mutable overrides are not connected yet.
    pub parameter_block: Option<Arc<crate::assets::parm_block::ParmBlock>>,
    /// Per-instance value; automatic owner overrides must update this alongside
    /// the instance block before masked submission is enabled.
    pub masking_factor: f32,
    /// The system's declaration, shared by its stages and trails.
    pub declaration: Arc<crate::assets::fx::particle_declaration::ParticleDeclaration>,
    pub stages: Vec<StageSimulation>,
    /// System +0x396: arkHavokParticleStageSimulators.
    pub havok_stages: Vec<crate::gameplay::particles::havok_stage::HavokStageSimulation>,
    /// World-space union of the trail strips built at the last publication.
    pub trail_bounds: Option<[[f32; 3]; 2]>,
    /// System +0x39a: (declaration trail index, arkTrail).
    pub trails: Vec<(usize, crate::gameplay::particles::trail_system::DeclarationTrail)>,
    /// System +0x39c: (declaration ribbon index, arkRibbon).
    pub ribbons: Vec<(usize, crate::gameplay::particles::ribbon::Ribbon)>,
    /// System +0x398: (declaration beam index, arkBeam).
    pub beams: Vec<(usize, crate::gameplay::particles::beam_system::BeamEmitter)>,
    /// System +0x1d10: 1401fa780's defaults, then the FX record's target
    /// (140227b90) on every update.
    pub beam_target: crate::gameplay::particles::beam::BeamTarget,
    beam_noise: Option<Arc<crate::gameplay::particles::beam_noise::SimplexTable>>,
    pub materials: HashMap<String, Arc<ParticleMaterial>>,
    /// World-program mesh materials, drawn as material surfaces (1401fbad0).
    pub world_mesh_materials: Arc<std::collections::HashSet<String>>,
    pub system_seed: u32,
    pub first_simulation: bool,
    pub publication: Option<ParticlePublication>,
    pub clipping: Option<ParticleClipPlanes>,
    /// 1fa780 initializes system+1d50 to zero; world FX then publishes the
    /// owning entity's linear velocity through 227b90's ordinary packet.
    pub source_velocity: [f32; 3],
    /// Actual owning-entity motion from the FX manager's publication packet.
    pub source_motion: Option<crate::gameplay::particles::spawn::SpawnVelocity>,
    /// 201400 refreshes system+1d00 before the queued simulation task.
    wind: [f32; 3],
    pub water_flags: u32,
    /// 1ffb10 publishes the current declaration detail before stage updates.
    detail_level: usize,
    simulated_frame: Option<u32>,
    visible_frame: Option<u32>,
    visibility: crate::rendering::particles::visibility::ParticleVisibilityState,
    force_simulate: bool,
    force_visible: bool,
    /// 1401ff460's occlusion kill (1401fcc10) released the system.
    killed: bool,
    /// +0x1d98: the query visibility the sprites and meshes read, 1 unless
    /// the declaration's occlusion query is enabled.
    query_visibility: f32,
    /// The query curves' size and alpha factors at the last result.
    query_scale: [f32; 2],
    /// +0x1bd4/+0x1bcc: the query manager slot and its last result.
    query_slot: Option<crate::rendering::occlusion_query::QuerySlot>,
    query_result: f32,
    texture_area: f32,
    texture_images: Vec<Arc<crate::rendering::particles::image_resources::ParticleImage>>,
    world_texture_images: Option<Vec<Arc<crate::rendering::particles::image_resources::ParticleImage>>>,
    probe_query: crate::probe_lighting::ProbeQuery,
    debug_reported_child_draw: bool,
    /// System +0x1cf0: the arkFXEventInstances (140201760).
    fx_events: Vec<crate::gameplay::particles::fx_event::FxEventInstance>,
    /// +0x1df8: the selected camera position (1401ff460).
    view_origin: [f32; 3],
    /// FX the event instances started at the last spawn pass (1401fc020),
    /// with the record target each takes; the FX manager starts them.
    pub fx_starts: Vec<(crate::gameplay::particles::fx_event::FxEventStart, crate::gameplay::particles::beam::BeamTarget)>,
}
impl ParticleSystem {
    pub fn draw_pose(&self, frame: u32) -> Option<ReferenceFrame> {
        (self.visible_frame == Some(frame))
            .then(|| {
                self.publication
                    .as_ref()
                    .filter(|p| p.visible)
                    .map(|p| p.pose)
            })
            .flatten()
    }

    pub fn publish_draws(
        &mut self,
        frame: u32,
        context: ParticleDrawContext<'_>,
        output: &mut Vec<super::particle_pass::ParticleDraw>,
        world_meshes: &mut Vec<WorldMeshDraw>,
    ) -> Result<(), String> {
        use super::particle_pass::ParticleList;
        let start = output.len();
        let transparency = context.transparency;
        self.publish_surfaces(frame, context, output, world_meshes)?;
        // 31dce0 also draws underWaterPredicated, after dynamicAlpha, with
        // dynamicAlpha's writePixel state and stencil reference 0x44.
        let mut alpha = Vec::new();
        for draw in &output[start..] {
            if draw.list != ParticleList::UnderwaterPredicated {
                continue;
            }
            let mut twin = draw.clone();
            twin.transparency = transparency.get(draw.material.state)?;
            let (state, reference) = crate::render_state::alpha_surface_draw_state(
                draw.material.state, twin.transparency.mode != 0, false);
            twin.state = state;
            twin.stencil_reference = reference;
            twin.list = ParticleList::PredicatedAlpha;
            alpha.push(twin);
        }
        output.extend(alpha);
        Ok(())
    }

    fn publish_surfaces(
        &mut self,
        frame: u32,
        context: ParticleDrawContext<'_>,
        output: &mut Vec<super::particle_pass::ParticleDraw>,
        world_meshes: &mut Vec<WorldMeshDraw>,
    ) -> Result<(), String> {
        if self.visible_frame != Some(frame) {
            return Ok(());
        }
        let Some(publication) = self.publication.as_ref().filter(|p| p.visible) else {
            return Ok(());
        };
        let declaration = self.declaration.clone();
        // 14031ad30 walks prepared surfaces without an instance-count test.
        // Prefetch inactive renderers too, before their next burst activates.
        for image in &self.texture_images { image.demand.submit_dynamic(self.texture_area); }
        if self.world_texture_images.is_none() {
            let mut seen = std::collections::HashSet::new();
            let mut images = Vec::new();
            for stage in &self.stages {
                let definition = &stage.declaration.stages[stage.index];
                for name in definition.sprites.iter().filter(|d| d.enabled).filter_map(|d| d.material.as_ref())
                    .chain(definition.meshes.iter().filter(|d| d.enabled && d.model.is_some()).filter_map(|d| d.material.as_ref())) {
                    if let Some(bindings) = context.world_images.get(name) {
                        for image in bindings {
                            if seen.insert(image.path.clone()) { images.push(image.clone()); }
                        }
                    }
                }
            }
            self.world_texture_images = Some(images);
        }
        for image in self.world_texture_images.as_ref().unwrap() {
            image.demand.submit_dynamic(self.texture_area);
        }
        // System +0x1de0/+0x1de4: the distance size and alpha, scaled by
        // the occlusion query curves (1401ffbc0).
        let owner_scale = crate::float::multiply(
            context.view.owner_scale,
            crate::float::multiply(
                self.query_scale[0],
                declaration.distance.size.sample_visibility(context.camera.emitter_distance)?,
            ),
        );
        let alpha_scale = crate::float::multiply(
            context.alpha_scale,
            crate::float::multiply(
                self.query_scale[1],
                declaration.distance.alpha.sample_visibility(context.camera.emitter_distance)?,
            ),
        );
        let receiver_bounds = crate::rendering::particles::bounds::system_bounds(
            &self.stages,
            &declaration.visibility,
            publication.pose,
            owner_scale,
            self.trail_bounds,
            &self.havok_stages,
        )?;
        let mut trail_bounds = [[f32::from_bits(0x7149_f2ca); 3], [f32::from_bits(0xf149_f2ca); 3]];
        let volumes = context
            .areas
            .iter()
            .map(|area| area.bounds)
            .collect::<Vec<_>>();
        let center = std::array::from_fn(|i| {
            crate::float::multiply(
                crate::float::add(receiver_bounds[0][i], receiver_bounds[1][i]),
                0.5,
            )
        });
        let fog = crate::rendering::fog::selection::select(
            &volumes,
            context.camera.position,
            center,
            receiver_bounds,
            context.view_z,
        )
        .map(|selected| {
            super::fog_objects::FogObjectParameters::selected(
                &context.areas[selected.index],
                selected.depths,
            )
        })
        .unwrap_or_default();
        // Particle root+1c40 defaults to zero in1fcaa0. Its virtual+88
        // (1fc4c0) then samples the emitter origin, not the live bounds centre.
        // 2b3dc0 ->385f20 ->385da0 uses the shared cached Voronoi query.
        // The Havok stages publish their renderers after the ordinary ones
        // (1401e4930), through the same draw path and the same root record.
        let needs_lighting = self
            .stages
            .iter()
            .map(|stage| &stage.declaration.stages[stage.index])
            .chain(self.havok_stages.iter().map(|stage| &stage.definition().stage))
            .flat_map(|definition| {
                definition.sprites.iter().filter(|sprite|sprite.enabled).map(|sprite|&sprite.material)
                    .chain(definition.meshes.iter().filter(|mesh|mesh.enabled&&mesh.model.is_some()).map(|mesh|&mesh.material))
            })
            .try_fold(false, |needed, material| {
                let Some(name) = material.as_ref() else {
                    return Ok::<_, String>(needed);
                };
                // 1eaa40 copies the system's shared tetrahedron record into
                // each world-mesh instance (measured non-null for eotw flies).
                if self.world_mesh_materials.contains(name) {
                    return Ok(true);
                }
                let material = self
                    .materials
                    .get(name)
                    .ok_or_else(|| format!("particle draw material absent: {name}"))?;
                Ok(needed || material.lighting.is_some())
            })?;
        let lighting = if needs_lighting {
            Some(
                context
                    .probes
                    .ok_or("particle probe network is unavailable")?
                    .interpolate_query_sh(publication.pose.position, &mut self.probe_query)
                    .ok_or("particle origin has no authored probe tetrahedron")?
                    .object_shader_record(),
            )
        } else {
            None
        };
        let view = crate::rendering::particles::SpriteView {
            camera_position: context.camera.position,
            camera_axes: context.camera.axes,
            custom_basis: publication.pose.basis,
            owner_colour: publication.colour,
            // 1fda30 samples both curves at emitter-to-camera distance;
            // 1ee9b0 consumes these as context+d0/d4 (system+1de0/1de4).
            owner_scale,
            ..*context.view
        };
        // Ordinary stages, then the Havok stages (1401fef80's publication
        // 1401e4930 after the system's ordinary stages).
        let havok_bodies: Vec<_> = self.havok_stages.iter()
            .map(|stage| move |slot: usize| stage.orientation(slot)).collect();
        type HavokStage = crate::gameplay::particles::havok_stage::HavokStageSimulation;
        let mut draw_stages: Vec<(crate::rendering::particles::publication::DrawStage, Option<&StageSimulation>, Option<&HavokStage>)> =
            self.stages.iter().map(|stage| (crate::rendering::particles::publication::DrawStage::ordinary(stage), Some(stage), None)).collect();
        for (stage, bodies) in self.havok_stages.iter().zip(&havok_bodies) {
            draw_stages.push((crate::rendering::particles::publication::DrawStage {
                definition: &stage.definition().stage,
                particles: &stage.particles,
                sprites: &stage.sprites,
                meshes: &stage.meshes,
                bodies: Some((bodies as &dyn Fn(usize) -> Option<[[f32; 3]; 3]>, stage.centroid_offset())),
            }, None, Some(stage)));
        }
        for (draw, ordinary, havok) in &draw_stages {
            let definition = draw.definition;
            let transform = if definition.worldspace || ordinary.is_none() {
                ReferenceFrame::default()
            } else {
                publication.pose
            };
            // 1f2420: one publication order per stage, shared by its renderers.
            let order = draw.particles.publication_order(definition.sorting,
                context.camera.position, context.camera.axes[0]);
            for (index, sprite) in definition.sprites.iter().enumerate() {
                if !sprite.enabled {
                    continue;
                }
                let Some(name) = sprite.material.as_ref() else {
                    continue;
                };
                let grid_level = crate::rendering::particles::geometry::SpriteGrid::level_for_distance(
                    sprite.tessellation_distance,
                    context.camera.emitter_distance,
                );
                // A world-program sprite is its material's surface over the
                // selected grid (1401edab0), like a world-material mesh.
                if self.world_mesh_materials.contains(name) {
                    let mut instances = Vec::with_capacity(draw.particles.len());
                    let bounds = crate::rendering::particles::publication::stage_instances(
                        draw, index, transform, &view, self.query_visibility, alpha_scale,
                        context.camera.admission_plane, sprite.minimal_distance, order.as_deref(), &mut instances,
                    )?;
                    if bounds.is_some() && !instances.is_empty() {
                        world_meshes.push(WorldMeshDraw {
                            source: crate::rendering::particles::world_sources::sprite_grid_source(grid_level),
                            material: name.clone(),
                            lighting: lighting.ok_or("world particle sprite lighting was not selected")?,
                            instances,
                            system: 0,
                            system_bounds: receiver_bounds,
                        });
                    }
                    continue;
                }
                let material = self
                    .materials
                    .get(name)
                    .ok_or_else(|| format!("particle draw material absent: {name}"))?;
                let mut instances = Vec::with_capacity(draw.particles.len());
                let bounds = crate::rendering::particles::publication::stage_instances(
                    draw,
                    index,
                    transform,
                    &view,
                    self.query_visibility,
                    alpha_scale,
                    context.camera.admission_plane,
                    sprite.minimal_distance,
                    order.as_deref(),
                    &mut instances,
                )?;
                if bounds.is_none() {
                    continue;
                }
                let mut transparency = context.transparency.get(material.state)?;
                if material.distortion { transparency.mode = 0; }
                let (state, stencil_reference, list) = self.surface_route(material, &mut transparency);
                if !self.debug_reported_child_draw
                    && definition.burst
                    && !definition.burst_event.is_empty()
                    && !instances.is_empty()
                {
                    bevy::log::info!(
                        stage = definition.name,
                        event = definition.burst_event,
                        material = name,
                        particles = instances.len(),
                        "Particle child-stage draw published"
                    );
                    self.debug_reported_child_draw = true;
                }
                output.push(super::particle_pass::ParticleDraw {
                    list,
                    material: material.clone(),
                    masking_factor: self.masking_factor,
                    lighting: if material.option("fx/opt/use_lighting")? != 0 {
                        lighting
                    } else {
                        None
                    },
                    transparency,
                    state,
                    stencil_reference,
                    mesh: None,
                    strip: Vec::new(),
                    grid_level:
                        crate::rendering::particles::geometry::SpriteGrid::level_for_distance(
                            sprite.tessellation_distance,
                            context.camera.emitter_distance,
                        ),
                    instances,
                    clip_plane: self
                        .clipping
                        .as_ref()
                        .and_then(|planes| planes.rendering)
                        .map_or(bevy::prelude::Vec4::W, bevy::prelude::Vec4::from_array),
                    clip_fade_distance: self.clipping.as_ref().filter(|p| p.rendering.is_some())
                        .map_or(0., |p| p.fade_distance),
                    fog,
                });
            }
            for (index,mesh) in definition.meshes.iter().enumerate() {
                if !mesh.enabled {continue;}
                let (Some(name),Some(source),Some(model))=(&mesh.material,&mesh.source,&mesh.model) else {continue};
                if model.vertices.is_empty(){continue;}
                if self.world_mesh_materials.contains(name) {
                    let mut instances=Vec::with_capacity(draw.particles.len());
                    crate::rendering::particles::mesh_publication::stage_instances(draw,index,transform,
                        &view,self.query_visibility,alpha_scale,order.as_deref(),&mut instances)?;
                    if !instances.is_empty() {
                        world_meshes.push(WorldMeshDraw{source:source.clone(),material:name.clone(),
                            lighting:lighting.ok_or("world particle mesh lighting was not selected")?,instances,
                            system:0,system_bounds:receiver_bounds});
                    }
                    continue;
                }
                let material=self.materials.get(name).ok_or_else(||format!("particle mesh material absent: {name}"))?;
                let mut instances=Vec::with_capacity(draw.particles.len());
                crate::rendering::particles::mesh_publication::stage_instances(draw,index,transform,
                    &view,self.query_visibility,alpha_scale,order.as_deref(),&mut instances)?;
                if instances.is_empty(){continue;}
                let mut transparency=context.transparency.get(material.state)?;
                if material.distortion{transparency.mode=0;}
                let (state, stencil_reference, list) = self.surface_route(material, &mut transparency);
                output.push(super::particle_pass::ParticleDraw {
                    list,
                    masking_factor:self.masking_factor,
                    material:material.clone(),lighting:if material.lighting.is_some(){lighting}else{None},
                    transparency,state,stencil_reference,grid_level:0,
                    mesh:Some((source.clone(),model.clone())),instances,strip:Vec::new(),
                    clip_plane:self.clipping.as_ref().and_then(|planes|planes.rendering).map_or(bevy::prelude::Vec4::W,bevy::prelude::Vec4::from_array),
                    clip_fade_distance:self.clipping.as_ref().filter(|planes|planes.rendering.is_some()).map_or(0.,|planes|planes.fade_distance),fog,
                });
            }
            // 1401f3060: each trail renderer builds its live particles'
            // strips, in slot order, within its vertex budget.
            // Ordinary stages draw their live slots in slot order; a Havok
            // stage (1401e5980) its post-step live list, in world space.
            let (stage_trails, live_slots, capacity, local): (&[Vec<Option<crate::gameplay::particles::trail::TrailRecord>>], Vec<usize>, usize, bool) =
                match (ordinary, havok) {
                    (Some(stage), _) => (
                        &stage.trails,
                        stage.particles.iter().map(|(slot, _)| slot).collect(),
                        stage.particles.capacity(),
                        !definition.worldspace,
                    ),
                    (None, Some(stage)) => (
                        &stage.trails,
                        stage.live.iter().map(|&body| body as usize).collect(),
                        stage.particles.capacity(),
                        false,
                    ),
                    (None, None) => continue,
                };
            for (trail_index, trail) in definition.trails.iter().enumerate() {
                let records = &stage_trails[trail_index];
                if records.is_empty() {
                    continue;
                }
                let Some(name) = trail.material.as_ref() else { continue };
                let material = self.materials.get(name).ok_or_else(|| format!("particle trail material absent: {name}"))?;
                let live: Vec<&crate::gameplay::particles::trail::TrailRecord> = live_slots.iter()
                    .filter_map(|&slot| records.get(slot).and_then(Option::as_ref))
                    .collect();
                let budget = crate::gameplay::particles::trail::TrailBudget::new(
                    live.iter().map(|record| record.segments),
                    crate::gameplay::particles::trail::TrailBudget::limit(trail, capacity),
                );
                // 140211a90: a local stage's local trail is built in stage
                // space and moved by the owner matrix (140210350).
                let owner = (local && !trail.worldspace)
                    .then(|| crate::gameplay::particles::trail::owner_transform(publication.pose));
                let mut strip = Vec::new();
                for record in &live {
                    let segments = budget.segments(record.segments);
                    if segments != 0 {
                        let mut bounds = record.build(trail, segments, context.camera.position, publication.colour,
                            owner.as_ref(), &mut strip)?;
                        if owner.is_some() && bounds[0][0] <= bounds[1][0] {
                            bounds = crate::rendering::particles::bounds::transform(bounds, publication.pose);
                        }
                        crate::rendering::particles::bounds::include(&mut trail_bounds, bounds);
                    }
                }
                if strip.is_empty() {
                    continue;
                }
                let mut transparency = context.transparency.get(material.state)?;
                if material.distortion { transparency.mode = 0; }
                let (state, stencil_reference, list) = self.surface_route(material, &mut transparency);
                output.push(super::particle_pass::ParticleDraw {
                    list,
                    masking_factor: self.masking_factor,
                    material: material.clone(),
                    lighting: if material.lighting.is_some() { lighting } else { None },
                    transparency,
                    state,
                    stencil_reference,
                    grid_level: 0,
                    mesh: None,
                    instances: vec![crate::rendering::particles::ParticleInstance {
                        position: [0.; 3],
                        axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                        parameters: crate::rendering::particles::PackedParticleParameters {
                            colour: u32::MAX, flipbook_seed_uv: 0, size: 0,
                        },
                    }],
                    strip,
                    clip_plane: self.clipping.as_ref().and_then(|planes| planes.rendering)
                        .map_or(bevy::prelude::Vec4::W, bevy::prelude::Vec4::from_array),
                    clip_fade_distance: self.clipping.as_ref().filter(|planes| planes.rendering.is_some())
                        .map_or(0., |planes| planes.fade_distance),
                    fog,
                });
            }
        }
        // 1401d8d50: each beam emitter draws its live beams as one strip.
        for (index, beam) in &self.beams {
            let asset = &declaration.beams[*index];
            let Some(name) = asset.material.as_ref() else { continue };
            let material = self.materials.get(name).ok_or_else(|| format!("particle beam material absent: {name}"))?;
            let mut strip = Vec::new();
            let bounds = beam.build(asset, context.camera.position, publication.colour, publication.pose,
                self.beam_noise.as_deref(), &mut strip)?;
            crate::rendering::particles::bounds::include(&mut trail_bounds, bounds);
            if strip.is_empty() {
                continue;
            }
            let mut transparency = context.transparency.get(material.state)?;
            if material.distortion { transparency.mode = 0; }
            let (state, stencil_reference, list) = self.surface_route(material, &mut transparency);
            output.push(super::particle_pass::ParticleDraw {
                list,
                masking_factor: self.masking_factor,
                material: material.clone(),
                lighting: if material.lighting.is_some() { lighting } else { None },
                transparency,
                state,
                stencil_reference,
                grid_level: 0,
                mesh: None,
                instances: vec![crate::rendering::particles::ParticleInstance {
                    position: [0.; 3],
                    axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    parameters: crate::rendering::particles::PackedParticleParameters {
                        colour: u32::MAX, flipbook_seed_uv: 0, size: 0,
                    },
                }],
                strip,
                clip_plane: self.clipping.as_ref().and_then(|planes| planes.rendering)
                    .map_or(bevy::prelude::Vec4::W, bevy::prelude::Vec4::from_array),
                clip_fade_distance: self.clipping.as_ref().filter(|planes| planes.rendering.is_some())
                    .map_or(0., |planes| planes.fade_distance),
                fog,
            });
        }
        // 140212410: each declaration trail draws its live records.
        for (index, trail) in &self.trails {
            let asset = &declaration.trails[*index];
            let Some(name) = asset.trail.material.as_ref() else { continue };
            let material = self.materials.get(name).ok_or_else(|| format!("particle trail material absent: {name}"))?;
            let mut strip = Vec::new();
            let bounds = trail.build(asset, context.camera.position, publication.colour, None, &mut strip)?;
            crate::rendering::particles::bounds::include(&mut trail_bounds, bounds);
            if strip.is_empty() {
                continue;
            }
            let mut transparency = context.transparency.get(material.state)?;
            if material.distortion { transparency.mode = 0; }
            let (state, stencil_reference, list) = self.surface_route(material, &mut transparency);
            output.push(super::particle_pass::ParticleDraw {
                list,
                masking_factor: self.masking_factor,
                material: material.clone(),
                lighting: if material.lighting.is_some() { lighting } else { None },
                transparency,
                state,
                stencil_reference,
                grid_level: 0,
                mesh: None,
                instances: vec![crate::rendering::particles::ParticleInstance {
                    position: [0.; 3],
                    axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    parameters: crate::rendering::particles::PackedParticleParameters {
                        colour: u32::MAX, flipbook_seed_uv: 0, size: 0,
                    },
                }],
                strip,
                clip_plane: self.clipping.as_ref().and_then(|planes| planes.rendering)
                    .map_or(bevy::prelude::Vec4::W, bevy::prelude::Vec4::from_array),
                clip_fade_distance: self.clipping.as_ref().filter(|planes| planes.rendering.is_some())
                    .map_or(0., |planes| planes.fade_distance),
                fog,
            });
        }
        // 140207230: each ribbon draws its point chain. Its bounds never
        // reach the system (see Ribbon::build).
        for (index, ribbon) in &self.ribbons {
            let asset = &declaration.ribbons[*index];
            let Some(name) = asset.material.as_ref() else { continue };
            let material = self.materials.get(name).ok_or_else(|| format!("particle ribbon material absent: {name}"))?;
            let mut strip = Vec::new();
            ribbon.build(asset, context.camera.position, publication.colour, publication.pose,
                self.beam_noise.as_deref(), &mut strip)?;
            if strip.is_empty() {
                continue;
            }
            let mut transparency = context.transparency.get(material.state)?;
            if material.distortion { transparency.mode = 0; }
            let (state, stencil_reference, list) = self.surface_route(material, &mut transparency);
            output.push(super::particle_pass::ParticleDraw {
                list,
                masking_factor: self.masking_factor,
                material: material.clone(),
                lighting: if material.lighting.is_some() { lighting } else { None },
                transparency,
                state,
                stencil_reference,
                grid_level: 0,
                mesh: None,
                instances: vec![crate::rendering::particles::ParticleInstance {
                    position: [0.; 3],
                    axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    parameters: crate::rendering::particles::PackedParticleParameters {
                        colour: u32::MAX, flipbook_seed_uv: 0, size: 0,
                    },
                }],
                strip,
                clip_plane: self.clipping.as_ref().and_then(|planes| planes.rendering)
                    .map_or(bevy::prelude::Vec4::W, bevy::prelude::Vec4::from_array),
                clip_fade_distance: self.clipping.as_ref().filter(|planes| planes.rendering.is_some())
                    .map_or(0., |planes| planes.fade_distance),
                fog,
            });
        }
        self.trail_bounds = (trail_bounds[0][0] <= trail_bounds[1][0]).then_some(trail_bounds);
        Ok(())
    }

    pub fn start_selected(
        definition: &ParticleDefinition,
        index: usize,
        offsets: [f32; 4],
        seed: &mut u32,
    ) -> Result<Self, String> {
        let declaration = definition.declarations[index].as_ref().unwrap();
        // 1c4b50 derives decl+618 bit1 from finite, non-looping lifetime
        // stages without an external burst-event producer. 1c4460 derives
        // bit4 from enabled sprite materials with a dark-vision layer.
        // 1c4b50 also tests beams and declaration trails the same way, and
        // ribbons by their cycle count alone.
        let force_simulate = declaration.stages.iter().any(|stage| {
            stage.enabled
                && stage.emitter.cycles != 0
                && !stage.infinite_life
                && !(stage.burst && !stage.burst_event.is_empty())
        }) || declaration.beams.iter().map(|beam| (beam.enabled, &beam.timings))
            .chain(declaration.trails.iter().map(|trail| (trail.enabled, &trail.timings)))
            .any(|(enabled, timings)| {
                enabled
                    && timings.emitter.cycles != 0
                    && !timings.infinite_life
                    && !(timings.burst && !timings.burst_event.is_empty())
            })
            || declaration.ribbons.iter().any(|ribbon| ribbon.enabled && ribbon.timings.cycles != 0);
        let texture_materials: Vec<_> = declaration
            .stages
            .iter()
            .filter(|stage| stage.enabled)
            .flat_map(|stage|stage.sprites.iter().filter(|sprite|sprite.enabled).filter_map(|sprite|sprite.material.as_ref())
                .chain(stage.meshes.iter().filter(|mesh|mesh.enabled&&mesh.model.is_some()).filter_map(|mesh|mesh.material.as_ref())))
            .filter_map(|name| definition.materials.get(name))
            .collect();
        let force_visible = texture_materials.iter().any(|material| material.darkvision_layer != 0);
        let mut seen = std::collections::HashSet::new();
        let texture_images = texture_materials.iter().flat_map(|material| material.images.values())
            .filter(|image| image.scene_owned && seen.insert(image.path.clone()))
            .cloned().collect();
        let (effect, (stages, havok_stages, beams, trails, ribbons, system_seed)) = ParticleEffect::new(
            definition.owner.clone(),
            offsets,
            seed,
            |_, mut system_seed| {
                let mut stages = Vec::new();
                for (index, stage) in declaration.stages.iter().enumerate() {
                    if stage.enabled {
                        stages.push(StageSimulation::new(
                            declaration.clone(),
                            index,
                            ReferenceFrame::default(),
                            &mut system_seed,
                        )?);
                    }
                }
                // 140201760 links each stage to the first enabled stage
                // whose descriptor its m_spawnStage names.
                let names: Vec<&str> = stages
                    .iter()
                    .map(|stage| declaration.stages[stage.index].name.as_str())
                    .collect();
                for stage in &mut stages {
                    let wanted = &declaration.stages[stage.index].spawn_stage.stage_name;
                    if !wanted.is_empty() {
                        stage.parent = names.iter().position(|name| name == wanted);
                    }
                }
                // 140201760 then creates a simulator for each enabled Havok
                // stage (1401dfc20), before the beams.
                let world_gravity = crate::gameplay::physics_world::WorldInfo::game_settings([[0.; 4]; 2]).gravity;
                let mut havok_stages = Vec::new();
                for (index, stage) in declaration.havok_stages.iter().enumerate() {
                    if stage.stage.enabled {
                        havok_stages.push(crate::gameplay::particles::havok_stage::HavokStageSimulation::new(
                            declaration.clone(), index, ReferenceFrame::default(), &mut system_seed, world_gravity)?);
                    }
                }
                // 140201760 then creates an arkBeam for each enabled beam
                // with a material (1401d0ff0), before the trails.
                let mut beams = Vec::new();
                for (index, beam) in declaration.beams.iter().enumerate() {
                    if beam.enabled && beam.material.is_some() {
                        beams.push((index, crate::gameplay::particles::beam_system::BeamEmitter::new(
                            beam, &mut system_seed, ReferenceFrame::default())?));
                    }
                }
                // 140201760 then creates an arkTrail for each enabled
                // declaration trail with a material (1402107a0).
                let mut trails = Vec::new();
                for (index, trail) in declaration.trails.iter().enumerate() {
                    if trail.enabled && trail.trail.material.is_some() {
                        trails.push((index, crate::gameplay::particles::trail_system::DeclarationTrail::new(
                            trail, &mut system_seed, ReferenceFrame::default())?));
                    }
                }
                // ...and an arkRibbon for each enabled ribbon with a
                // material (1402042b0), after the trails.
                let mut ribbons = Vec::new();
                for (index, ribbon) in declaration.ribbons.iter().enumerate() {
                    if ribbon.enabled && ribbon.material.is_some() {
                        ribbons.push((index, crate::gameplay::particles::ribbon::Ribbon::new(ribbon, &mut system_seed)?));
                    }
                }
                Ok((stages, havok_stages, beams, trails, ribbons, system_seed))
            },
        )?;
        Ok(Self {
            effect,
            parameter_block: definition.parameter_block.clone(),
            masking_factor: definition.parameter_block.as_ref().map(|block|
                block.numeric_parameter("rendering/masking/bendtimefactor"))
                .transpose()?.flatten().map_or(0.0, |words| f32::from_bits(words[0])),
            declaration: declaration.clone(),
            stages,
            havok_stages,
            trails,
            beams,
            ribbons,
            beam_target: Default::default(),
            beam_noise: definition.beam_noise.clone(),
            trail_bounds: None,
            materials: definition.materials.clone(),
            world_mesh_materials: Arc::new(definition.world_mesh_materials.clone()),
            system_seed,
            first_simulation: true,
            publication: None,
            clipping: None,
            source_velocity: [0.; 3],
            source_motion: None,
            wind: [0.; 3],
            water_flags: 0,
            detail_level: 0,
            simulated_frame: None,
            visible_frame: None,
            visibility: Default::default(),
            force_simulate,
            force_visible,
            killed: false,
            query_visibility: 1.,
            query_scale: [1.; 2],
            query_slot: None,
            query_result: 0.,
            texture_area: 0.,
            texture_images,
            world_texture_images: None,
            probe_query: Default::default(),
            debug_reported_child_draw: false,
            fx_events: crate::gameplay::particles::events::fx_event_instances(declaration)
                .into_iter()
                .map(crate::gameplay::particles::fx_event::FxEventInstance::new)
                .collect(),
            view_origin: [0.; 3],
            fx_starts: Vec::new(),
        })
    }

    /// 1fc170 considers both remaining emission and live particle storage.
    /// Its rate threshold at141ad2734 is exactly 2^-23.
    /// 14033b350 routes a dynamicAlpha surface by the FX model's +0x1c1c
    /// water bits (1fddc0): bit 2 to underWaterAlphaLit and bit 4 to
    /// underWaterPredicated, both drawn by 140311aa0 with the material's lit
    /// program (3119a0 key 2) instead of writePixel; otherwise ALPHA_LIT's
    /// 31dce0 state, whose stencil reference follows bit 8.
    fn surface_route(
        &self,
        material: &crate::rendering::particles::material::ParticleMaterial,
        transparency: &mut crate::rendering::particles::transparency::TransparencyParameters,
    ) -> (crate::render_state::RenderState, u32, super::particle_pass::ParticleList) {
        use super::particle_pass::ParticleList;
        if !material.distortion && !material.opaque {
            if self.water_flags & 2 != 0 {
                transparency.mode = 0;
                return (crate::render_state::underwater_alpha_lit_state(material.state), 0, ParticleList::UnderwaterLit);
            }
            if self.water_flags & 4 != 0 {
                transparency.mode = 0;
                let (state, reference) = crate::render_state::underwater_predicated_state(material.state);
                return (state, reference, ParticleList::UnderwaterPredicated);
            }
        }
        let (state, reference) = crate::render_state::alpha_surface_draw_state(
            material.state,
            transparency.mode != 0,
            self.water_flags & 8 != 0,
        );
        (state, reference, ParticleList::DynamicAlpha)
    }

    fn finished(&self) -> bool {
        if self.killed {
            return true;
        }
        let rate = self.effect.published_rate();
        let emitting = |can_emit: bool| f32::from_bits(0x3400_0000) < rate && can_emit;
        self.stages.iter().all(|stage| {
            stage.particles.is_empty()
                && !emitting(stage.emitter.cycle.can_emit(stage.emitter.definition.cycles))
        }) && self.havok_stages.iter().all(|stage| {
            // 1401e5ca0 / 1401e3cd0
            stage.live_count() == 0
                && !emitting(stage.emitter.cycle.can_emit(stage.emitter.definition.cycles))
        }) && self.trails.iter().all(|(index, trail)| {
            // 1402122f0 / 140212720
            trail.live_count() == 0
                && !emitting(trail.can_emit(&self.declaration.trails[*index]))
        }) && self.beams.iter().all(|(index, beam)| {
            // 1401d9070 / 1401d8c60
            beam.live_count() == 0
                && !emitting(beam.can_emit(&self.declaration.beams[*index]))
        }) && self.ribbons.iter().all(|(index, ribbon)| {
            // 14012d960 / 1402054b0
            ribbon.live_count() == 0
                && !emitting(ribbon.can_emit(&self.declaration.ribbons[*index]))
        })
    }

    /// The Havok stages' events to the ordinary stages' input queues and
    /// the FX event instances (their listener calls in the original).
    fn route_havok_events(
        havok_stages: &mut [crate::gameplay::particles::havok_stage::HavokStageSimulation],
        stages: &mut [StageSimulation],
        fx_events: &mut [crate::gameplay::particles::fx_event::FxEventInstance],
        declaration: &crate::assets::fx::particle_declaration::ParticleDeclaration,
    ) {
        for havok in havok_stages {
            for delivery in havok.deliveries.drain(..) {
                if let Some(receiver) = stages.iter_mut().find(|stage| stage.index == delivery.receiver) {
                    let worldspace = receiver.declaration.stages[receiver.index].worldspace;
                    receiver.particles.enqueue(delivery.input, worldspace, receiver.owner_pose.position, receiver.owner_pose.basis);
                }
            }
            Self::record_fx_events(fx_events, declaration, havok.fx_pending.drain(..));
        }
    }

    /// 1401db0b0 for each record, in publication order.
    fn record_fx_events(
        fx_events: &mut [crate::gameplay::particles::fx_event::FxEventInstance],
        declaration: &crate::assets::fx::particle_declaration::ParticleDeclaration,
        records: impl Iterator<Item = (usize, crate::gameplay::particles::fx_event::FxEventRecord)>,
    ) {
        for (instance, record) in records {
            if let Some(event) = fx_events.get_mut(instance) {
                event.record(&declaration.fx_events[event.index], record);
            }
        }
    }

    /// 1401fc020 after the system's simulation: every pending record starts
    /// its FX. The target is the system origin (+0x1860/+0x186c) or the
    /// system's target (+0x1d10).
    fn start_fx_events(&mut self, origin: ReferenceFrame) {
        let declaration = self.declaration.clone();
        for event in &mut self.fx_events {
            let asset = &declaration.fx_events[event.index];
            let mut starts = Vec::new();
            event.start(asset, self.view_origin, &mut self.system_seed, &mut starts);
            for start in starts {
                let target = if start.target_is_system_target {
                    crate::gameplay::particles::beam::BeamTarget { position: self.beam_target.position, basis: self.beam_target.basis }
                } else {
                    crate::gameplay::particles::beam::BeamTarget { position: origin.position, basis: origin.basis }
                };
                self.fx_starts.push((start, target));
            }
        }
    }

    pub fn update(
        &mut self,
        now: f32,
        frame: &ParticleEffectFrame,
        view_underwater: bool,
        collision: Option<(&crate::gameplay::navigation::CollisionMap, &[bool])>,
        seed: &mut u32,
    ) -> Result<(), String> {
        self.publication = self.effect.update(now, frame, self.finished(), seed)?;
        // The physics world steps every Havok particle pipeline each frame,
        // whether or not the system is simulated or drawn (1400908c0).
        let gravity = crate::gameplay::physics_world::WorldInfo::game_settings([[0.; 4]; 2]).gravity;
        for stage in &mut self.havok_stages {
            let level = collision.map(|(map, owners)| crate::gameplay::havok_fx::physics_collide::CollideWorld {
                map,
                owners,
                filter: stage.collisions.as_ref().map_or(0, |collisions| collisions.filter),
            });
            stage.step(frame.delta, gravity, level)?;
        }
        Self::route_havok_events(&mut self.havok_stages, &mut self.stages, &mut self.fx_events, &self.declaration);
        // 140227b90: the FX record's target parameters.
        self.beam_target = frame.target;
        crate::render_model::update_particle_water_flags(
            &mut self.water_flags,
            self.declaration.underwater,
            self.declaration.handle_seen_from_underwater,
            view_underwater,
        );
        Ok(())
    }

    /// Consume a completed physics/ocean lookup at the published FX origin.
    /// None means neither a queried water body nor an active ocean was found;
    /// an unavailable query must be resolved before invoking this method.
    pub fn simulate_in_world(
        &mut self,
        frame: u32,
        world: &crate::gameplay::world_point_query::level::LevelPointQueries,
        point_query: &mut crate::gameplay::world_point_query::WorldPointQuery,
        ray_query: &mut crate::gameplay::world_ray_query::WorldRayQuery,
        ocean_height: Option<f32>,
        view: &crate::rendering::view_history::ViewMatrices,
        visibility: Option<&crate::tome::RenderAabbQuery<'_>>,
        minimum_angular_size: f32,
        wind: &crate::gameplay::wind::WindSnapshot,
    ) -> Result<(), String> {
        if self.simulated_frame == Some(frame) {
            return Ok(());
        }
        let Some(publication) = self.publication.as_ref().filter(|p| p.visible) else {
            return Ok(());
        };
        let declaration = self.declaration.clone();
        let delta = publication.simulation_delta;
        let camera = crate::rendering::particles::view::ParticleCamera::select(
            publication.pose.position,
            view.position,
            view.position,
            [
                std::array::from_fn(|i| -view.view[2][i]),
                std::array::from_fn(|i| -view.view[0][i]),
                std::array::from_fn(|i| view.view[1][i]),
            ],
        );
        self.detail_level = declaration.detail_level(camera.emitter_distance);
        self.view_origin = camera.position;
        let size = declaration
            .distance
            .size
            .sample_visibility(camera.emitter_distance)?;
        let alpha = declaration
            .distance
            .alpha
            .sample_visibility(camera.emitter_distance)?;
        // 1401ffbc0 with 14031d7e0's per-system request: a drawn system with
        // an enabled query keeps a manager slot at its position and reads
        // the slot's six-frame average; otherwise the slot is released.
        self.query_visibility = 1.;
        self.query_scale = [1.; 2];
        let query = &declaration.occlusion_query;
        if query.enabled {
            let mut manager = crate::rendering::occlusion_query::shared().lock().map_err(|_| "occlusion query manager poisoned")?;
            if self.query_slot.is_none() {
                self.query_slot = crate::rendering::occlusion_query::QuerySlot::allocate(&mut manager);
            }
            if let Some(slot) = &self.query_slot {
                manager.enable(slot.index(), true);
                manager.request(slot.index(), publication.pose.position, query.radius, query.offset, false, false);
                if let Some(result) = manager.result(slot.index(), true) {
                    self.query_result = result;
                }
            }
            self.query_visibility = self.query_result;
            self.query_scale = [
                query.size.sample_visibility(self.query_result)?,
                query.alpha.sample_visibility(self.query_result)?,
            ];
        } else {
            self.query_slot = None;
        }
        let bounds = crate::rendering::particles::bounds::system_bounds(
            &self.stages,
            &declaration.visibility,
            publication.pose,
            size,
            self.trail_bounds,
            &self.havok_stages,
        )?;
        let frustum_visible = || {
            !crate::render_model::transformed_aabb_clip_cull(
                &std::array::from_fn(|i| view.view_projection[i / 4][i % 4]),
                bounds[0],
                bounds[1],
            )
            .culled
        };
        // 14031ad30 forwards the flag-zero bounds-query area to image demand.
        // This is the same query already used for ordinary particle admission.
        let mut area = None;
        let root_visible = if declaration.visibility.always_visible || self.force_visible {
            frustum_visible()
        } else {
            match visibility {
                Some(query) => {
                    let (visible, projected_area) = query.visibility_and_area(bounds)?;
                    area = Some(projected_area);
                    visible
                },
                None => frustum_visible(),
            }
        };
        self.texture_area = if root_visible {
            area.unwrap_or_else(|| {
                let projected = crate::render_shadow::projected_box_bounds(
                    &view.view_projection, bounds, false);
                (projected[1][0] - projected[0][0]) * (projected[1][1] - projected[0][1]) * 0.5
            })
        } else { 0. };
        let schedule = self.visibility.prepare(
            &declaration.visibility,
            &declaration.occlusion_kill,
            crate::rendering::particles::visibility::ParticleAdmission {
                bounds,
                camera: camera.position,
                emitter_distance: camera.emitter_distance,
                minimum_angular_size,
                root_visible,
                distance_alpha: crate::float::multiply(self.query_scale[1], alpha),
                distance_size: crate::float::multiply(self.query_scale[0], size),
            },
            delta,
            self.first_simulation,
            self.force_simulate || publication.stop,
        );
        if schedule.kill {
            self.killed = true;
            self.visible_frame = None;
            self.simulated_frame = Some(frame);
            return Ok(());
        }
        if schedule.flush {
            for stage in &mut self.stages {
                stage.flush();
            }
            for stage in &mut self.havok_stages {
                stage.flush();
            }
        }
        if !schedule.update_cycles {
            self.visible_frame = None;
            self.simulated_frame = Some(frame);
            return Ok(());
        }
        // Cycle-only updates require no water clipping query or integration.
        if !schedule.simulate {
            for stage in &mut self.stages {
                stage.advance_cycles(schedule.delta, publication.pose)?;
            }
            for stage in &mut self.havok_stages {
                stage.advance_cycles(schedule.delta)?;
            }
            self.visible_frame = None;
            self.simulated_frame = Some(frame);
            return Ok(());
        }
        let settings = &self.declaration.clipping;
        self.wind = if self.declaration.wind.enabled {
            wind.velocity(world, point_query, publication.pose.position)?
        } else {
            [0.; 3]
        };
        let surface = if settings.use_ocean_plane {
            match world.particle_water_height(
                point_query,
                ray_query,
                publication.pose.position,
                settings.water_volume_raycast_distance,
            )? {
                Some(height) => Some(ParticleWaterSurface::Volume(height)),
                None => ocean_height.map(ParticleWaterSurface::Ocean),
            }
        } else {
            None
        };
        self.simulate(frame, surface, self.source_velocity, schedule.delta)?;
        // bevy_backend: the original starts these at the system's render
        // submission (1401fc340); the FX manager starts them next frame.
        if let Some(origin) = self.publication.as_ref().map(|publication| publication.pose) {
            self.start_fx_events(origin);
        }
        self.visibility.simulated();
        self.visible_frame = schedule.draw.then_some(frame);
        Ok(())
    }

    pub fn simulate(
        &mut self,
        frame: u32,
        surface: Option<ParticleWaterSurface>,
        source_velocity: [f32; 3],
        delta: f32,
    ) -> Result<(), String> {
        if self.simulated_frame == Some(frame) {
            return Ok(());
        }
        let Some(publication) = self.publication.as_ref().filter(|p| p.visible) else {
            return Ok(());
        };
        self.clipping = ParticleClipPlanes::new(&self.declaration.clipping, publication.pose, surface);
        // 140202060: the FX event clocks advance before the stages.
        for event in &mut self.fx_events {
            event.advance(&self.declaration.fx_events[event.index], delta);
        }
        for stage_slot in 0..self.stages.len() {
            let parent_slot = self.stages[stage_slot].parent.filter(|&slot| slot != stage_slot);
            let (stage, parent) = match parent_slot {
                Some(slot) if slot < stage_slot => {
                    let (before, after) = self.stages.split_at_mut(stage_slot);
                    (&mut after[0], Some(&mut before[slot]))
                }
                Some(slot) => {
                    let (before, after) = self.stages.split_at_mut(slot);
                    (&mut before[stage_slot], Some(&mut after[0]))
                }
                None => (&mut self.stages[stage_slot], None),
            };
            let definition = &stage.declaration.stages[stage.index];
            let worldspace = definition.worldspace;
            let mut delta = delta;
            if self.first_simulation {
                stage.emitter.pose = EmitterPose::new(if worldspace {
                    publication.pose
                } else {
                    ReferenceFrame::default()
                });
                delta = crate::gameplay::particles::emitter::warmup_delta(
                    delta,
                    definition.warmup,
                    &mut self.system_seed,
                );
            }
            // 1fe2b0 keeps the descriptor box in local space and translates
            // its world-space counterpart by the system origin, without rotation.
            let bounds = stage.declaration.clipping.bounding_box;
            let bounds = [bounds[1], bounds[0]].map(|bound| {
                if worldspace {
                    std::array::from_fn(|i| {
                        crate::float::add(bound[i], publication.pose.position[i])
                    })
                } else {
                    bound
                }
            });
            let planes = self
                .clipping
                .as_ref()
                .map_or(&[][..], |p| if worldspace { &p.world[..] } else { &p.local[..] });
            stage.advance_frame_with_sources(
                delta,
                &StageFrame {
                    pose: publication.pose,
                    bounds,
                    planes,
                    source_velocity,
                    rate: publication.rate,
                    lod: self.detail_level,
                },
                self.wind,
                self.source_motion.as_ref(),
                parent.map(|parent| ParentStage {
                    particles: &parent.particles,
                    seed: &mut parent.emitter.seed,
                }),
            )?;
            let mut deliveries = std::mem::take(&mut stage.events.pending);
            for delivery in deliveries.drain(..) {
                if let Some(receiver) = self
                    .stages
                    .iter_mut()
                    .find(|stage| stage.index == delivery.receiver)
                {
                    let worldspace = receiver.declaration.stages[receiver.index].worldspace;
                    receiver.particles.enqueue(
                        delivery.input,
                        worldspace,
                        receiver.owner_pose.position,
                        receiver.owner_pose.basis,
                    );
                }
            }
            self.stages[stage_slot].events.pending = deliveries;
            Self::record_fx_events(&mut self.fx_events, &self.declaration, self.stages[stage_slot].events.fx_pending.drain(..));
        }
        // 1401fef80 for each Havok stage, after the ordinary stages. Havok
        // bodies are world space: the descriptor box moves with the origin.
        {
            let bounds = self.declaration.clipping.bounding_box;
            let bounds = [bounds[1], bounds[0]].map(|bound| {
                std::array::from_fn(|i| crate::float::add(bound[i], publication.pose.position[i]))
            });
            let planes = self.clipping.as_ref().map_or(&[][..], |p| &p.world[..]);
            for stage in &mut self.havok_stages {
                stage.advance_frame(
                    delta,
                    &StageFrame {
                        pose: publication.pose,
                        bounds,
                        planes,
                        source_velocity,
                        rate: publication.rate,
                        lod: self.detail_level,
                    },
                    self.source_motion.as_ref(),
                )?;
            }
        }
        // Birth events of the Havok stages' new records.
        Self::route_havok_events(&mut self.havok_stages, &mut self.stages, &mut self.fx_events, &self.declaration);
        {
        }
        let declaration = self.declaration.clone();
        // 1401fde70 for each beam, before the trails. Its first frame
        // (+0x1c9c bit 4) adds the beam's warmup from the system seed.
        for (index, beam) in &mut self.beams {
            let asset = &declaration.beams[*index];
            let delta = if self.first_simulation {
                crate::gameplay::particles::emitter::warmup_delta(delta, asset.timings.warmup, &mut self.system_seed)
            } else {
                delta
            };
            beam.advance(asset, &declaration, delta, publication.pose, &self.beam_target, self.wind,
                self.source_motion.as_ref(), publication.rate, true)?;
        }
        // 140200bf0 for each declaration trail. Its first frame (+0x1c9c
        // bit 4) draws one system seed and starts the pose at the system.
        for (index, trail) in &mut self.trails {
            if self.first_simulation {
                crate::random::lcg_step(&mut self.system_seed);
                trail.reset_pose(publication.pose);
            }
            trail.advance(&declaration.trails[*index], &declaration.wind, delta, publication.pose,
                self.wind, publication.rate, true)?;
        }
        // 1402007b0 for each ribbon, after the trails, with the same warmup.
        for (index, ribbon) in &mut self.ribbons {
            let asset = &declaration.ribbons[*index];
            let delta = if self.first_simulation {
                crate::gameplay::particles::emitter::warmup_delta(delta, asset.timings.warmup, &mut self.system_seed)
            } else {
                delta
            };
            ribbon.advance(asset, &declaration, delta, publication.pose, self.wind,
                self.source_motion.as_ref(), publication.rate, true)?;
        }
        self.first_simulation = false;
        self.simulated_frame = Some(frame);
        Ok(())
    }
}
