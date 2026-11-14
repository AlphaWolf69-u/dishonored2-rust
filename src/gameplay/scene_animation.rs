//! Scene-driven MD6 tracks; immutable resources are shared by instances.
use crate::{archive::Catalog, assets::animation_rig::{Clips, Rig}, edge_animation::EdgeAnimation,
    entities::scene_motion::AnimatedMotionClip, gameplay::midnight_timing::MarkerTiming};
use std::{collections::HashMap, sync::Arc};

struct Clip {
    key: usize,
    animation: EdgeAnimation,
    timing: MarkerTiming,
    priority: i32,
}

pub struct SceneAnimation {
    skeleton: Arc<crate::edge_skeleton::EdgeSkeleton>,
    root: crate::edge_skeleton::JointRestPose,
    clips: Vec<Clip>,
    definition: Arc<AnimatedMotionClip>,
    group: u32,
}

impl SceneAnimation {
    pub fn skeleton(&self) -> &Arc<crate::edge_skeleton::EdgeSkeleton> { &self.skeleton }

    /// Prepare only tracks actually cast by a started Midnight scene. Shared
    /// rigs and Edge clips remain owned by their ordinary resource owners.
    pub fn load(catalog: &Catalog, rig: Arc<Rig>, definition: Arc<AnimatedMotionClip>, group: u32) -> Result<Self, String> {
        let keys = definition.animations.get(&group).ok_or("scene animation group is absent")?;
        let mut loaded = Clips { catalog, rig: &rig, rest: &rig.skeleton.rest,
            animations: Vec::new(), loaded: HashMap::new() };
        let mut clips = Vec::with_capacity(keys.len());
        let mut next_priority = 0;
        for (key_index, key) in keys.iter().enumerate() {
            let index = loaded.clip(&key.anim)?;
            let animation = loaded.animations[index].clone();
            let markers = if key.markers == 0 { &[][..] } else { rig.animation_marker_events(&key.anim)? };
            let fps = animation.frame_rate.ceil() as u32;
            let duration = (animation.duration() * 1000.0) as i32;
            let timing = MarkerTiming::new(key, markers, fps, duration)?;
            let priority = key.priority + next_priority;
            next_priority = priority + 1;
            clips.push(Clip { key: key_index, animation, timing, priority });
        }
        clips.sort_by_key(|clip| clip.priority);
        Ok(Self { skeleton: Arc::clone(&rig.skeleton), root: rig.root.clone(), clips, definition, group })
    }

    /// Evaluate over the rest pose for the currently admitted startup scenes.
    /// General actor scenes must use their live lower tree via `pose_over`.
    pub fn pose(&self, scene_ms: i32) -> Result<crate::gameplay::player::animation::Pose, String> {
        self.pose_over(scene_ms, self.rest_pose())
    }

    /// Midnight is an animation-tree layer. The caller supplies the actor's
    /// live lower pose so an interrupted scene can return to that same tree.
    /// The actor owner must provide a pose from this scene's skeleton; joint
    /// counts alone cannot establish skeleton identity.
    pub fn pose_over(&self, scene_ms: i32, mut pose: crate::gameplay::player::animation::Pose)
        -> Result<crate::gameplay::player::animation::Pose, String> {
        use crate::gameplay::player::animation::{blend_add, blend_lerp, sample};
        let rest = &self.skeleton.rest;
        if pose.joints.len() != rest.len() || pose.weights.len() != rest.len() {
            return Err("scene lower pose does not match its skeleton".into());
        }
        for clip in &self.clips {
            let key = &self.definition.animations[&self.group][clip.key];
            let fps = clip.animation.frame_rate.ceil() as u32;
            let Some(frame) = clip.timing.frame(scene_ms, fps) else { continue };
            let weight = crate::gameplay::player::midnight::weight(&key.curve, scene_ms - key.start_ms);
            if weight <= 0.0 { continue; }
            let last = (clip.animation.frame_count as f32 - 1.0).max(0.0);
            let layer = sample(&clip.animation, rest, frame.min(last) / clip.animation.frame_rate)?;
            if clip.animation.flags & 1 != 0 { blend_add(&mut pose, &layer, weight); }
            else if weight == 1.0 && layer.weights.iter().all(|&joint| joint == 255) { pose = layer; }
            else { blend_lerp(&mut pose, &layer, weight); }
        }
        Ok(pose)
    }

    pub fn rest_pose(&self) -> crate::gameplay::player::animation::Pose {
        let joints = self.skeleton.rest.clone();
        let weights = vec![255; joints.len()];
        crate::gameplay::player::animation::Pose { joints, weights }
    }

    pub fn matrices_from_pose(&self, pose: &crate::gameplay::player::animation::Pose) -> Result<Vec<[[f32; 4]; 3]>, String> {
        crate::edge_skeleton::pose_matrices(&pose.joints, &self.skeleton.hierarchy, &self.root)
    }

    pub fn palette_from_pose(&self, pose: &crate::gameplay::player::animation::Pose) -> Result<Vec<[[f32; 4]; 3]>, String> {
        let current = crate::edge_skeleton::pose_matrices(&pose.joints, &self.skeleton.hierarchy, &self.root)?;
        crate::edge_skeleton::skin_palette(&current, &self.skeleton.inverse_bind)
    }

    pub fn palette(&self, scene_ms: i32) -> Result<Vec<[[f32; 4]; 3]>, String> {
        self.palette_from_pose(&self.pose(scene_ms)?)
    }
}

