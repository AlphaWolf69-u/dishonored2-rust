//! Sensor27 retained acquisition (3b3d10/3b6c90/3b49c0/3b4420).
//! Slots and shape leases belong to the shared physics query owner. This
//! consumer keeps tickets and source camera snapshots, never another world.
use crate::gameplay::{
    collision_queries::QueryId,
    entity_lifetime::EntityId,
    occluded_sensor::{ConvexRequest, Hit, Request, Witness},
    player::grapple::{PreparedSensor, TargetHit},
};
use bevy_math::{Mat3, Vec3};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Movable,
    Pickup,
    Grenade,
}
#[derive(Clone, Copy, Debug)]
pub struct ObjectTarget {
    pub entity: EntityId,
    pub kind: ObjectKind,
    pub movable: bool,
    pub origin: Vec3,
    pub normal: Vec3,
}
#[derive(Clone, Copy, Debug)]
pub struct Acquired {
    pub surface: TargetHit,
    pub object: Option<ObjectTarget>,
}
#[derive(Clone, Copy)]
pub struct Input {
    pub eye: Vec3,
    pub view_axis: Mat3,
    pub player_origin: Vec3,
    pub player_axis: Mat3,
    pub origin: Vec3,
    pub direction: Vec3,
    pub length: f32,
    pub horizontal_range: f32,
    pub vertical_range: f32,
    pub snapping: bool,
    pub pull_objects: bool,
}
pub trait Host {
    fn submit(&mut self, request: Request) -> Result<QueryId, String>;
    fn take(&mut self, id: &mut QueryId, hits: &mut Vec<Hit>) -> Result<bool, String>;
    fn cancel(&mut self, id: &mut QueryId) -> Result<(), String>;
    ///3b4610 is the synchronous refinement of a completed sample ray.
    fn refine(&mut self, request: ConvexRequest, hits: &mut Vec<Hit>) -> Result<(), String>;
    ///3b2810 material/class filtering followed by3b49c0's real entity bit15.
    fn choose(&self, hits: &[Hit], input: Input, radius: f32) -> Result<Option<Acquired>, String>;
}
#[derive(Clone, Copy)]
struct Saved {
    relative_origin: Vec3,
    direction: Vec3,
    distance: f32,
    yaw: f32,
}
pub struct Sensor {
    prepared: Arc<PreparedSensor>,
    vertices: Arc<[[f32; 4]]>,
    primary: QueryId,
    fallback: QueryId,
    samples: Vec<QueryId>,
    input: Option<Input>,
    fallback_input: Option<Input>,
    saved: Option<Saved>,
    saved_pending: Option<Input>,
    hits: Vec<Hit>,
    refined: Vec<Hit>,
}
impl Sensor {
    pub fn submitted_input(&self) -> Option<Input> {
        self.input
    }
    pub fn new(prepared: Arc<PreparedSensor>) -> Self {
        //One support allocation at Start, shared by every outstanding request;
        //no archive access, hull rebuild or shape owner is introduced per frame.
        let vertices = prepared
            .shape(0)
            .iter()
            .enumerate()
            .map(|(index, point)| {
                [
                    point.x,
                    point.y,
                    point.z,
                    f32::from_bits(0x3f00_0000 | index as u32),
                ]
            })
            .collect::<Vec<_>>()
            .into();
        Self {
            vertices,
            samples: vec![QueryId::INVALID; prepared.samples().len()],
            prepared,
            primary: QueryId::INVALID,
            fallback: QueryId::INVALID,
            input: None,
            fallback_input: None,
            saved: None,
            saved_pending: None,
            hits: Vec::new(),
            refined: Vec::new(),
        }
    }
    fn packet(&self, input: Input, extra_layers: u32) -> ConvexRequest {
        ConvexRequest {
            vertices: self.vertices.clone(),
            shape_type: 5,
            radius: 0.,
            transform: [
                input.view_axis.x_axis.extend(0.).to_array(),
                input.view_axis.y_axis.extend(0.).to_array(),
                input.view_axis.z_axis.extend(0.).to_array(),
                input.origin.extend(1.).to_array(),
            ],
            displacement: (input.direction * input.length).to_array(),
            layers: self.prepared.decl.collision_layers | extra_layers,
            required_flags: 0x40,
        }
    }
    ///Native096240 resolves prior submissions before phase4 invokes3b49c0;
    ///new requests are published only afterward by3b6c90. Never resolve here.
    pub fn gather(&mut self, host: &mut impl Host) -> Result<Option<(Input, Acquired)>, String> {
        let Some(input) = self.input else {
            return Ok(None);
        };
        let mut acquired = if host.take(&mut self.primary, &mut self.hits)? {
            host.choose(&self.hits, input, self.prepared.decl.shape_radius)?
        } else {
            None
        };
        let mut fallback_used = false;
        if host.take(&mut self.fallback, &mut self.hits)? {
            if acquired.is_none() {
                if let Some(fallback) = self
                    .fallback_input
                    .filter(|fallback| fallback.direction.z <= input.direction.z)
                {
                    acquired =
                        host.choose(&self.hits, fallback, self.prepared.decl.shape_radius)?;
                    fallback_used = acquired.is_some();
                }
            }
        }
        //All tickets are transferred even if an earlier candidate succeeds.
        //The first successful sample refinement wins, in authored ring order.
        for index in 0..self.samples.len() {
            if !host.take(&mut self.samples[index], &mut self.hits)? {
                continue;
            }
            if acquired.is_some() || !input.snapping {
                continue;
            }
            let Some(position) = self.hits.first().and_then(|hit| match hit.witness {
                Some(Witness::Ray(hit)) => {
                    Some(Vec3::from_array(hit.position[..3].try_into().unwrap()))
                }
                _ => None,
            }) else {
                continue;
            };
            let delta = position - input.origin;
            let refined_input = Input {
                direction: normalized(delta),
                length: delta.length(),
                ..input
            };
            let mut packet = self.packet(refined_input, 0);
            //3b6c90's queued primary uses camera axes;3b4610's immediate
            //sample refinement explicitly reads the player's entity axes.
            packet.transform[0] = input.player_axis.x_axis.extend(0.).to_array();
            packet.transform[1] = input.player_axis.y_axis.extend(0.).to_array();
            packet.transform[2] = input.player_axis.z_axis.extend(0.).to_array();
            host.refine(packet, &mut self.refined)?;
            acquired = host.choose(
                &self.refined,
                refined_input,
                self.prepared.decl.shape_radius,
            )?;
        }
        self.input = None;
        self.fallback_input = None;
        if acquired.is_some_and(|acquired| {
            acquired.surface.position.distance_squared(input.origin)
                < self.prepared.decl.minimum_camera_distance
                    * self.prepared.decl.minimum_camera_distance
        }) {
            acquired = None;
        }
        if let Some(acquired) = acquired {
            if !fallback_used {
                self.saved_pending = Some(input);
            }
            Ok(Some((input, acquired)))
        } else {
            self.saved = None;
            self.saved_pending = None;
            Ok(None)
        }
    }
    ///3b49c0 commits hysteresis only after the real hole/range/posture
    ///admission.3b6c90 then submits the next fallback using that exact point.
    pub fn confirm_target(&mut self, position: Option<Vec3>) {
        let Some(position) = position else {
            self.saved = None;
            self.saved_pending = None;
            return;
        };
        if let Some(input) = self.saved_pending.take() {
            let delta = position - input.origin;
            self.saved = Some(Saved {
                relative_origin: input.origin - input.player_origin,
                direction: normalized(delta),
                distance: delta.length(),
                yaw: input.direction.y.atan2(input.direction.x),
            });
        }
    }
    pub fn submit(&mut self, host: &mut impl Host, input: Input) -> Result<(), String> {
        if self.primary.is_valid()
            || self.fallback.is_valid()
            || self.samples.iter().any(|id| id.is_valid())
        {
            return Err("sensor27 resubmitted before its prior query transfer/cancel".into());
        }
        self.input = Some(input);
        self.primary = host.submit(Request::Convex(self.packet(input, 0x4400)))?;
        let step =
            self.prepared.decl.max_angle_snapping / self.prepared.decl.num_extra_samples as f32;
        let pitch = (-input.direction.z)
            .atan2(input.direction.truncate().length())
            .to_degrees();
        let yaw = input.direction.y.atan2(input.direction.x).to_degrees();
        let sample_axis = if step != 0. && step.is_finite() {
            Mat3::from_rotation_z((yaw - yaw % step).to_radians())
                * Mat3::from_rotation_y((pitch - pitch % step).to_radians())
        } else {
            Mat3::IDENTITY
        };
        for (id, direction) in self.samples.iter_mut().zip(self.prepared.samples()) {
            *id = host.submit(Request::RayClosest {
                origin: input.origin.to_array(),
                direction: (sample_axis * *direction).to_array(),
                maximum_fraction: input.length,
                layers: self.prepared.decl.collision_layers,
                required_flags: 0x40,
            })?;
        }
        if let Some(saved) = self.saved {
            let current_yaw = input.direction.y.atan2(input.direction.x);
            let rotation = Mat3::from_rotation_z(current_yaw - saved.yaw);
            let fallback = Input {
                origin: input.player_origin + rotation * saved.relative_origin,
                direction: rotation * saved.direction,
                length: saved.distance + self.prepared.decl.max_distance_snapping,
                ..input
            };
            self.fallback_input = Some(fallback);
            self.fallback = host.submit(Request::Convex(self.packet(fallback, 0)))?;
        }
        Ok(())
    }
    ///3b4420 cancels every retained ID before3b4360 releases the shapes.
    ///Canceled pending slots retain their lease until the shared worker drains.
    pub fn cancel(&mut self, host: &mut impl Host) -> Result<(), String> {
        if self.input.is_none()
            && !self.primary.is_valid()
            && !self.fallback.is_valid()
            && self.samples.iter().all(|id| !id.is_valid())
        {
            self.saved = None;
            self.saved_pending = None;
            self.fallback_input = None;
            self.hits.clear();
            self.refined.clear();
            return Ok(());
        }
        host.cancel(&mut self.primary)?;
        host.cancel(&mut self.fallback)?;
        for id in &mut self.samples {
            host.cancel(id)?;
        }
        self.input = None;
        self.fallback_input = None;
        self.saved = None;
        self.saved_pending = None;
        self.hits.clear();
        self.refined.clear();
        Ok(())
    }
}
fn normalized(value: Vec3) -> Vec3 {
    value / value.length_squared().max(f32::MIN_POSITIVE).sqrt()
}
