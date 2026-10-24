//! The player body anim tree subset that places `camera_jnt`
//! (`animtree_player_body.animtree`): the loco node (arkAnimTreeNode_Loco,
//! update FUN_1406f4c70) for the body and the camera bob, their blendarray
//! chains (FUN_1406e7cb0) and the Edge joint blends (lerp FUN_1412e6000,
//! additive FUN_1412e4c70) through the command executor (FUN_14024b600).
//!
//! `locoBobAdditives = body ⊕ bobAmount·bob ⊕ additives`. The jump/land
//! additives node (arkAnimTreeNode_AnimBasic), midnight scenes, grapple,
//! versus and lean layers are not ported; lean at its neutral frame and an
//! empty midnight pass the pose through unchanged.
use crate::edge_animation::{slerp, EdgeAnimation};
use crate::edge_skeleton::JointRestPose;
use std::collections::HashMap;

/// A sampled pose with its per-joint weights (0..255).
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub joints: Vec<JointRestPose>,
    pub weights: Vec<u8>,
}

/// Executor op 2: sample at `time`, remove root motion per the clip's
/// m_flags (bits 1..3 root T, bit 4 root R; always on for the body tree), and
/// take the clip's weight bytes (255 when it has none).
pub fn sample(animation: &EdgeAnimation, rest: &[JointRestPose], time: f32) -> Result<Pose, String> {
    let mut joints = animation.sample(rest, time)?;
    if let Some(root) = joints.first_mut() {
        let flags = animation.flags;
        for (bit, axis) in [(1, 0), (2, 1), (3, 2)] {
            if flags & (1 << bit) != 0 {
                root.translation[axis] = 0.0;
            }
        }
        if flags & (1 << 4) != 0 {
            root.rotation = [0.0, 0.0, 0.0, 1.0];
        }
    }
    // Joints past a weighted clip's weight table are not part of its mask.
    let weights = match &animation.joint_weights {
        Some(bytes) => (0..joints.len()).map(|i| bytes.get(i).copied().unwrap_or(0)).collect(),
        None => vec![255; joints.len()],
    };
    Ok(Pose { joints, weights })
}

fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// FUN_1412e6000: per-joint weighted lerp of `b` over `a`.
pub fn blend_lerp(a: &mut Pose, b: &Pose, alpha: f32) {
    for j in 0..a.joints.len().min(b.joints.len()) {
        let (wa, wb) = (a.weights[j] as f32 / 255.0, b.weights[j] as f32 / 255.0);
        let (aw, bw) = (a.weights[j], b.weights[j]);
        let top = wa.max(wb);
        let t = if top > 0.0 { (if wa < wb { (wb - wa) + wa * alpha } else { wb * alpha }) / top } else { 0.0 };
        if aw != 0 && bw != 0 {
            let (ja, jb) = (a.joints[j].clone(), &b.joints[j]);
            a.joints[j] = JointRestPose {
                rotation: slerp(ja.rotation, jb.rotation, t),
                translation: lerp3(ja.translation, jb.translation, t),
                scale: lerp3(ja.scale, jb.scale, t),
            };
        } else if bw != 0 {
            a.joints[j] = b.joints[j].clone();
        }
        a.weights[j] = ((t * wb + (1.0 - t) * wa) * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
}

/// FUN_1412e4c70 (mode 0): the additive pose is applied as an absolute delta.
pub fn blend_add(a: &mut Pose, b: &Pose, alpha: f32) {
    for j in 0..a.joints.len().min(b.joints.len()) {
        let (aw, bw) = (a.weights[j], b.weights[j]);
        if aw == 0 || bw == 0 {
            continue;
        }
        let t = bw as f32 / 255.0 * alpha;
        let ja = a.joints[j].clone();
        let jb = &b.joints[j];
        a.joints[j] = JointRestPose {
            rotation: slerp(ja.rotation, quat_mul(ja.rotation, jb.rotation), t),
            translation: std::array::from_fn(|i| ja.translation[i] + t * jb.translation[i]),
            scale: std::array::from_fn(|i| ja.scale[i] * (t * jb.scale[i] + (1.0 - t))),
        };
        a.weights[j] = (aw as u16 + bw as u16).min(255) as u8;
    }
}

/// FUN_1401a5b00: wrap to (-180, 180].
pub fn wrap180(mut degrees: f32) -> f32 {
    degrees = degrees.rem_euclid(360.0);
    if degrees > 180.0 {
        degrees -= 360.0;
    }
    degrees
}

/// Integer degrees of `angle + 180` in [0, 360), its quadrant and fraction
/// (q: 0 back, 1 right, 2 front, 3 left).
fn quadrant(angle: f32) -> (usize, f32, bool) {
    let a = ((angle + 180.0).rem_euclid(360.0)) as i32 % 360;
    let q = ((a / 90) as usize).min(3);
    (q, (a % 90) as f32 / 90.0, a % 90 != 0)
}

/// One speed definition: clips and clip speeds for back, right, front, left.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeedSet {
    pub clips: [usize; 4],
    pub speeds: [f32; 4],
}

impl SpeedSet {
    /// FUN_140242210: the set's speed at `angle`.
    fn speed_at(&self, angle: f32) -> f32 {
        let (q, f, between) = quadrant(angle);
        let v = self.speeds[q];
        if between && v >= 0.0 { (1.0 - f) * v + f * self.speeds[(q + 1) & 3] } else { v }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub clip: usize,
    pub blend_in: f32,
    pub blend_out: f32,
}

/// Compiled loco decl data for one node (body: m_body/transitions; bob:
/// m_bob, no transitions).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocoData {
    /// stance → speed sets (Idle, Walk, Run, Sprint), `None` when undefined.
    pub stances: HashMap<String, [Option<SpeedSet>; 4]>,
    pub transitions: HashMap<(String, String), Transition>,
}

/// The stance manager (node+0xd08, FUN_1406fcf90/fd5b0/fd760/fd400).
#[derive(Clone, Debug, Default, PartialEq)]
struct StanceManager {
    state: u8,
    blend: f32,
    transition_duration: f32,
    transition_out: f32,
    p: f32,
    p3: f32,
    transition_weight: f32,
    from: Option<String>,
    to: Option<String>,
    transition_from: Option<String>,
    transition: Option<usize>,
    transition_time: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocoNode {
    pub data: LocoData,
    manager: StanceManager,
    /// +0xe48, normalized phase shared by every clip.
    pub progress: f32,
    /// m_walkAnimAdditionalRateMultiplier (+0xbb8).
    pub rate_multiplier: f32,
    /// (clip, weight, clip speed).
    inputs: Vec<(usize, f32, f32)>,
    transition_input: Option<(usize, f32, f32)>,
}

impl LocoNode {
    pub fn new(data: LocoData) -> Self {
        Self {
            data,
            manager: StanceManager::default(),
            progress: 0.0,
            rate_multiplier: 1.0,
            inputs: Vec::new(),
            transition_input: None,
        }
    }

    /// Duration of the max-weight input (m_animDuration).
    pub fn dominant_duration(&self, animations: &[EdgeAnimation]) -> f32 {
        self.inputs
            .iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0.0, |&(clip, _, _)| animations[clip].duration())
    }

    fn manage(&mut self, dt: f32, target: &str, speed: f32, animations: &[EdgeAnimation]) {
        let m = &mut self.manager;
        if m.from.is_none() {
            m.from = Some(target.to_owned());
            m.to = Some(target.to_owned());
        }
        if m.state == 0 && m.to.as_deref() != Some(target) {
            let from = m.from.clone().unwrap_or_default();
            m.to = Some(target.to_owned());
            let _moving = speed != 0.0;
            match self.data.transitions.get(&(from, target.to_owned())) {
                Some(t) => {
                    m.state = 2;
                    m.blend = t.blend_in;
                    m.transition_out = t.blend_out;
                    m.transition_duration = animations[t.clip].duration();
                    m.transition = Some(t.clip);
                    m.transition_time = 0.0;
                }
                // m_defaultTransBlendDuration 0.
                None => {
                    m.state = 1;
                    m.blend = 0.0;
                }
            }
        }
        if m.transition.is_some() {
            m.transition_time += dt;
        }
        match m.state {
            1 | 2 | 4 | 5 => {
                m.p = (m.p + if m.blend > 0.0 { dt / m.blend } else { 1.0 }).min(1.0);
                if m.p >= 1.0 {
                    match m.state {
                        1 => {
                            m.from = m.to.clone();
                            m.state = 0;
                        }
                        2 => {
                            m.transition_from = m.from.clone();
                            m.state = 3;
                            m.p3 = 0.0;
                        }
                        _ => {
                            m.state = 0;
                            m.transition = None;
                        }
                    }
                    m.p = 0.0;
                }
                m.transition_weight = match m.state {
                    2 => m.p,
                    1 => 0.0,
                    _ => 1.0 - m.p,
                };
            }
            3 => {
                m.transition_weight = 1.0;
                let span = m.transition_duration - m.transition_out;
                m.p3 = (m.p3 + dt / span.max(0.1)).min(1.0);
                if m.transition_time >= span {
                    m.blend = m.transition_out;
                    m.from = m.to.clone();
                    // resetProgress = 1 (FUN_140702a50 writes u16 1 at +0x40).
                    m.state = 5;
                    self.progress = 0.0;
                }
            }
            _ => {}
        }
        if m.state == 0 {
            m.transition = None;
        }
    }

    fn stance_list(&self) -> Vec<(String, f32)> {
        let m = &self.manager;
        let from = m.from.clone().unwrap_or_default();
        let to = m.to.clone().unwrap_or_default();
        match m.state {
            1 => vec![(from, 1.0 - m.p), (to, m.p)],
            3 => vec![(m.transition_from.clone().unwrap_or_default(), 1.0 - m.p3), (to, m.p3)],
            5 => vec![(to, 1.0)],
            _ => vec![(from, 1.0)],
        }
    }

    /// One node update (FUN_1406f4c70): stance manager, speed sets,
    /// direction pairs and the shared progress.
    pub fn update(&mut self, dt: f32, target: &str, speed: f32, angle: f32, animations: &[EdgeAnimation]) {
        let speed = speed.max(0.0);
        let angle = wrap180(angle);
        self.manage(dt, target, speed, animations);
        self.inputs.clear();
        let (q, f, between) = quadrant(angle);
        for (stance, weight) in self.stance_list() {
            let Some(sets) = self.data.stances.get(&stance) else { continue };
            let ratio = speed;
            let (mut lower, mut upper) = (None, None);
            let mut previous = -1.0f32;
            for set in sets.iter() {
                if ratio <= previous {
                    break;
                }
                let v = set.as_ref().map_or(-1.0, |s| s.speed_at(angle));
                if (0.0..=ratio).contains(&v) {
                    lower = set.as_ref().map(|s| (s, v));
                }
                if v > ratio {
                    upper = set.as_ref().map(|s| (s, v));
                }
                previous = v;
            }
            let w_lower = match (lower, upper) {
                (Some((_, vl)), Some((_, vu))) => 1.0 - (ratio - vl) / (vu - vl),
                _ => 1.0,
            };
            let mut pair = |set: &SpeedSet, w: f32| {
                self.inputs.push((set.clips[q], (1.0 - f) * w, set.speeds[q]));
                if between {
                    self.inputs.push((set.clips[(q + 1) & 3], f * w, set.speeds[(q + 1) & 3]));
                }
            };
            if let Some((set, _)) = lower {
                pair(set, weight * w_lower);
            }
            if let Some((set, _)) = upper {
                pair(set, weight * (1.0 - w_lower));
            }
        }
        self.inputs.truncate(8);
        // FUN_1406f33b0: one phase for every clip.
        let mut stride = 0.0;
        let mut dominant: Option<(f32, f32)> = None;
        for &(clip, w, clip_speed) in &self.inputs {
            if w > 0.0 {
                let duration = animations[clip].duration();
                stride += w * duration * clip_speed;
                if dominant.is_none_or(|(dw, _)| w > dw) {
                    dominant = Some((w, duration));
                }
            }
        }
        if let Some((_, duration)) = dominant.filter(|&(_, d)| d > 0.0) {
            let mut rate = if speed > 0.0 && stride / speed > 0.0 { duration / (stride / speed) } else { 1.0 };
            rate *= self.rate_multiplier;
            self.progress = (self.progress + dt * rate / duration).rem_euclid(1.0);
        }
        self.transition_input = self.manager.transition.map(|clip| (clip, 0.999 * self.manager.transition_weight, self.manager.transition_time));
    }

    /// The node's blendarray chain: inputs 6..13 then the transition (2),
    /// folded from the last with raw weights as BLEND (lerp) alphas.
    pub fn evaluate(&self, animations: &[EdgeAnimation], rest: &[JointRestPose]) -> Result<Option<Pose>, String> {
        let mut children: Vec<(Pose, f32)> = Vec::new();
        if let Some((clip, weight, time)) = self.transition_input.filter(|&(_, w, _)| w > 1e-4) {
            let animation = &animations[clip];
            children.push((sample(animation, rest, time.min(animation.duration()))?, weight));
        }
        for &(clip, weight, _) in self.inputs.iter().filter(|&&(_, w, _)| w > 1e-4) {
            let animation = &animations[clip];
            // frame = 30 * duration * progress; the executor samples at frame / rate.
            let frame = 30.0 * animation.duration() * self.progress;
            children.push((sample(animation, rest, frame / animation.frame_rate)?, weight));
        }
        // children are in index order (transition 2, then 6..13).
        let mut children = children.into_iter().rev();
        let Some((mut pose, _)) = children.next() else { return Ok(None) };
        for (child, weight) in children {
            if weight >= 1.0 {
                pose = child;
            } else {
                blend_lerp(&mut pose, &child, weight);
            }
        }
        Ok(Some(pose))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joint(t: [f32; 3]) -> JointRestPose {
        JointRestPose { rotation: [0.0, 0.0, 0.0, 1.0], translation: t, scale: [1.0; 3] }
    }

    #[test]
    fn lerp_and_add_follow_the_edge_weight_rules() {
        let mut a = Pose { joints: vec![joint([0.0; 3]), joint([1.0, 0.0, 0.0])], weights: vec![255, 255] };
        let b = Pose { joints: vec![joint([2.0, 0.0, 0.0]), joint([3.0, 0.0, 0.0])], weights: vec![255, 0] };
        blend_lerp(&mut a, &b, 0.25);
        assert!((a.joints[0].translation[0] - 0.5).abs() < 1e-6);
        assert_eq!(a.joints[1].translation[0], 1.0);
        let delta = Pose { joints: vec![joint([0.1, 0.0, 0.0]), joint([5.0, 0.0, 0.0])], weights: vec![255, 0] };
        blend_add(&mut a, &delta, 0.5);
        assert!((a.joints[0].translation[0] - 0.55).abs() < 1e-6);
        assert_eq!(a.joints[1].translation[0], 1.0);
    }

    #[test]
    fn quadrants_put_front_at_zero_degrees() {
        assert_eq!(quadrant(0.0), (2, 0.0, false));
        assert_eq!(quadrant(90.0), (3, 0.0, false));
        assert_eq!(quadrant(-90.0), (1, 0.0, false));
        let (q, f, between) = quadrant(45.0);
        assert_eq!((q, between), (2, true));
        assert!((f - 0.5).abs() < 1e-6);
        assert!((wrap180(270.0) + 90.0).abs() < 1e-6);
    }
}
