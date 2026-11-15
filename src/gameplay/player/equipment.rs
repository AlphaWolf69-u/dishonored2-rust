//! An equipped entity's own animation tree, cast independently from its arm.
//!
//! 1408542b0 casts equipment entities with masks 8/16 only when they have
//! their own tree (+f0); 1406fea90 evaluates that tree's Midnight layer.
//! The loader supplies the authored animBasic input and attachment tag. No
//! arm pose, guessed tag, or rigid rest pose replaces the equipment's tree.
use super::animation::{AnimBasicCommand, AnimBasicNode, Pose};
use super::midnight::Midnight;
use crate::{assets::animation_rig::Rig, edge_animation::EdgeAnimation, edge_skeleton};
use bevy_math::{Mat3, Vec3};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipmentBinding {
    Body,
    Arm(usize),
}

pub struct EquipmentTree {
    pub rig: Arc<Rig>,
    pub model: String,
    pub animations: Vec<EdgeAnimation>,
    pub input: AnimBasicNode,
    pub binding: EquipmentBinding,
    /// Attachment joint in the parent arm rig, resolved from its actual tag.
    pub bind_joint: usize,
    pub bind: (Vec3, Mat3),
    pub visible: bool,
}

impl EquipmentTree {
    /// The same animBasic command executor as the body and arms (1406e4af0).
    /// The caller provides the declaration's initial/default command; there
    /// is deliberately no implicit idle alias or playback mode here.
    pub fn update(&mut self, dt: f32, command: Option<AnimBasicCommand>) {
        self.input.update(dt, command, &self.animations);
    }

    /// Equipment model-space joints, before the authored arm attachment.
    pub fn pose(
        &self,
        midnight: &mut Midnight,
        target: usize,
    ) -> Result<Vec<(Vec3, Mat3)>, String> {
        let skeleton = &self.rig.skeleton;
        let input = self
            .input
            .evaluate(&self.animations, &skeleton.rest)?
            .unwrap_or_else(|| Pose {
                joints: skeleton.rest.clone(),
                weights: vec![255; skeleton.rest.len()],
            });
        let pose = midnight.evaluate_node(target, input, &self.animations, &skeleton.rest)?;
        let matrices =
            edge_skeleton::pose_matrices(&pose.joints, &skeleton.hierarchy, &self.rig.root)?;
        Ok(matrices
            .iter()
            .map(|m| {
                (
                    Vec3::new(m[0][3], m[1][3], m[2][3]),
                    Mat3::from_cols(
                        Vec3::new(m[0][0], m[1][0], m[2][0]),
                        Vec3::new(m[0][1], m[1][1], m[2][1]),
                        Vec3::new(m[0][2], m[1][2], m[2][2]),
                    ),
                )
            })
            .collect())
    }
}
