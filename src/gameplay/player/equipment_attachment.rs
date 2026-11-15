//! 875e80 ->752900: named model tags, real bone names and empty entity
//! bindings have distinct offset rules. Never replace one with a guessed joint.
use crate::{
    assets::animation_rig::Rig,
    gameplay::reference_frame::{self, ReferenceFrame},
};

#[derive(Clone, Copy)]
pub enum Selection {
    Entity,
    Tag { joint: usize, local: ReferenceFrame },
    Bone { joint: usize },
}

#[derive(Clone, Copy)]
pub struct Binding {
    pub joint: Option<usize>,
    pub local: ReferenceFrame,
}

impl Selection {
    pub fn read(rig: &Rig, tag: &str) -> Result<Self, String> {
        if tag.is_empty() {
            return Ok(Self::Entity);
        }
        if let Some((joint, offset, rotation)) =
            rig.optional_tag_quaternion_in_group("_info", tag)?
        {
            return Ok(Self::Tag {
                joint,
                local: ReferenceFrame {
                    position: offset.to_array(),
                    basis: reference_frame::quaternion_basis(rotation.to_array()),
                },
            });
        }
        Ok(Self::Bone {
            joint: rig.joint(tag)?,
        })
    }
    pub fn joint(self) -> Option<usize> {
        match self {
            Self::Entity => None,
            Self::Tag { joint, .. } | Self::Bone { joint } => Some(joint),
        }
    }
    /// Upgrade61a2b0 supplies explicit zero/identity; ammo44dd00/62b240
    /// supplies null arguments. A real model tag supplies its own values in
    /// either case. Bone-only/null uses the actual spawn world frame.
    pub fn bind(
        self,
        explicit_identity: bool,
        parent: ReferenceFrame,
        joint_at_bind: Option<ReferenceFrame>,
        spawn: ReferenceFrame,
    ) -> Option<Binding> {
        match self {
            Self::Tag { joint, local } => Some(Binding {
                joint: Some(joint),
                local,
            }),
            Self::Entity | Self::Bone { .. } if explicit_identity => Some(Binding {
                joint: self.joint(),
                local: ReferenceFrame::default(),
            }),
            Self::Entity | Self::Bone { .. } => {
                let frame = if self.joint().is_some() {
                    joint_at_bind?
                } else {
                    parent
                };
                let (position, orientation) = reference_frame::relative_binding(frame, spawn);
                Some(Binding {
                    joint: self.joint(),
                    local: ReferenceFrame {
                        position,
                        basis: reference_frame::quaternion_basis(orientation),
                    },
                })
            }
        }
    }
}
impl Binding {
    pub fn world(
        self,
        parent: ReferenceFrame,
        joint: Option<ReferenceFrame>,
    ) -> Option<ReferenceFrame> {
        let parent = if self.joint.is_some() { joint? } else { parent };
        Some(reference_frame::compose(parent, self.local))
    }
}
