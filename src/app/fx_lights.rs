//! FXLight's retained renderer allocation (226060/224eb0).
//!
//! The effect pool slot owns `Light`: disabling a flash withdraws its enabled
//! record without discarding the renderer allocation. Only pool teardown frees it.
use super::{entities, render_light, state::LocalLightCull, view_geometry};
use crate::gameplay::effects::light::LightSample;
use bevy::prelude::*;

/// The actual evaluated native light, also consumed by the ordinary fog and
/// shadow packet owners. It is not an index into the immutable level lights.
#[derive(Component)]
pub(super) struct FxLightRecord {
    pub(super) base: [u8; 0xe8],
    pub(super) renders_in_fog: bool,
    pub(super) descriptor: entities::PlacedLight,
}

#[derive(Default)]
pub(super) struct Light {
    entity: Option<Entity>,
    enabled: bool,
}

impl Light {
    pub(super) fn publish(
        &mut self,
        commands: &mut Commands,
        placed_count: usize,
        slot: u16,
        sample: &LightSample,
    ) -> Result<(), String> {
        let source = super::local_shadow_updates::LocalLightSource::Fx(slot);
        source.key(placed_count)?;
        let kind = match sample.kind {
            0 => entities::LightType::Point,
            2 => entities::LightType::Spot,
            value => return Err(format!("FXLight renderer type {value} is not admitted")),
        };
        let p = &sample.parameters;
        let (cutoff, falloff) = render_light::distance_fields(p[2], p[1]);
        let angles =
            (sample.kind == 2).then(|| view_geometry::authored_light_cosines(p[11], p[10]));
        let converted = render_light::convert(render_light::PresetInput {
            kind: sample.kind,
            preset: sample.preset,
            reflector: 0, //2249f0 passes zero as 3595f0's final parameter.
            area_energy_conservative: false,
            cutoff,
            falloff,
            cutoff_cosine: angles.map_or(0.0, |a| a.0),
            //224ed0 leaves the non-spot stack slots untouched. Do not read them.
            back_reflection: if sample.kind == 2 { p[12] } else { 0.0 },
            area_size: [1.0; 2], //232780's constructor; not read for point/spot.
            color: sample.colour,
        })
        .ok_or("FXLight photometry is not admitted")?;
        if !converted.color.iter().all(|v| v.is_finite())
            || !converted.range.is_finite()
            || !converted.falloff.is_finite()
            || !sample.frame.position.iter().all(|v| v.is_finite())
        {
            return Err("FXLight evaluated record contains non-finite values".into());
        }
        let mut base = [0; 0xe8];
        render_light::initialize_base(&mut base);
        base[0] = sample.flags[0];
        base[1] = sample.flags[1];
        base[0x64] = 1;
        base[0x68] = 1;
        put(&mut base, 0x5c, &[f32::from_bits(sample.kind)]);
        for axis in 0..3 {
            put(&mut base, 4 + axis * 12, &sample.frame.basis[axis]);
        }
        put(&mut base, 0x28, &sample.frame.position);
        put(&mut base, 0x34, &converted.color);
        put(&mut base, 0x4c, &sample.culling_offset);
        put(
            &mut base,
            0x74,
            &[falloff, cutoff, converted.falloff, converted.range],
        );
        put(&mut base, 0x88, &[p[0]]);
        if let Some((outer, inner)) = angles {
            put(&mut base, 0x8c, &[inner, outer]);
        }
        put(&mut base, 0x98, &[p[3], p[4], p[5], p[6], p[7], p[8], p[9]]);
        let descriptor = entities::PlacedLight {
            id: String::new(),
            definition_name_hash: 0,
            culling: entities::LightCulling::DynamicPosition,
            kind,
            position: sample.frame.position,
            rotation: sample.frame.basis,
            orientation_authored: true,
            color: sample.colour,
            indirect_lighting_boost: None,
            back_reflection: Some(if sample.kind == 2 { p[12] } else { 0. }),
            decay: p[0],
            specular_attenuation: p[3],
            distance_cutoff: cutoff,
            distance_falloff: falloff,
            max_visible_range: 0.,
            angular_cutoff: if sample.kind == 2 {
                p[11]
            } else {
                0.5f32.acos().to_degrees() * 2.
            },
            angular_falloff: if sample.kind == 2 {
                p[10]
            } else {
                0.6f32.acos().to_degrees() * 2.
            },
            area_size: [1.; 2],
            area_texture: None,
            area_diffusion: 0.,
            area_projection_direction: [0.; 2],
            area_shadow_source_offset: [0.; 2],
            area_shadow_blur: 0.,
            area_shadow_hardening: 0.,
            area_shadow_range: 0.,
            area_energy_conservative: false,
            no_shadows: sample.flags[0] & 2 == 0,
            cast_volumetric_shadows: sample.flags[0] & 0x20 != 0,
            dont_alter_volumetric_dynamic_shadow: sample.flags[0] & 0x40 != 0,
            scatter_only: sample.flags[1] & 1 != 0,
            scattering: render_light::ScatteringParameters {
                value: p[4],
                origin_offset: p[5],
                shape_scale: [p[6], p[7]],
                offset: p[8],
                turbulence: p[9],
                second_canvas: sample.flags[0] & 0x80 != 0,
                custom_shape: sample.flags[1] & 2 != 0,
            },
            scatter_only_with_particle: false,
            shadow_near_plane: None,
            spot_shadow_quality: None,
            visible: true,
            preset: None,
            reflector_preset: None,
        };
        let position = super::game_to_view(sample.frame.position);
        let direction = super::game_to_view(sample.frame.basis[0]);
        let up = super::game_to_view(sample.frame.basis[1]);
        let transform = if sample.kind == 2 {
            if direction.length_squared() == 0.0 || up.cross(direction).length_squared() < 0.0001 {
                return Err("FXLight spot frame is degenerate".into());
            }
            Transform::from_translation(position).looking_to(direction, up)
        } else {
            Transform::from_translation(position)
        };
        let clip_to_world = view_geometry::local_light_clip_to_world(
            position,
            sample.frame.basis,
            kind,
            converted.range,
            angles.map(|a| a.0),
        );
        let bounds = clip_to_world.and_then(super::light_culling::light_projection_bounds);
        let cull = LocalLightCull {
            range: converted.range,
            kind,
            culling: entities::LightCulling::DynamicPosition,
            definition_name_hash: 0, //Dynamic sources do not bind a static declaration ID.
            linear_color: converted.color,
            projection_bounds: bounds,
            proximity_bounds: None, //FXLight has no idLight max-visible-range field.
            initial_sphere: Some([
                sample.frame.position[0] + sample.culling_offset[0],
                sample.frame.position[1] + sample.culling_offset[1],
                sample.frame.position[2] + sample.culling_offset[2],
                converted.range,
            ]),
            clip_to_world,
        };
        let intensity = view_geometry::light_backend_intensity(1.0, kind, [2.0; 2])
            .ok_or("FXLight host intensity is invalid")?;
        let colour = converted.color;
        let color = Color::linear_rgb(colour[0], colour[1], colour[2]);
        let entity = match self.entity {
            Some(entity) => entity,
            None => {
                let entity = commands
                    .spawn((
                        transform,
                        GlobalTransform::from(transform),
                        Visibility::Hidden,
                    ))
                    .id();
                self.entity = Some(entity);
                entity
            }
        };
        let mut target = commands.entity(entity);
        target.insert((
            transform,
            GlobalTransform::from(transform),
            Visibility::Inherited,
            cull,
            source,
            FxLightRecord {
                base,
                renders_in_fog: sample.renders_in_fog,
                descriptor,
            },
        ));
        if sample.flags[1] & 1 != 0 {
            target.remove::<PointLight>();
            target.remove::<SpotLight>();
        } else if let Some((outer, inner)) = angles {
            target.remove::<PointLight>();
            target.insert(SpotLight {
                color,
                intensity,
                range: converted.range,
                falloff_start: converted.falloff,
                decay: p[0],
                specular_attenuation: p[3],
                outer_angle: outer.acos().clamp(0.01, std::f32::consts::FRAC_PI_2 - 0.01),
                inner_angle: inner
                    .acos()
                    .min(outer.acos().clamp(0.01, std::f32::consts::FRAC_PI_2 - 0.01)),
                shadow_maps_enabled: false, //The canonical shadow owner consumes the retained record.
                ..default()
            });
        } else {
            target.remove::<SpotLight>();
            target.insert(PointLight {
                color,
                intensity,
                range: converted.range,
                falloff_start: converted.falloff,
                decay: p[0],
                specular_attenuation: p[3],
                shadow_maps_enabled: false,
                ..default()
            });
        }
        self.enabled = true;
        Ok(())
    }

    pub(super) fn disable(&mut self, commands: &mut Commands) {
        if !self.enabled {
            return;
        }
        let Some(entity) = self.entity else {
            return;
        };
        commands.queue(move |world: &mut World| {
            let mut target = world
                .get_entity_mut(entity)
                .expect("retained FXLight renderer allocation disappeared");
            if let Some(mut record) = target.get_mut::<FxLightRecord>() {
                record.base[0] &= !1;
            }
            //The ordinary culler writes Visibility, so disabled allocations leave
            //its candidate set without discarding the renderer allocation.
            target.remove::<LocalLightCull>();
            target.insert(Visibility::Hidden);
        });
        self.enabled = false;
    }

    pub(super) fn destroy(&mut self, world: &mut World) -> Result<(), String> {
        if let Some(entity) = self.entity {
            if !world.despawn(entity) {
                return Err("FXLight renderer allocation disappeared".into());
            }
            self.entity = None;
            self.enabled = false;
        }
        Ok(())
    }
}

fn put(record: &mut [u8; 0xe8], offset: usize, values: &[f32]) {
    for (index, value) in values.iter().enumerate() {
        record[offset + index * 4..offset + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
}

pub(super) fn value(record: &[u8; 0xe8], offset: usize) -> f32 {
    f32::from_le_bytes(record[offset..offset + 4].try_into().unwrap())
}

/// Mirrors the 128 real FXLight pool slots. This is a level-scoped renderer
/// allocation owner, not another effect clock or gameplay resource cache.
#[derive(Resource)]
pub(super) struct FxLightAllocations {
    slots: [Light; 128],
    error: Option<String>,
}
impl Default for FxLightAllocations {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| Light::default()),
            error: None,
        }
    }
}

/// Apply this system's deferred commands before the shared light culler,
/// shadow preparation and fog publication. GlobalTransform is supplied with
/// each update, so publication does not await another transform propagation.
pub(super) fn publish(
    mut commands: Commands,
    scene: Res<super::SceneData>,
    simulation: Res<crate::rendering::bevy_backend::camera_scene::FxSimulation>,
    mut allocations: ResMut<FxLightAllocations>,
) {
    if let Err(error) = allocations.publish(
        &mut commands,
        scene.placed_lights.len(),
        simulation.light_publication(),
    ) {
        if allocations.error.as_ref() != Some(&error) {
            error!("FXLight publication failed: {error}");
        }
        allocations.error = Some(error);
    }
}

pub(super) fn error(world: &World) -> Option<&str> {
    world
        .get_resource::<FxLightAllocations>()
        .and_then(|owner| owner.error.as_deref())
}
impl FxLightAllocations {
    /// Run after FX evaluation and apply Commands before ordinary light culling.
    /// The iterator visits only ever-allocated native slots, including disabled ones.
    pub(super) fn publish<'a>(
        &mut self,
        commands: &mut Commands,
        placed_count: usize,
        samples: impl IntoIterator<Item = (u16, Option<&'a LightSample>)>,
    ) -> Result<(), String> {
        for (slot, sample) in samples {
            let target = self
                .slots
                .get_mut(usize::from(slot))
                .ok_or("FXLight native pool slot exceeds128")?;
            match sample {
                Some(sample) => target.publish(commands, placed_count, slot, sample)?,
                None => target.disable(commands),
            }
        }
        Ok(())
    }
    /// Only after the canonical FX owner has disabled/drained every light chain.
    pub(super) fn shutdown(&mut self, world: &mut World) -> Result<(), String> {
        for slot in &mut self.slots {
            slot.destroy(world)?;
        }
        Ok(())
    }
}
