//! World/listener integration for the recovered Wwise spatial calculations.
use bevy::prelude::*;
use crate::{assets::wwise::{Attenuation, MediaProperties}, gameplay::sound::{panning::{Rule, Stereo}, spatial}};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy)]
pub(super) struct Position {
    pub position: Vec3,
    pub front: Option<Vec3>,
}

#[derive(Resource)]
pub(super) struct Scene {
    listener: Option<spatial::Listener>,
    rule: Rule,
    panner: Stereo,
}
impl Default for Scene {
    fn default() -> Self { Self { listener: None, rule: Rule::Speakers, panner: Stereo::default() } }
}

pub(super) fn update(mut scene: ResMut<Scene>, cameras: Query<&Transform, With<Camera3d>>) {
    scene.listener = cameras.single().ok().map(|camera| {
        let game = |value| Vec3::from_array(super::super::view_to_game(value));
        spatial::Listener { position: game(camera.translation),
            axis: Mat3::from_cols(game(*camera.right()), game(*camera.up()), game(*camera.forward())), distance_scale: 1.0 }
    });
    scene.rule = if crate::platform::console::shared().read().cvars.int("s_useHeadphoneMode").unwrap_or(0) != 0 {
        Rule::Headphones
    } else { Rule::Speakers };
}

pub(super) struct Voice {
    attenuation: Option<Arc<Attenuation>>,
}

impl Voice {
    pub fn read(properties: &MediaProperties, attenuations: &HashMap<u32, Arc<Attenuation>>) -> Result<Option<Self>, String> {
        let Some(node) = properties.nodes.iter().find(|node| node.positioning_flags & 1 != 0) else { return Ok(None) };
        let Some(positioning) = &node.positioning_3d else { return Ok(None) };
        if positioning.flags & 3 != 1 { return Err("Wwise automated positioning requires its path runtime".into()); }
        let attenuation = if positioning.attenuation == 0 { None } else {
            Some(attenuations.get(&positioning.attenuation).cloned()
                .ok_or_else(|| format!("missing Wwise attenuation {:#010x}", positioning.attenuation))?)
        };
        if attenuation.as_ref().is_some_and(|attenuation| !attenuation.rtpcs.is_empty()) {
            return Err("Wwise attenuation RTPCs require spatial property evaluation".into());
        }
        Ok(Some(Self { attenuation }))
    }

    pub fn targets(&self, scene: &Scene, position: Option<Position>) -> Result<([f32; 2], [f32; 2]), String> {
        let position = position.ok_or("spatial Wwise event has no emitter position")?;
        let listener = scene.listener.ok_or("spatial Wwise event has no listener")?;
        let attenuation = self.attenuation.as_deref();
        let cone = attenuation.and_then(|attenuation| attenuation.cone.as_ref());
        if cone.is_some() && position.front.is_none() { return Err("coned Wwise event has no emitter orientation".into()); }
        let ray = listener.ray(position.position, position.front.unwrap_or(Vec3::ZERO), 1.0);
        let gains = spatial::gains(attenuation, ray, cone.map_or(0.0, |cone| cone.outside_volume_db));
        let pan = scene.panner.gains(ray.azimuth, spatial::spread(attenuation, ray), spatial::focus(attenuation, ray), scene.rule);
        let filters = spatial::filters(attenuation, ray, gains.cone_ratio,
            cone.map_or([0.0; 2], |cone| [cone.low_pass, cone.high_pass]), [0.0; 2]);
        Ok(([pan[0] * gains.dry, pan[1] * gains.dry], filters))
    }
}
