//! World/listener integration for the recovered Wwise spatial calculations.
use bevy::prelude::*;
use crate::{assets::wwise::{Attenuation, MediaProperties}, gameplay::sound::{panning::{Rule, Stereo}, spatial, Emitter}};
use std::{collections::HashMap, sync::Arc};
use crate::gameplay::sound::environment::{self, ControlLevels, FilterRay};

#[derive(Clone, Copy)]
pub(super) struct Position {
    pub position: Vec3,
    pub front: Option<Vec3>,
}

pub(super) struct Propagated {
    pub positions: Vec<Position>,
    pub levels: ControlLevels,
}

pub(super) struct Targets {
    pub gains: [f32; 4],
    pub voice_filters: [f32; 2],
    pub output_filters: [Option<f32>; 2],
}

#[derive(Resource)]
pub(super) struct Scene {
    listener: Option<spatial::Listener>,
    minimum: environment::Levels,
    reflection: bool,
    rule: Rule,
    panner: Stereo,
    emitters: HashMap<Emitter, Position>,
    propagated: HashMap<Emitter, Propagated>,
    rates: HashMap<Emitter, (f32, bool)>,
    time_dilation: Option<u32>,
}
impl Default for Scene {
    fn default() -> Self { Self { listener: None, minimum: environment::Levels::default(), reflection: false,
        rule: Rule::Speakers, panner: Stereo::default(),
        emitters: HashMap::new(), propagated: HashMap::new(), rates: HashMap::new(), time_dilation: None } }
}
impl Scene {
    pub fn emitter(&self, emitter: Emitter) -> Option<Position> { self.emitters.get(&emitter).copied() }
    pub fn propagated(&self, emitter: Emitter) -> Option<&Propagated> { self.propagated.get(&emitter) }
    pub fn listener(&self) -> Option<spatial::Listener> { self.listener }
    pub fn minimum(&self) -> environment::Levels { self.minimum }
    pub fn set_rate(&mut self, emitter: Emitter, rate: f32, frozen: bool) { self.rates.insert(emitter, (rate, frozen)); }
    pub fn reflection_normal(&self) -> Option<Vec3> { self.listener.filter(|_| self.reflection).map(|listener| listener.axis.z_axis) }
    pub fn publish(&mut self, emitter: Emitter, position: Position, propagated: Propagated, rate: f32, frozen: bool) {
        self.emitters.insert(emitter, position);
        self.propagated.insert(emitter, propagated);
        self.rates.insert(emitter, (rate, frozen));
    }
    pub fn frozen(&self, emitter: Emitter) -> bool { self.rates.get(&emitter).is_some_and(|(_, frozen)| *frozen) }
    pub fn parameters(&self, emitter: Emitter, parameter: Option<(u32, f32)>) -> Vec<(u32, f32)> {
        let mut parameters = parameter.into_iter().collect::<Vec<_>>();
        if let (Some(id), Some(&(rate, _))) = (self.time_dilation, self.rates.get(&emitter)) {
            // The component's time parameter is written after posting the
            // event (140708b00), and replaces any prior value of the same ID.
            parameters.retain(|(parameter, _)| *parameter != id);
            parameters.push((id, rate));
        }
        parameters
    }
}

pub(super) fn update(mut scene: ResMut<Scene>, cameras: Query<&Transform, With<Camera3d>>,
    player: Option<Res<super::super::player::PlayerState>>, mix: Res<super::AudioMix>,
    clock: Option<Res<crate::gameplay::game_clock_runtime::ClockRuntime>>) {
    scene.emitters.clear();
    scene.propagated.clear();
    scene.rates.clear();
    scene.time_dilation = mix.time_dilation;
    scene.listener = None;
    scene.minimum = environment::Levels::default();
    scene.reflection = false;
    let rate = clock.as_ref().and_then(|clock| clock.clock().ok()).and_then(|clock| clock.rates(3)).map(|rates| rates[1]);
    if let Some(player) = player.as_ref().and_then(|state| state.player.as_ref()) {
        if let Some((listener, settings)) = player.sound_listener() {
            scene.listener = Some(listener);
            scene.minimum = settings.minimum;
            scene.reflection = settings.swap_back_to_front;
        }
        for emitter in [Emitter::PlayerBody, Emitter::PlayerLeftArm, Emitter::PlayerRightArm] {
            if let Some((position, axis)) = player.sound_transform(emitter) {
                scene.emitters.insert(emitter, Position { position, front: Some(axis.x_axis) });
                if let Some(rate) = rate {
                    if let Some(value) = player.sound_rate(emitter, rate) {
                        scene.rates.insert(emitter, (value, rate == 0.0));
                    }
                }
            }
        }
    }
    // G is a host diagnostic camera; ordinary gameplay uses the component's
    // declared skeleton joints above, independently of the rendered camera.
    if player.as_ref().is_some_and(|state| state.debug_free_fly) {
        scene.listener = cameras.single().ok().map(|camera| {
        let game = |value| Vec3::from_array(super::super::view_to_game(value));
        spatial::Listener { position: game(camera.translation),
            axis: Mat3::from_cols(game(*camera.right()), game(*camera.up()), game(*camera.forward())), distance_scale: 1.0 }
        });
    }
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

    pub fn targets(&self, scene: &Scene, position: Option<Position>, channels: u16) -> Result<([f32; 4], [f32; 2]), String> {
        let position = position.ok_or("spatial Wwise event has no emitter position")?;
        self.direction_targets(scene, &[position], channels)
    }

    pub fn propagated_targets(&self, scene: &Scene, propagated: &Propagated, channels: u16,
        settings: &crate::assets::wwise::EnvironmentSettings) -> Result<Targets, String>
    {
        let listener = scene.listener.ok_or("propagated Wwise event has no listener")?;
        let attenuation = self.attenuation.as_deref();
        let cone = attenuation.and_then(|attenuation| attenuation.cone.as_ref());
        let mut matrices = Vec::with_capacity(propagated.positions.len());
        let mut rays = Vec::with_capacity(propagated.positions.len());
        for &position in &propagated.positions {
            let (mut matrix, _) = self.ray_targets(scene, position, channels)?;
            let transmission = environment::gains(settings, propagated.levels,
                spatial::Gains { dry: 1.0, user_auxiliary: 1.0, game_auxiliary: 1.0, cone_ratio: 1.0 });
            for gain in &mut matrix { *gain *= transmission.dry; }
            matrices.push(matrix);
            let ray = listener.ray(position.position, position.front.unwrap_or(Vec3::ZERO), 1.0);
            let gains = spatial::gains(attenuation, ray, cone.map_or(0.0, |cone| cone.outside_volume_db));
            rays.push(FilterRay {
                levels: propagated.levels,
                distance: std::array::from_fn(|index| attenuation.and_then(|attenuation|
                    spatial::curve(attenuation, 3 + index, ray.distance)).unwrap_or(0.0)),
                cone: cone.map_or([0.0; 2], |cone|
                    [gains.cone_ratio * cone.low_pass, gains.cone_ratio * cone.high_pass]),
            });
        }
        let filters = environment::filters(settings, &rays, [0.0; 2])?;
        Ok(Targets { gains: crate::gameplay::sound::panning::directions(matrices),
            voice_filters: filters.voice, output_filters: filters.output })
    }

    /// Mode 2's per-channel maximum and minimum ray filters (1410cbbb0,
    /// 141104200, 1410cb3b0). The level owner supplies its projected positions.
    pub fn direction_targets(&self, scene: &Scene, positions: &[Position], channels: u16) -> Result<([f32; 4], [f32; 2]), String> {
        let [first, remaining @ ..] = positions else { return Err("spatial Wwise event has no admitted position".into()) };
        let first = self.ray_targets(scene, *first, channels)?;
        if remaining.is_empty() { return Ok(first); }
        let mut matrices = vec![first.0];
        let mut filters = [100.0; 2];
        for index in 0..2 { if first.1[index] <= filters[index] { filters[index] = first.1[index]; } }
        for position in remaining {
            let (matrix, next) = self.ray_targets(scene, *position, channels)?;
            matrices.push(matrix);
            for index in 0..2 { if next[index] <= filters[index] { filters[index] = next[index]; } }
        }
        Ok((crate::gameplay::sound::panning::directions(matrices), filters))
    }

    fn ray_targets(&self, scene: &Scene, position: Position, channels: u16) -> Result<([f32; 4], [f32; 2]), String> {
        let listener = scene.listener.ok_or("spatial Wwise event has no listener")?;
        let attenuation = self.attenuation.as_deref();
        let cone = attenuation.and_then(|attenuation| attenuation.cone.as_ref());
        if cone.is_some() && position.front.is_none() { return Err("coned Wwise event has no emitter orientation".into()); }
        let ray = listener.ray(position.position, position.front.unwrap_or(Vec3::ZERO), 1.0);
        let gains = spatial::gains(attenuation, ray, cone.map_or(0.0, |cone| cone.outside_volume_db));
        let spread = spatial::spread(attenuation, ray);
        let focus = spatial::focus(attenuation, ray);
        let pan = match channels {
            1 => [scene.panner.gains(ray.azimuth, spread, focus, scene.rule), [0.0; 2]],
            2 => scene.panner.stereo_gains(ray.azimuth, spread, focus, scene.rule),
            _ => return Err("spatial audio above stereo requires its input speaker layout".into()),
        };
        let filters = spatial::filters(attenuation, ray, gains.cone_ratio,
            cone.map_or([0.0; 2], |cone| [cone.low_pass, cone.high_pass]), [0.0; 2]);
        Ok(([pan[0][0] * gains.dry, pan[0][1] * gains.dry,
            pan[1][0] * gains.dry, pan[1][1] * gains.dry], filters))
    }
}
