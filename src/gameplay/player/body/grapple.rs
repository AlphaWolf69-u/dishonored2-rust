//! Player-body grapple inputs and beam requests (774680/776840/774d90).
//! Resources and live FX instances remain with their canonical asset owners.
use super::super::Player;
use super::Body;
use bevy_math::{Mat3, Vec3};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Definition {
    pub aliases: [Option<String>; 2],
    pub frame_inputs: [String; 2],
    pub beam_fx: Option<Arc<str>>,
    pub beam_tag: Option<String>,
}
impl Definition {
    /// Body reflection fields +1c0/+1c8/+1e0/+1e8; entry names default in76f960.
    pub fn read(fields: &[(String, crate::entities::Value)]) -> Self {
        let text = |key| crate::entities::scene_motion::atom(fields, key)
            .filter(|value| !value.is_empty() && *value != "NULL");
        Self { aliases: ["m_grappleLeftAnimAlias", "m_grappleUpAnimAlias"].map(|key| text(key).map(str::to_owned)),
            frame_inputs: [("m_grappleLeftFrameEntryName", "grappleLeftFrame"), ("m_grappleUpFrameEntryName", "grappleUpFrame")]
                .map(|(key, fallback)| text(key).unwrap_or(fallback).to_owned()),
            beam_fx: text("m_grappleBeamFX").map(Arc::from), beam_tag: text("m_grappleBeamTag").map(str::to_owned) }
    }
}

pub struct Prepared {
    pub definition: Arc<Definition>,
    pub body_clips: [Option<usize>; 2],
    pub arm_clips: [[Option<usize>; 2]; 2],
    /// The actual left-arm rig tag resolved once during demand preparation.
    pub beam_tag: Option<(usize, Vec3, Mat3)>,
}

#[derive(Clone, Debug)]
pub enum BeamRequest {
    Start { effect: Arc<str>, origin: Vec3, endpoint: Vec3 },
    Update { origin: Vec3, endpoint: Vec3 },
    Stop,
}

pub(super) struct Beam { destination: Vec3, position: Vec3, speed: f32 }

impl Body {
    /// Install only after all bank appends have passed the host's atomic
    /// base-index checks. No archive reads or alternative rig owner here.
    pub fn install_grapple(&mut self, prepared: Prepared) -> Result<(), String> {
        if prepared.definition.aliases.iter().zip(prepared.body_clips).any(|(alias, clip)| alias.is_some() && clip.is_none()) {
            return Err("authored grapple body additive has not been prepared".into());
        }
        if prepared.body_clips.iter().flatten().any(|index| *index >= self.animations.len()) {
            return Err("grapple body clip bank is not admitted".into());
        }
        for (side, clips) in prepared.arm_clips.iter().enumerate() {
            if clips.iter().flatten().any(|index| self.arms.get(side).is_none_or(|arm| *index >= arm.animations.len())) {
                return Err("grapple arm clip bank is not admitted".into());
            }
        }
        if prepared.definition.beam_fx.is_some() {
            let (joint, _, _) = prepared.beam_tag.ok_or("grapple beam has no admitted left-arm tag")?;
            if self.arms.first().is_none_or(|arm| joint >= arm.skeleton.rest.len()) {
                return Err("grapple beam tag joint is outside the actual left-arm rig".into());
            }
        }
        self.grapple.clips = prepared.body_clips;
        for (side, arm) in self.arms.iter_mut().enumerate().take(2) { arm.grapple.clips = prepared.arm_clips[side]; }
        self.grapple_definition = Some(prepared.definition);
        self.grapple_tag = prepared.beam_tag;
        Ok(())
    }

    /// SetGrapple774680 / EndGrapple775260, then ApplyGrapple776840.
    pub fn set_grapple_control(&mut self, values: Option<[f32; 2]>) {
        let (values, blend) = values.map_or(([0.0; 2], 0.0), |values| (values.map(|value| value.clamp(-1.0, 1.0)), 1.0));
        self.grapple.frames = std::array::from_fn(|axis| self.grapple.clips[axis]
            .map_or(0.0, |clip| (values[axis] + 1.0) * (self.animations[clip].frame_count as f32 - 1.0) * 0.5));
        self.grapple.blend = blend;
        for arm in &mut self.arms { arm.grapple.frames = self.grapple.frames; arm.grapple.blend = blend; }
    }
}

impl Player {
    fn grapple_beam_origin(&self) -> Result<Vec3, String> {
        let (joint, offset, _) = self.body.grapple_tag.ok_or("grapple beam tag has not been admitted")?;
        let arm = self.body.arms.first().ok_or("grapple beam has no left arm")?;
        let rows = self.arm_render.iter().find(|render| render.managed_entity.is_none() && render.model == arm.model)
            .and_then(|render| render.joints.get(joint)).ok_or("grapple beam has no published left-arm palette")?;
        let p = Vec3::new(rows[0][3], rows[1][3], rows[2][3]);
        let r = Mat3::from_cols(Vec3::new(rows[0][0], rows[1][0], rows[2][0]),
            Vec3::new(rows[0][1], rows[1][1], rows[2][1]), Vec3::new(rows[0][2], rows[1][2], rows[2][2]));
        Ok(p + r * offset)
    }

    /// 774d90 starts at the actual hand tag; the beam tip initially coincides
    /// with it. A new launch detaches/releases the previous FX first.
    pub fn start_grapple_beam(&mut self, destination: Vec3, speed: f32) -> Result<bool, String> {
        let Some(effect) = self.body.grapple_definition.as_ref().and_then(|definition| definition.beam_fx.clone()) else { return Ok(false) };
        let origin = self.grapple_beam_origin()?;
        self.stop_grapple_beam();
        self.body.grapple_beam = Some(Beam { destination, position: origin, speed });
        self.body.grapple_beam_requests.push(BeamRequest::Start { effect, origin, endpoint: origin });
        Ok(true)
    }

    /// 7766b0 preserves the remaining tip-to-destination distance when an
    /// actual moving endpoint changes; no synthetic target is selected here.
    pub fn retarget_grapple_beam(&mut self, destination: Vec3) -> Result<Option<f32>, String> {
        if self.body.grapple_beam.is_none() { return Ok(None); }
        let origin = self.grapple_beam_origin()?;
        let beam = self.body.grapple_beam.as_mut().unwrap();
        let remaining = beam.destination.distance(beam.position);
        let delta = origin - destination;
        beam.destination = destination;
        beam.position = destination + delta * (remaining / delta.length_squared().max(f32::MIN_POSITIVE).sqrt());
        Ok(Some(remaining))
    }

    /// 776490 advances only from the owning entity clock and retained tag.
    pub(in crate::gameplay::player) fn update_grapple_beam(&mut self, dt: f32) {
        if self.body.grapple_beam.is_none() { return; }
        let Ok(origin) = self.grapple_beam_origin() else { return };
        let beam = self.body.grapple_beam.as_mut().unwrap();
        let remaining = (beam.destination.distance(beam.position) - beam.speed * dt).max(0.0);
        let delta = origin - beam.destination;
        beam.position = beam.destination + delta * (remaining / delta.length_squared().max(f32::MIN_POSITIVE).sqrt());
        self.body.grapple_beam_requests.push(BeamRequest::Update { origin, endpoint: beam.position });
    }

    /// 775210 detaches the body callback before releasing the live FX.
    pub fn stop_grapple_beam(&mut self) {
        if self.body.grapple_beam.take().is_some() { self.body.grapple_beam_requests.push(BeamRequest::Stop); }
    }
    pub fn drain_grapple_beam_requests(&mut self) -> std::vec::Drain<'_, BeamRequest> { self.body.grapple_beam_requests.drain(..) }
    pub fn grapple_beam_endpoint(&self) -> Option<Vec3> { self.body.grapple_beam.as_ref().map(|beam| beam.position) }
}
