//! The player body pose the camera reads `camera_jnt` from.
//!
//! `approximate_stance_pose`: the body plays the locomotion stance's
//! transition animation (`m_stanceTransitions` m_bodyAnim, the same clip
//! whose duration times the posture change, FUN_140771270) and then loops the
//! stance's Idle body animation (`m_stances` / `m_speeds` "Idle" m_body
//! m_front). The game's speed-driven locomotion blend (walk/run/sprint body
//! clips), clip blending and the camera animset bob additive are not ported.
use super::camera::BodyPose;
use super::states::LocoStances;
use crate::edge_animation::EdgeAnimation;
use crate::edge_skeleton::{self, JointRestPose};
use bevy_math::{Mat3, Vec3};
use std::collections::HashMap;

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

pub struct Body {
    pub skeleton: BodySkeleton,
    pub animations: Vec<EdgeAnimation>,
    /// Stance → its Idle body animation.
    pub idle: HashMap<String, usize>,
    /// (from, to) stance → transition body animation.
    pub transitions: HashMap<(String, String), usize>,
    playing: Option<usize>,
    time: f32,
    stance: Option<String>,
}

impl Body {
    pub fn new(skeleton: BodySkeleton, animations: Vec<EdgeAnimation>, idle: HashMap<String, usize>, transitions: HashMap<(String, String), usize>) -> Self {
        Self { skeleton, animations, idle, transitions, playing: None, time: 0.0, stance: None }
    }

    /// Transition durations for the states' posture timing.
    pub fn stance_transition_durations(&self) -> HashMap<(String, String), f32> {
        self.transitions.iter().map(|(key, &index)| (key.clone(), self.animations[index].duration())).collect()
    }

    pub fn update(&mut self, dt: f32, stances: &LocoStances) {
        if stances.current != self.stance {
            let transition = match (&self.stance, &stances.current) {
                (Some(from), Some(to)) => self.transitions.get(&(from.clone(), to.clone())).copied(),
                _ => None,
            };
            self.stance = stances.current.clone();
            self.playing = transition.or_else(|| self.stance.as_ref().and_then(|s| self.idle.get(s).copied()));
            self.time = 0.0;
            return;
        }
        self.time += dt;
        let Some(playing) = self.playing else { return };
        let duration = self.animations[playing].duration();
        let idle = self.stance.as_ref().and_then(|s| self.idle.get(s).copied());
        if Some(playing) != idle {
            if self.time >= duration {
                self.time -= duration;
                self.playing = idle;
            }
        } else if duration > 0.0 && self.time >= duration {
            self.time %= duration;
        }
    }

    pub fn pose(&self) -> Result<BodyPose, String> {
        let skeleton = &self.skeleton;
        let local = match self.playing {
            Some(index) => self.animations[index].sample(&skeleton.rest, self.time)?,
            None => skeleton.rest.clone(),
        };
        let matrices = edge_skeleton::pose_matrices(&local, &skeleton.hierarchy, &skeleton.root)?;
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
