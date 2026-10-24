//! The first-person arms (arkCpntPlayerBody arm parts body+0x370 /
//! +0x680): two entities bound to the body's `*_arm_attach_tag` on
//! spine03, each with its own tree (`animtree_player_{left,right}_arm`):
//! the loco node of the arm's equip state (FUN_1407021d0) ⊕ the animBasic
//! jump additives (FUN_1407730c0). BodyPart.Update (FUN_140777180) feeds
//! them the body's speed and angle with a 4/s ramp, and FUN_140775440
//! syncs their phase to the body.
//!
//! The rewrite has no hand equipment, so both arms stay in "Empty". Arm
//! midnight groups, hit reactions, grapple and versus layers and the
//! weapon-lowering bind rotation (identity for empty hands) are not ported.
use super::animation::{blend_add, AnimBasicCommand, AnimBasicNode, LocoNode, Pose};
use crate::edge_animation::EdgeAnimation;
use crate::edge_skeleton::{self, JointRestPose};
use bevy_math::{Mat3, Vec3};

pub struct ArmSkeleton {
    pub rest: Vec<JointRestPose>,
    pub hierarchy: Vec<[u16; 2]>,
    pub parents: Vec<Option<usize>>,
    /// The md6 init offset as the hierarchy root.
    pub root: JointRestPose,
    /// The camera's m_leftArmControlJointName / m_rightArmControlJointName.
    pub shoulder: usize,
}

pub struct Arm {
    /// The arm entity's render model (md6 decl path).
    pub model: String,
    /// m_visible (decl arkBodyPart +0x10).
    pub visible: bool,
    /// The body md6 bind tag: offset translation and rotation on its joint.
    pub bind_joint: usize,
    pub bind: (Vec3, Mat3),
    pub skeleton: ArmSkeleton,
    pub animations: Vec<EdgeAnimation>,
    pub loco: LocoNode,
    pub additives: AnimBasicNode,
    /// The equip state's jump additive items (body+0x90 / +0xa0 table).
    pub jump_items: [Option<usize>; 6],
    /// +0x2b0 m_speedInterpolation.
    speed_interpolation: f32,
}

impl Arm {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: String,
        visible: bool,
        bind_joint: usize,
        bind: (Vec3, Mat3),
        skeleton: ArmSkeleton,
        animations: Vec<EdgeAnimation>,
        loco: LocoNode,
        additives: AnimBasicNode,
        jump_items: [Option<usize>; 6],
    ) -> Self {
        Self { model, visible, bind_joint, bind, skeleton, animations, loco, additives, jump_items, speed_interpolation: 0.0 }
    }

    /// FUN_140777180 / FUN_140776f40 for this part (`midnight`: a full-body
    /// midnight holds the ramp), then the tree's node updates with the
    /// frame's additive command and the phase `rate`.
    pub fn update(&mut self, dt: f32, stance: &str, speed: f32, angle: f32, midnight: bool, rate: f32, command: Option<AnimBasicCommand>) {
        // 4.0 (DAT_141ac8fdc).
        self.speed_interpolation = if midnight { 0.0 } else { (self.speed_interpolation + 4.0 * dt).min(1.0) };
        self.loco.rate_multiplier = rate;
        self.loco.update(dt, stance, speed * self.speed_interpolation, angle, &self.animations);
        self.additives.update(dt, command, &self.animations);
    }

    pub fn dominant_duration(&self) -> f32 {
        self.loco.dominant_duration(&self.animations)
    }

    /// The tree's pose (loco ⊕ additives) as model-space joint transforms of
    /// the arm skeleton (its origin is the bind frame).
    pub fn pose(&self) -> Result<Vec<(Vec3, Mat3)>, String> {
        let s = &self.skeleton;
        let mut pose = match self.loco.evaluate(&self.animations, &s.rest)? {
            Some(pose) => pose,
            None => Pose { joints: s.rest.clone(), weights: vec![255; s.rest.len()] },
        };
        if let Some(additives) = self.additives.evaluate(&self.animations, &s.rest)? {
            blend_add(&mut pose, &additives, 1.0);
        }
        let matrices = edge_skeleton::pose_matrices(&pose.joints, &s.hierarchy, &s.root)?;
        Ok(matrices
            .iter()
            .map(|m| {
                (
                    Vec3::new(m[0][3], m[1][3], m[2][3]),
                    Mat3::from_cols(Vec3::new(m[0][0], m[1][0], m[2][0]), Vec3::new(m[0][1], m[1][1], m[2][1]), Vec3::new(m[0][2], m[1][2], m[2][2])),
                )
            })
            .collect())
    }

    /// Whether `joint` is `ancestor` or below it.
    pub fn descends_from(&self, mut joint: usize, ancestor: usize) -> bool {
        loop {
            if joint == ancestor {
                return true;
            }
            match self.skeleton.parents[joint] {
                Some(parent) => joint = parent,
                None => return false,
            }
        }
    }
}
