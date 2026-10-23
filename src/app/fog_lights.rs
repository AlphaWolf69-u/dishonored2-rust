//! Publish the same admitted level lights and shadow indices used by gameplay.
use super::{
    entities, local_shadow_updates::LocalLightSource, render_light, shadow_pass,
    state::LocalLightCull, SceneData,
};
use crate::rendering::bevy_backend::fog_light_scattering::LocalFogFrame;
use bevy::prelude::*;

pub(super) fn publish(
    scene: Res<SceneData>,
    lights: Query<(
        Entity,
        &LocalLightSource,
        &LocalLightCull,
        &Visibility,
        &GlobalTransform,
    )>,
    cameras: Query<Entity, With<Camera3d>>,
    shadows: Res<shadow_pass::LocalShadowSceneFrame>,
    mut output: ResMut<LocalFogFrame>,
    mut last_error: Local<Option<String>>,
) {
    let result = (|| -> Result<_, String> {
        let camera = cameras.single().map_err(|e| e.to_string())?;
        let mut ordered: Vec<_> = lights
            .iter()
            .filter(|(_, _, _, visibility, _)| **visibility != Visibility::Hidden)
            .collect();
        ordered.sort_by_key(|(_, source, _, _, _)| source.0);
        let receiver = shadows
            .0
            .as_ref()
            .map(|s| s.receiver.as_ref())
            .transpose()
            .map_err(Clone::clone)?;
        let mut packets = Vec::new();
        for (entity, source, cull, _, transform) in ordered {
            let light = &scene.placed_lights[source.0];
            let kind = match light.kind {
                entities::LightType::Point => 0,
                entities::LightType::HalfPoint => 1,
                entities::LightType::Spot => 2,
                entities::LightType::Area => 3,
                _ => continue,
            };
            let (cutoff, falloff) = super::view_geometry::authored_light_cosines(
                light.angular_cutoff,
                light.angular_falloff,
            );
            let value = render_light::scattering_value(
                light.scattering.value,
                kind,
                cull.linear_color,
                (light.angular_falloff * f32::from_bits(0x3c8e_fa35) * 0.5).cos(),
                (light.angular_cutoff * f32::from_bits(0x3c8e_fa35) * 0.5).cos(),
            );
            let linear = Color::srgb(light.color[0], light.color[1], light.color[2]).to_linear();
            let preset =
                super::view_geometry::light_preset(light, [linear.red, linear.green, linear.blue])
                    .ok_or("scattering light preset unavailable")?;
            let mut base = [0u8; 0xe8];
            render_light::initialize_base(&mut base);
            let mut put = |offset: usize, values: &[f32]| {
                for (i, value) in values.iter().enumerate() {
                    base[offset + i * 4..offset + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
                }
            };
            put(4, &light.rotation[0]);
            put(0x10, &light.rotation[1]);
            put(0x1c, &light.rotation[2]);
            put(0x28, &super::view_to_game(transform.translation()));
            put(0x34, &cull.linear_color);
            put(0x7c, &[preset.falloff, cull.range]);
            put(0x88, &[light.decay, falloff, cutoff]);
            put(0x98, &[light.specular_attenuation]);
            put(0xc0, &light.area_size);
            put(
                0xc8,
                &[
                    light.area_diffusion,
                    light.area_projection_direction[0],
                    light.area_projection_direction[1],
                ],
            );
            base[0x5c..0x60].copy_from_slice(&kind.to_le_bytes());
            base[0] = (base[0] & !0x62)
                | if light.no_shadows || kind == 3 { 0 } else { 2 }
                | (u8::from(light.cast_volumetric_shadows) << 5)
                | (u8::from(light.dont_alter_volumetric_dynamic_shadow) << 6);
            let mut scattering = light.scattering;
            scattering.value = value;
            scattering.write_base(&mut base);
            render_light::set_scatter_only(&mut base, light.scatter_only);
            let shadow = receiver.and_then(|r| r.lights.iter().find(|l| l.entity == entity));
            let mut packet = [0; 28];
            let mut first = shadow.map_or(0, |s| s.first);
            render_light::prepare_tlf_lighting_packet(
                &base,
                shadow.map_or(0, |s| s.count),
                false,
                &mut first,
                &mut packet,
            );
            packets.push(packet);
        }
        if packets.len() >= crate::tiled_lighting::LIGHT_BUFFER_CAPACITY {
            return Err("too many admitted scattering lights".into());
        }
        Ok((camera, packets))
    })();
    match result {
        Ok((camera, lights)) => {
            output.camera = Some(camera);
            output.shadow_revision = shadows.0.as_ref().map(|s| s.revision);
            output.lights = std::sync::Arc::new(lights);
            *last_error = None;
        }
        Err(error) => {
            output.camera = None;
            output.lights = Default::default();
            if last_error.as_ref() != Some(&error) {
                warn!("Local fog lights pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}
