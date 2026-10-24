//! arkCpntPlayerBody: the DEFAULT-stage loco inputs (FUN_140777280), the
//! body and bob loco nodes with their phase sync (FUN_140775440) and the
//! pose the camera reads `camera_jnt` from (`animation.rs`).
//!
//! The jump/land camera additives (animBasic node), arms, footsteps, follow
//! slope and the swim branch are not ported.
use super::animation::{blend_add, wrap180, LocoNode, Pose};
use super::camera::BodyPose;
use super::states::LocoStances;
use crate::edge_animation::EdgeAnimation;
use crate::edge_skeleton::{self, JointRestPose};
use bevy_math::{Mat3, Vec3};

pub struct BodySkeleton {
    pub rest: Vec<JointRestPose>,
    pub hierarchy: Vec<[u16; 2]>,
    pub parents: Vec<Option<usize>>,
    /// The md6 init offset as the hierarchy root.
    pub root: JointRestPose,
    pub camera: usize,
    /// head, neck, arms control joints.
    pub controls: [usize; 3],
}

/// arkDeclCpntPlayerBody fields the port uses (none are overridden).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyDecl {
    pub speed_blend_duration: f32,
    pub sync_duration: f32,
    pub min_sync_playrate: f32,
    pub max_sync_playrate: f32,
}

impl Default for BodyDecl {
    fn default() -> Self {
        Self { speed_blend_duration: 0.1, sync_duration: 0.5, min_sync_playrate: 0.75, max_sync_playrate: 1.5 }
    }
}

/// The cvars the body reads: `ark_playerSmoothedLoco` and the head-bob
/// option `ark_playerBobAmount`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodySettings {
    pub smoothed_loco: bool,
    pub bob_amount: f32,
}

impl Default for BodySettings {
    fn default() -> Self {
        Self { smoothed_loco: true, bob_amount: 1.0 }
    }
}

/// arkTimeSmoothedValue<8> (FUN_140770040 / FUN_140770680): a moving
/// average of 1/30 s buckets.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Smooth8 {
    index: usize,
    ring: [f32; 8],
    remaining: f32,
    accum: f32,
    count: u32,
}

const PERIOD: f32 = f32::from_bits(0x3d08_8889);

impl Default for Smooth8 {
    fn default() -> Self {
        Self { index: 7, ring: [0.0; 8], remaining: PERIOD, accum: 0.0, count: 1 }
    }
}

impl Smooth8 {
    fn push(&mut self, value: f32, dt: f32) -> f32 {
        self.accum += value;
        self.count += 1;
        self.remaining -= dt;
        if self.remaining <= 0.0 {
            self.index = (self.index + 1) % 8;
            self.ring[self.index] = self.accum / self.count as f32;
            self.remaining += PERIOD;
            while self.remaining <= 0.0 {
                self.remaining += PERIOD;
                self.index = (self.index + 1) % 8;
                self.ring[self.index] = value;
            }
            self.accum = value;
            self.count = 1;
        }
        self.value()
    }

    fn value(&self) -> f32 {
        let sum: f32 = self.ring.iter().sum();
        let snap = |v: f32| if v.abs() <= 1e-18 { 0.0 } else { v };
        let full = snap(sum / 8.0);
        let shifted = snap((sum - self.ring[(self.index + 1) % 8] + self.accum / self.count as f32) / 8.0);
        let k = self.remaining / PERIOD;
        (1.0 - k) * shifted + k * full
    }
}

/// FUN_1407721f0: angle smoothing on the unwrapped angle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SmoothAngle {
    smooth: Smooth8,
    last: f32,
    unwrapped: f32,
}

impl SmoothAngle {
    fn push(&mut self, angle: f32, dt: f32) -> f32 {
        self.unwrapped += wrap180(angle - self.last);
        self.last = angle;
        wrap180(self.smooth.push(self.unwrapped, dt))
    }
}

pub struct Body {
    pub skeleton: BodySkeleton,
    pub animations: Vec<EdgeAnimation>,
    pub decl: BodyDecl,
    pub body_node: LocoNode,
    pub bob_node: LocoNode,
    speed_smooth: Smooth8,
    angle_smooth: SmoothAngle,
    move_timer: f32,
    /// m_speedInterpolation (+0x2b0), ramped in at 4/s.
    speed_interpolation: f32,
    pub speed: f32,
    pub angle: f32,
    bob_amount: f32,
}

impl Body {
    pub fn new(skeleton: BodySkeleton, animations: Vec<EdgeAnimation>, decl: BodyDecl, body_node: LocoNode, bob_node: LocoNode) -> Self {
        Self {
            skeleton,
            animations,
            decl,
            body_node,
            bob_node,
            speed_smooth: Smooth8::default(),
            angle_smooth: SmoothAngle::default(),
            move_timer: 0.0,
            speed_interpolation: 0.0,
            speed: 0.0,
            angle: 0.0,
            bob_amount: 1.0,
        }
    }

    /// Transition durations for the states' posture timing (FUN_140771270).
    pub fn stance_transition_durations(&self) -> std::collections::HashMap<(String, String), f32> {
        self.body_node.data.transitions.iter().map(|(key, t)| (key.clone(), self.animations[t.clip].duration())).collect()
    }

    /// DEFAULT stage FUN_140777280 then the tree's node updates.
    /// `velocity` is FUN_1407fdb50 (entity velocity minus physics gravity);
    /// `jumping` is a non-zero jump stage.
    pub fn update(&mut self, dt: f32, stances: &LocoStances, velocity: Vec3, axis: Mat3, jumping: bool, settings: BodySettings) {
        let mut speed = velocity.length();
        if jumping {
            speed = 0.0;
        }
        let mut angle = 0.0;
        if velocity.truncate().length() < 0.001 {
            speed = 0.0;
            self.move_timer = 0.0;
        } else {
            if self.move_timer < self.decl.speed_blend_duration {
                speed = 0.0;
                self.move_timer += dt;
            }
            let yaw = |v: Vec3| v.y.atan2(v.x).to_degrees().rem_euclid(360.0);
            angle = wrap180(yaw(velocity) - yaw(axis.x_axis));
        }
        // Clamp by the navigation state's {back, strafe, fwd, strafe}.
        let a = ((angle + 180.0).rem_euclid(360.0)) as i32 % 360;
        let q = ((a / 90) as usize).min(3);
        let mut max = stances.max_speeds[q];
        if a % 90 != 0 {
            let f = (a % 90) as f32 / 90.0;
            max = f * stances.max_speeds[(q + 1) & 3] + (1.0 - f) * max;
        }
        speed = speed.min(max);
        if settings.smoothed_loco {
            speed = self.speed_smooth.push(speed, dt);
            angle = self.angle_smooth.push(angle, dt);
        }
        self.speed = speed;
        self.angle = angle;
        self.speed_interpolation = (self.speed_interpolation + 4.0 * dt).min(1.0);
        self.bob_amount = settings.bob_amount.clamp(0.0, 1.0);
        self.phase_sync();
        let body_stance = stances.current.clone().unwrap_or_default();
        let bob_stance = stances.bob.clone().unwrap_or_else(|| body_stance.clone());
        self.body_node.update(dt, &body_stance, speed * self.speed_interpolation, angle, &self.animations);
        self.bob_node.update(dt, &bob_stance, speed, angle, &self.animations);
    }

    /// FUN_140775440 / FUN_140775610 for the bob slave.
    fn phase_sync(&mut self) {
        let d = self.body_node.dominant_duration(&self.animations);
        let slave = self.bob_node.dominant_duration(&self.animations);
        let mut rate = 1.0;
        if d > 0.0 && (d - slave).abs() < 0.001 {
            let (pm, ps) = (self.body_node.progress, self.bob_node.progress);
            if (pm * d - ps * d).abs() >= 0.001 {
                let sync = self.decl.sync_duration;
                let mut x = (sync + pm * d).rem_euclid(d);
                x = if x < ps * d { (d - ps * d) + x } else { x - ps * d };
                rate = (x / sync).clamp(self.decl.min_sync_playrate, self.decl.max_sync_playrate);
            }
        }
        self.bob_node.rate_multiplier = rate;
    }

    /// The model-space pose: body ⊕ bobAmount·bob.
    pub fn pose(&self) -> Result<BodyPose, String> {
        let skeleton = &self.skeleton;
        let mut pose = match self.body_node.evaluate(&self.animations, &skeleton.rest)? {
            Some(pose) => pose,
            None => Pose { joints: skeleton.rest.clone(), weights: vec![255; skeleton.rest.len()] },
        };
        if self.bob_amount > 1e-4 {
            if let Some(bob) = self.bob_node.evaluate(&self.animations, &skeleton.rest)? {
                blend_add(&mut pose, &bob, self.bob_amount);
            }
        }
        let matrices = edge_skeleton::pose_matrices(&pose.joints, &skeleton.hierarchy, &skeleton.root)?;
        Ok(BodyPose {
            positions: matrices.iter().map(|m| Vec3::new(m[0][3], m[1][3], m[2][3])).collect(),
            rotations: matrices
                .iter()
                .map(|m| {
                    Mat3::from_cols(
                        Vec3::new(m[0][0], m[1][0], m[2][0]),
                        Vec3::new(m[0][1], m[1][1], m[2][1]),
                        Vec3::new(m[0][2], m[1][2], m[2][2]),
                    )
                })
                .collect(),
            parents: skeleton.parents.clone(),
            camera: skeleton.camera,
            controls: skeleton.controls,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoother_converges_to_a_constant_input() {
        let mut s = Smooth8::default();
        let mut v = 0.0;
        for _ in 0..240 {
            v = s.push(3.6, 1.0 / 120.0);
        }
        assert!((v - 3.6).abs() < 1e-4, "{v}");
    }
}
