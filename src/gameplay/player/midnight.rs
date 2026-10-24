//! Midnight scene playback on the player body: the manager's time step
//! (FUN_1407d2da0, integer ms with a float carry), the CONDMD6 graph walk
//! (FUN_1407ccdc0), the anim track's clip schedule and per-frame write
//! (FUN_1407e2860 / FUN_1407e27b0, weights FUN_14018fe90), EMTT_EVENT point
//! keys (FUN_1407dfdd0), the body tree's midnight node (FUN_1406fea90:
//! clips over the loco pose, snapshot blend FUN_1406fe490) and its root
//! motion (FUN_1406ffde0).
//!
//! Only the groups cast to the player entity (PlayerBody, PlayerCamera;
//! FUN_1408542b0 bit 1) play here: the arm, weapon and dummy entities have
//! no trees in the rewrite. Sfx, barks and timed events are not ported.
use super::animation::{blend_add, blend_lerp, sample, Pose};
use crate::edge_animation::EdgeAnimation;
use crate::edge_skeleton::JointRestPose;
use crate::entities::midnight::{FloatCurve, MidnightScene, Next};
use bevy_math::{Mat3, Quat, Vec3};
use std::sync::Arc;

/// A scene decl with its anim names resolved to body clips.
#[derive(Debug)]
pub struct PreparedScene {
    pub decl: MidnightScene,
    /// Per (scene index, key index) of the player group: the body clip.
    pub clips: Vec<Vec<usize>>,
}

/// The group names cast to the player entity.
pub fn player_group(name: &str) -> bool {
    name == "PlayerBody" || name == "PlayerCamera"
}

impl PreparedScene {
    /// The scene's player group (at most one per scene in the installed
    /// decls).
    pub fn player_group(&self, scene: usize) -> Result<Option<usize>, String> {
        let s = &self.decl.scenes[scene];
        let mut found = None;
        for (i, g) in s.groups.iter().enumerate() {
            if self.decl.group_name(g.group).is_some_and(player_group) {
                if found.is_some() {
                    return Err("a midnight scene with two player groups is not decoded".into());
                }
                found = Some(i);
            }
        }
        Ok(found)
    }
}

/// One scheduled clip (0x98 record): scene ms → anim ms between two
/// segments.
#[derive(Clone, Debug)]
struct Clip {
    anim: usize,
    scene_from: i32,
    scene_to: i32,
    anim_from: i32,
    anim_to: i32,
    priority: i32,
    curve: Option<FloatCurve>,
    main: bool,
}

/// One clip of the frame's midnightData.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClipInfo {
    anim: usize,
    weight: f32,
    frame: f32,
    main: bool,
}

struct Instance {
    handle: u32,
    scene: Arc<PreparedScene>,
    current: usize,
    time: i32,
    abs: i64,
    carry: f32,
    rate: f32,
    clips: Vec<Clip>,
    events: Vec<(i32, String)>,
    md6: String,
}

/// The player's midnight manager subset and the body tree's midnight node.
#[derive(Default)]
pub struct Midnight {
    instance: Option<Instance>,
    next_handle: u32,
    /// The anim track's midnightData of the latest write.
    data: Vec<ClipInfo>,
    /// The previous tree evaluation's inputs, for the root delta.
    previous: Vec<ClipInfo>,
    /// Snapshot blend: pose, elapsed, duration.
    snapshot: Option<(Pose, f32, f32)>,
    pending_snapshot: Option<f32>,
    last: Option<Pose>,
    /// m_midnightWeight: the largest full-body clip weight.
    pub weight: f32,
    /// The frame's root motion: has full body, translation, yaw (degrees).
    pub root: Option<(Vec3, f32)>,
}

/// FUN_14018fe90 with KeyCurve_Float slot 13 (FUN_140182e00).
fn weight(curve: &Option<FloatCurve>, x: i32) -> f32 {
    let Some(c) = curve else { return 1.0 };
    let n = c.times.len();
    if n == 0 {
        return 1.0;
    }
    let stride = (c.values.len() / n).max(1);
    let v = if n == 1 || x < c.times[0] {
        c.values[0]
    } else if x >= c.times[n - 1] {
        c.values[stride * (n - 1)]
    } else {
        let i = (0..n - 1).find(|&i| c.times[i] < x && x <= c.times[i + 1]).unwrap_or(0);
        let span = (c.times[i + 1] - c.times[i]) as f32 * 0.001;
        let u = if span > f32::EPSILON { (x - c.times[i]) as f32 * 0.001 / span } else { (x - c.times[i]) as f32 * 0.001 };
        match c.mode {
            1 => c.values[i],
            2 => c.values[i] + (c.values[i + 1] - c.values[i]) * u,
            4 if stride == 3 => {
                let s = 1.0 - u;
                let (p0, p3) = (c.values[3 * i], c.values[3 * i + 3]);
                let (p1, p2) = (p0 + c.values[3 * i + 2], p3 + c.values[3 * i + 4]);
                s * s * s * p0 + 3.0 * u * s * s * p1 + 3.0 * u * u * s * p2 + u * u * u * p3
            }
            _ => c.values[stride * i],
        }
    };
    v.clamp(0.0, 1.0)
}

impl Midnight {
    /// The main clip of the latest data (mainAnimPtr / mainAnimFrame):
    /// (clip, frame).
    pub fn main_clip(&self) -> Option<(usize, f32)> {
        self.data.iter().rev().find(|c| c.main).map(|c| (c.anim, c.frame))
    }

    pub fn playing(&self) -> Option<u32> {
        self.instance.as_ref().map(|i| i.handle)
    }

    /// FUN_1407cf250(inst, ignoreBlendOut = 1): the scene reached its end.
    pub fn finished(&self, handle: u32) -> bool {
        self.instance.as_ref().is_none_or(|i| i.handle != handle)
    }

    /// FUN_1407cc650: resolve a graph element for the player md6.
    pub fn resolve(scene: &PreparedScene, mut next: Next, md6: &str) -> Result<Option<usize>, String> {
        for _ in 0..16 {
            match next {
                Next::Final => return Ok(None),
                Next::Scene(uid) => {
                    return scene.decl.scenes.iter().position(|s| s.uid == uid).map(Some).ok_or_else(|| format!("missing midnight scene {uid}"));
                }
                Next::Node(uid) => {
                    let node = scene.decl.nodes.iter().find(|n| n.uid == uid).ok_or_else(|| format!("missing midnight node {uid}"))?;
                    if !player_group(&node.group) {
                        return Err(format!("CONDMD6 on group {} is not decoded", node.group));
                    }
                    // The base-compatibility fallback (FUN_1401c8cc0) is not traced.
                    next = node.next.iter().find(|(m, _)| m == md6).map(|(_, n)| *n).ok_or_else(|| format!("no CONDMD6 match for {md6}"))?;
                }
            }
        }
        Err("midnight graph does not reach a scene".into())
    }

    fn enter(&self, prepared: &Arc<PreparedScene>, index: usize, animations: &[EdgeAnimation]) -> Result<(Vec<Clip>, Vec<(i32, String)>), String> {
        let mut clips = Vec::new();
        let mut events = Vec::new();
        if let Some(g) = prepared.player_group(index)? {
            let group = &prepared.decl.scenes[index].groups[g];
            if prepared.clips[index].len() != group.anim_keys.len() {
                return Err("midnight scene was not prepared for this body".into());
            }
            let mut next_priority = 0;
            for (k, key) in group.anim_keys.iter().enumerate() {
                if key.markers > 0 {
                    return Err("midnight clip markers are not decoded".into());
                }
                let anim = prepared.clips[index][k];
                let anim_ms = (animations[anim].duration() * 1000.0) as i32;
                let length = if key.stretched { key.duration_ms } else { anim_ms };
                // FUN_14018f250: the key priority over the clips already scheduled.
                let priority = key.priority + next_priority;
                next_priority = priority + 1;
                clips.push(Clip {
                    anim,
                    scene_from: key.start_ms,
                    scene_to: key.start_ms - 1 + length,
                    anim_from: key.start_offset,
                    anim_to: anim_ms - key.end_offset,
                    priority,
                    curve: key.curve.clone(),
                    main: key.main_clip,
                });
            }
            events = group.events.iter().map(|e| (e.time_ms, e.event.clone())).collect();
        }
        Ok((clips, events))
    }

    /// FUN_1408549c0 → FUN_1407d6040 / FUN_1407d9270: start at t = 0 (the
    /// first data is written now). An interrupted previous scene snapshots.
    pub fn start(&mut self, prepared: &Arc<PreparedScene>, md6: &str, rate: f32, animations: &[EdgeAnimation]) -> Result<u32, String> {
        let Some(index) = Self::resolve(prepared, prepared.decl.first, md6)? else { return Ok(0) };
        let (clips, events) = self.enter(prepared, index, animations)?;
        if let Some(old) = &self.instance {
            self.pending_snapshot = Some(old.scene.decl.blend_out_time);
        }
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        self.instance = Some(Instance { handle: self.next_handle, scene: prepared.clone(), current: index, time: 0, abs: 0, carry: 0.0, rate, clips, events, md6: md6.to_owned() });
        self.write(animations);
        Ok(self.next_handle)
    }

    /// FUN_1407d9590 (flags 0x600): an interrupting stop snapshots the
    /// last pose over the scene's m_blendOutTime.
    pub fn stop(&mut self, handle: u32) {
        if self.instance.as_ref().is_some_and(|i| i.handle == handle) {
            let blend = self.instance.take().map_or(0.0, |i| i.scene.decl.blend_out_time);
            self.data.clear();
            self.pending_snapshot = Some(blend);
        }
    }

    /// The manager update before the component stages: the time step, the
    /// events (returned as (event name, handle)) and the anim data.
    pub fn advance(&mut self, dt: f32, animations: &[EdgeAnimation]) -> Result<Vec<(String, u32)>, String> {
        let mut messages = Vec::new();
        let Some(inst) = self.instance.as_mut() else { return Ok(messages) };
        let length = inst.scene.decl.scenes[inst.current].length_ms;
        let end = length - 1;
        let total = dt * inst.rate + inst.carry;
        let mut ms = (total * 1000.0) as i32;
        let from = inst.time;
        let mut to = from + ms;
        let mut ended = false;
        if to >= end {
            ms = end - from;
            to = end;
            ended = true;
        }
        inst.carry = total - ms as f32 * 0.001;
        let first_step = inst.abs == 0;
        inst.abs += ms.unsigned_abs() as i64;
        inst.time = to;
        for (k, event) in &inst.events {
            let entered = (from <= *k && *k < to) || (*k == to && ended);
            if entered && !(*k == 0 && first_step) {
                messages.push((event.clone(), inst.handle));
            }
        }
        self.write(animations);
        if ended {
            // FUN_1407d2800: the next element; FINAL clears the data with no
            // snapshot.
            let inst = self.instance.as_ref().unwrap();
            let (prepared, md6) = (inst.scene.clone(), inst.md6.clone());
            match Self::resolve(&prepared, prepared.decl.scenes[inst.current].next, &md6)? {
                None => {
                    self.instance = None;
                    self.data.clear();
                }
                Some(index) => {
                    let (clips, events) = self.enter(&prepared, index, animations)?;
                    let inst = self.instance.as_mut().unwrap();
                    (inst.current, inst.time, inst.clips, inst.events) = (index, 0, clips, events);
                    self.write(animations);
                }
            }
        }
        Ok(messages)
    }

    /// FUN_1407e2ad0: the clips covering the scene time with their weights,
    /// ascending by priority.
    fn write(&mut self, animations: &[EdgeAnimation]) {
        self.data.clear();
        let Some(inst) = &self.instance else { return };
        let t = inst.time;
        let mut clips: Vec<&Clip> = inst.clips.iter().filter(|c| c.scene_from <= t && t <= c.scene_to).collect();
        clips.sort_by_key(|c| c.priority);
        for c in clips {
            let frac = if c.scene_to == c.scene_from { 0.0 } else { (t - c.scene_from) as f32 / (c.scene_to - c.scene_from) as f32 };
            let fps = animations[c.anim].frame_rate.ceil();
            let frame = fps * (c.anim_from as f32 * 0.001 + (c.anim_to - c.anim_from) as f32 * frac * 0.001);
            self.data.push(ClipInfo { anim: c.anim, weight: weight(&c.curve, t - c.scene_from), frame, main: c.main });
        }
    }

    /// The tree update of the midnight node: root motion (FUN_1406ffde0)
    /// and the snapshot timer. `dt` is the tree delta.
    pub fn update(&mut self, dt: f32, animations: &[EdgeAnimation], rest: &[JointRestPose]) -> Result<(), String> {
        let full = |c: &ClipInfo| animations[c.anim].flags & 1 == 0 && animations[c.anim].properties & 0xff == 0;
        self.weight = self.data.iter().filter(|c| full(c)).map(|c| c.weight).fold(0.0, f32::max);
        let mut root = None;
        let (mut delta, mut yaw) = (Vec3::ZERO, 0.0f32);
        for c in &self.data {
            if !(c.weight > 0.0 && full(c)) {
                continue;
            }
            let animation = &animations[c.anim];
            if animation.flags & 0x1e == 0 {
                continue;
            }
            root = Some((Vec3::ZERO, 0.0));
            if let Some(p) = self.previous.iter().find(|p| p.anim == c.anim) {
                if c.frame > p.frame {
                    let joint = |frame: f32| -> Result<JointRestPose, String> {
                        Ok(animation.sample(rest, frame / animation.frame_rate)?.swap_remove(0))
                    };
                    let (m0, m1) = (joint(p.frame)?, joint(c.frame)?);
                    let r0 = Mat3::from_quat(Quat::from_array(m0.rotation));
                    let r1 = Mat3::from_quat(Quat::from_array(m1.rotation));
                    let rel = r0.transpose() * r1;
                    let d_yaw = rel.x_axis.y.atan2(rel.x_axis.x) * 57.29578;
                    let d_t = r0.transpose() * (Vec3::from_array(m1.translation) - Vec3::from_array(m0.translation));
                    let w = c.weight;
                    yaw = (1.0 - w) * yaw + w * d_yaw;
                    delta = (1.0 - w) * delta + w * d_t;
                }
            }
        }
        self.root = root.map(|_| (delta, yaw));
        self.previous = self.data.clone();
        if let Some((_, elapsed, duration)) = &mut self.snapshot {
            *elapsed += dt;
            if *elapsed >= *duration {
                self.snapshot = None;
            }
        }
        Ok(())
    }

    /// The midnight node's pose over `loco` (blendarray leafMidnight_06..00,
    /// BLEND or ADD_RIGHT), then the snapshot blender. `last` records the
    /// output for a later snapshot.
    pub fn evaluate(&mut self, loco: Pose, animations: &[EdgeAnimation], rest: &[JointRestPose]) -> Result<Pose, String> {
        let mut pose = loco;
        for c in &self.data {
            if c.weight <= 0.0 {
                continue;
            }
            let animation = &animations[c.anim];
            let last = (animation.frame_count as f32 - 1.0).max(0.0);
            let layer = sample(animation, rest, c.frame.min(last) / animation.frame_rate)?;
            if animation.flags & 1 != 0 {
                blend_add(&mut pose, &layer, c.weight);
            } else if c.weight >= 1.0 {
                pose = layer;
            } else {
                blend_lerp(&mut pose, &layer, c.weight);
            }
        }
        if let Some(duration) = self.pending_snapshot.take() {
            if let Some(last) = self.last.clone() {
                self.snapshot = Some((last, 0.0, duration));
            }
        }
        if let Some((snapshot, elapsed, duration)) = &self.snapshot {
            let factor = if *duration > 0.0 { (1.0 - elapsed / duration).max(0.0) } else { 0.0 };
            if factor > 0.0 {
                blend_lerp(&mut pose, snapshot, factor);
            }
        }
        self.last = Some(pose.clone());
        Ok(pose)
    }
}
