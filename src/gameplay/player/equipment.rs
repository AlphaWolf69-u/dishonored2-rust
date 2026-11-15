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

pub enum EquipmentInput {
    /// The authored animleaf with WRAP_REPEAT, not the animBasic executor's
    /// loop (which subtracts its separate end margin).
    DefaultAnimation {
        /// None when the authored default alias names no clip of the
        /// model's animsets (gun.cpntweapongun's "gun_idle" exists in no
        /// installed animset, yet the original equips the gun): the leaf then
        /// yields the rest pose, as an empty animBasic input does.
        clip: Option<usize>,
        time: f32,
    },
    Basic(AnimBasicNode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipmentBinding {
    Body,
    Arm(usize),
    /// Generic empty-tag binding addresses the parent entity frame, not a joint.
    BodyEntity,
    ArmEntity(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct AttachmentBinding {
    pub owner: EquipmentBinding,
    pub joint: usize,
    pub offset: (Vec3, Mat3),
}

pub struct EquipmentTree {
    pub managed_entity: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub sound_settings: Option<crate::gameplay::sound::component::Settings>,
    pub rig: Arc<Rig>,
    pub model: String,
    pub animations: Vec<EdgeAnimation>,
    pub animation_aliases: std::collections::HashMap<String, usize>,
    pub input: EquipmentInput,
    pub binding: EquipmentBinding,
    pub equipped_attachment: AttachmentBinding,
    pub unequipped_attachment: AttachmentBinding,
    /// Attachment joint in the parent arm rig, resolved from its actual tag.
    pub bind_joint: usize,
    pub bind: (Vec3, Mat3),
    pub visible: bool,
}

impl EquipmentTree {
    /// 61aed0 binds to the actual arm tag; 61b010 rebinds the retained owned
    /// entity to the actual body tag. These are entity frames before MD6 root
    /// offsets, and remain available for holstered audio/effect ownership.
    pub fn bind_equipped(&mut self, equipped: bool) {
        let attachment = if equipped {
            self.equipped_attachment
        } else {
            self.unequipped_attachment
        };
        self.binding = attachment.owner;
        self.bind_joint = attachment.joint;
        self.bind = attachment.offset;
    }
    /// The same animBasic command executor as the body and arms (1406e4af0).
    /// The caller provides the declaration's initial/default command; there
    /// is deliberately no implicit idle alias or playback mode here.
    pub fn update(&mut self, dt: f32, command: Option<AnimBasicCommand>) {
        match &mut self.input {
            EquipmentInput::Basic(input) => input.update(dt, command, &self.animations),
            EquipmentInput::DefaultAnimation { clip: None, time } => *time = 0.0,
            EquipmentInput::DefaultAnimation { clip: Some(clip), time } => {
                let duration = self.animations[*clip].duration();
                *time = if duration > 0.0 {
                    (*time + dt).rem_euclid(duration)
                } else {
                    0.0
                };
            }
        }
    }

    /// Equipment model-space joints, before the authored arm attachment.
    pub fn pose(
        &self,
        midnight: &mut Midnight,
        target: usize,
    ) -> Result<Vec<(Vec3, Mat3)>, String> {
        let skeleton = &self.rig.skeleton;
        let input = self.input_pose()?;
        let pose = midnight.evaluate_node(target, input, &self.animations, &skeleton.rest)?;
        self.pose_matrices(&pose)
    }

    /// Retained inactive entities advance their own authored input. They do
    /// not borrow the currently held entity's Midnight casting target.
    pub fn pose_base(&self) -> Result<Vec<(Vec3, Mat3)>, String> {
        self.pose_matrices(&self.input_pose()?)
    }

    fn input_pose(&self) -> Result<Pose, String> {
        let skeleton = &self.rig.skeleton;
        Ok(match &self.input {
            EquipmentInput::DefaultAnimation { clip: Some(clip), time } => {
                super::animation::sample(&self.animations[*clip], &skeleton.rest, *time)?
            }
            EquipmentInput::DefaultAnimation { clip: None, .. } => Pose {
                joints: skeleton.rest.clone(),
                weights: vec![255; skeleton.rest.len()],
            },
            EquipmentInput::Basic(input) => input
                .evaluate(&self.animations, &skeleton.rest)?
                .unwrap_or_else(|| Pose {
                    joints: skeleton.rest.clone(),
                    weights: vec![255; skeleton.rest.len()],
                }),
        })
    }

    fn pose_matrices(&self, pose: &Pose) -> Result<Vec<(Vec3, Mat3)>, String> {
        let skeleton = &self.rig.skeleton;
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
