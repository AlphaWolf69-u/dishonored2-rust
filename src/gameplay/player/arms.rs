//! The first-person arms (arkCpntPlayerBody arm parts body+0x370 /
//! +0x680): two entities bound to the body's `*_arm_attach_tag` on
//! spine03, each with its own tree (`animtree_player_{left,right}_arm`):
//! the loco node of the arm's equip state (FUN_1407021d0) ⊕ the animBasic
//! jump additives (FUN_1407730c0). BodyPart.Update (FUN_140777180) feeds
//! them the body's speed and angle with a 4/s ramp, and FUN_140775440
//! syncs their phase to the body.
//!
//! The tree's midnight node takes the PlayerLeftArm / PlayerRightArm scene
//! groups (cast target 1 / 2) over loco ⊕ additives.
//!
//! Equipment locomotion uses the authored stance/equipment pairs and
//! transitions and paused grapple leaves. Hit reactions/versus and weapon-lowering
//! bind rotation remain separate layers.
use super::animation::{blend_add, AnimBasicCommand, AnimBasicNode, LocoNode, Pose};
use super::midnight::Midnight;
use crate::edge_animation::EdgeAnimation;
use crate::edge_skeleton::{self, JointRestPose};
use bevy_math::{Mat3, Vec3};

pub struct ArmSkeleton {
    pub data: std::sync::Arc<edge_skeleton::EdgeSkeleton>,
    /// The md6 init offset as the hierarchy root.
    pub root: JointRestPose,
    /// The camera's m_leftArmControlJointName / m_rightArmControlJointName.
    pub shoulder: usize,
}

impl std::ops::Deref for ArmSkeleton {
    type Target = edge_skeleton::EdgeSkeleton;
    fn deref(&self) -> &Self::Target { &self.data }
}

pub struct Arm {
    pub managed_entity: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub entity_declaration: String,
    pub entity_class: String,
    pub bind_tag: String,
    pub ai_tampering_declared: bool,
    pub rig: Option<std::sync::Arc<crate::assets::animation_rig::Rig>>,
    pub sound_settings: Option<crate::gameplay::sound::component::Settings>,
    pub animation_sounds: super::midnight::AnimationSoundBank,
    /// The arm entity's render model (md6 decl path).
    pub model: String,
    /// m_visible (decl arkBodyPart +0x10).
    pub visible: bool,
    /// The body md6 bind tag: offset translation and rotation on its joint.
    pub bind_joint: usize,
    pub bind: (Vec3, Mat3),
    pub skeleton: ArmSkeleton,
    pub animations: Vec<EdgeAnimation>,
    /// Stable indices in this arm's bank, retained for demanded acquisition.
    pub animation_aliases: std::collections::HashMap<String, usize>,
    pub loco: LocoNode,
    pub additives: AnimBasicNode,
    pub grapple: super::animation::PausedAdditives,
    /// The equip state's jump additive items (body+0x90 / +0xa0 table).
    pub jump_items: [Option<usize>; 6],
    /// +0x2b0 m_speedInterpolation.
    speed_interpolation: f32,
    equipment: String,
    equipment_stances: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    equipment_jump_items: std::collections::HashMap<String, [Option<usize>; 6]>,
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
        let stances = loco.data.stances.keys().map(|stance| (stance.clone(), stance.clone())).collect();
        Self { managed_entity: None, entity_declaration: String::new(), entity_class: String::new(), bind_tag: String::new(), ai_tampering_declared: false, rig: None, sound_settings: None, animation_sounds: Default::default(), model, visible, bind_joint, bind, skeleton, animations, animation_aliases: Default::default(), loco, additives, grapple: Default::default(), jump_items, speed_interpolation: 0.0,
            equipment: "Empty".into(), equipment_stances: [("Empty".into(), stances)].into(),
            equipment_jump_items: [("Empty".into(), jump_items)].into() }
    }

    /// FUN_140777180 / FUN_140776f40 for this part (`midnight`: a full-body
    /// midnight holds the ramp), then the tree's node updates with the
    /// frame's additive command and the phase `rate`.
    pub fn update(&mut self, dt: f32, stance: &str, speed: f32, angle: f32, midnight: bool, rate: f32, command: Option<AnimBasicCommand>) {
        // 4.0 (DAT_141ac8fdc).
        self.speed_interpolation = if midnight { 0.0 } else { (self.speed_interpolation + 4.0 * dt).min(1.0) };
        self.loco.rate_multiplier = rate;
        let key = self.equipment_stances.get(&self.equipment).and_then(|stances| stances.get(stance)).map(String::as_str).unwrap_or(stance);
        self.loco.update(dt, key, speed * self.speed_interpolation, angle, &self.animations);
        self.additives.update(dt, command, &self.animations);
    }

    /// Install only after preparation completes; cancelled preparations never
    /// change the live node. All indices belong to this arm's clip owner.
    pub fn install_equipment(&mut self, prepared: PreparedEquipment) {
        self.animation_sounds.absorb(prepared.animation_sounds);
        self.loco.data.idle_blend = prepared.data.idle_blend;
        self.loco.data.stances.extend(prepared.data.stances);
        self.loco.data.transitions.extend(prepared.data.transitions);
        self.loco.data.idle_clips.extend(prepared.data.idle_clips);
        self.equipment_stances.insert(prepared.name.clone(), prepared.stances);
        self.equipment_jump_items.insert(prepared.name, prepared.jump_items);
    }

    /// Commit a worker result only against the bank it captured. A concurrent
    /// scene preparation can append clips while this demand is in flight;
    /// refusing that stale result protects every live scene's indices.
    pub fn append_equipment(&mut self, base: usize, prepared: PreparedEquipment,
        animations: Vec<EdgeAnimation>, aliases: std::collections::HashMap<String, usize>) -> Result<(), String> {
        if self.animations.len() != base { return Err("arm clip bank changed during equipment preparation".into()); }
        if aliases.values().any(|index| *index >= animations.len()) { return Err("equipment worker returned an invalid clip index".into()); }
        self.animations.extend(animations);
        self.animation_aliases.extend(aliases.into_iter().map(|(alias, index)| (alias, base + index)));
        self.install_equipment(prepared);
        Ok(())
    }

    pub fn equipment(&self) -> &str { &self.equipment }
    pub fn has_equipment(&self, name: &str) -> bool { self.equipment_stances.contains_key(name) }

    /// Admission follows preparation: never show an unprepared/rest-pose hand.
    pub fn request_equipment(&mut self, name: &str) -> bool {
        if !self.equipment_stances.contains_key(name) { return false; }
        self.equipment.clear();
        self.equipment.push_str(name);
        self.jump_items = self.equipment_jump_items[name];
        true
    }

    pub fn dominant_duration(&self) -> f32 {
        self.loco.dominant_duration(&self.animations)
    }

    /// The tree's pose (loco ⊕ additives, then the midnight node of cast
    /// `target`) as model-space joint transforms of the arm skeleton (its
    /// origin is the bind frame).
    pub fn pose(&self, midnight: &mut Midnight, target: usize) -> Result<Vec<(Vec3, Mat3)>, String> {
        let s = &self.skeleton;
        let mut pose = match self.loco.evaluate(&self.animations, &s.rest)? {
            Some(pose) => pose,
            None => Pose { joints: s.rest.clone(), weights: vec![255; s.rest.len()] },
        };
        if let Some(additives) = self.additives.evaluate(&self.animations, &s.rest)? {
            blend_add(&mut pose, &additives, 1.0);
        }
        let mut pose = midnight.evaluate_node(target, pose, &self.animations, &s.rest)?;
        self.grapple.apply(&mut pose, &self.animations, &s.rest)?;
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

/// One demanded arm equip state, resolved through the existing Rig/Clips owner.
/// The host supplies an immutable alias-index snapshot, prepares missing
/// clips in a separate bank and appends it atomically at `base`; no model, skeleton, archive cache or second pose owner exists.
pub struct PreparedEquipment {
    pub(crate) animation_sounds: super::midnight::AnimationSoundBank,
    name: String,
    data: super::animation::LocoData,
    stances: std::collections::HashMap<String, String>,
    jump_items: [Option<usize>; 6],
}
impl PreparedEquipment {
    pub fn visit_lower_animation_sounds(&self,mut visit:impl FnMut(&super::midnight::AnimationSound)) {
        self.animation_sounds.visit(|_,sound|visit(sound));
    }
    pub fn visit_animation_effects(&self,mut visit:impl FnMut(&crate::assets::animation_rig::animset_effects::Effect)) {
        self.animation_sounds.visit_effects(|_,effect|visit(effect));
    }
}

fn equip_key(stance: &str, equipment: &str) -> String {
    // Host key for the original manager's (stance index, equip index) pair.
    if equipment == "Empty" { stance.to_owned() } else { format!("{stance}\0{equipment}") }
}

/// Compact immutable demand metadata extracted while player declarations are
/// borrowed at setup. Only this hand's aliases and authored numeric values
/// survive; the large generic locomotion/body declaration trees do not.
#[derive(Clone, Debug)]
pub struct EquipmentDefinition {
    pub name: String,
    pub stances: Vec<EquipmentStance>,
    pub stance_transitions: Vec<(String, String, String, f32, f32)>,
    pub equip_transitions: Vec<super::decls::ArmEquipTransition>,
    pub jump_aliases: [Option<String>; 6],
    pub idle_blend: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct EquipmentStance {
    pub name: String,
    pub speeds: Vec<(usize, [String; 4], [f32; 4])>,
    pub idle: Option<String>,
}

impl EquipmentDefinition {
    pub fn read(locomotion: &[(String, crate::entities::Value)], body: &[(String, crate::entities::Value)], side: usize, name: &str) -> Result<Self, String> {
        use super::decls;
        if side > 1 || !decls::arm_equipment_names(locomotion, side).iter().any(|value| value == name) { return Err(format!("arm {side} has no equipment state {name}")); }
        let stances = decls::loco_stances(locomotion, [name; 2])?.into_iter().map(|(stance, speeds)| {
            let idle = decls::loco_arm_idle(locomotion, &stance, side, name);
            EquipmentStance { name: stance, idle, speeds: speeds.into_iter().filter_map(|entry|
                entry.arms[side].clone().map(|names| (entry.speed, names, entry.speeds))).collect() }
        }).collect();
        Ok(Self { name: name.to_owned(), stances,
            stance_transitions: decls::loco_arm_transitions(locomotion, side, name)?,
            equip_transitions: decls::arm_equip_transitions(locomotion, side)?.into_iter().filter(|entry| entry.from == name || entry.to == name).collect(),
            jump_aliases: decls::arm_jump_aliases(body, side, name), idle_blend: decls::loco_idle_blend(locomotion)? })
    }
}

/// Resolve a demanded state with only a snapshot of the live alias indices.
/// `clips` holds the worker's new clips, appended atomically at `base` by the
/// host; existing clips continue animating without copying their payload.
pub fn prepare_equipment(
    clips: &mut crate::assets::animation_rig::Clips<'_>,
    existing: &std::collections::HashMap<String, usize>,
    base: usize,
    definition: &EquipmentDefinition,
    additive_items: &[super::animation::AnimBasicItem],
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<PreparedEquipment, String> {
    use super::animation::{LocoData, SpeedSet, Transition};
    let check = || if cancelled.is_some_and(|cancelled| cancelled()) { Err("arm equipment preparation cancelled".to_owned()) } else { Ok(()) };
    let first_new_clip = clips.animations.len();
    let result = (|| {
        let rig = clips.rig;
        let mut clip = |name: &str| -> Result<usize, String> {
            check()?;
            if let Some(&index) = existing.get(name) { return Ok(index); }
            Ok(base + clips.clip_cancellable(name, cancelled)?)
        };
        let mut data = LocoData::default();
        data.idle_blend = definition.idle_blend;
        let mut stances = std::collections::HashMap::new();
        for stance in &definition.stances {
            check()?;
            let key = equip_key(&stance.name, &definition.name);
            let mut sets: [Option<SpeedSet>; 4] = Default::default();
            for (speed, names, speeds) in &stance.speeds {
                if *speed >= sets.len() { continue; }
                for name in names { check()?; rig.movement(name)?; }
                sets[*speed] = Some(SpeedSet { clips: [clip(&names[0])?, clip(&names[1])?, clip(&names[2])?, clip(&names[3])?], speeds: *speeds });
            }
            if let Some(alias) = &stance.idle { data.idle_clips.insert(key.clone(), clip(alias)?); }
            data.stances.insert(key.clone(), sets);
            stances.insert(stance.name.clone(), key);
        }
        for (from, to, alias, blend_in, blend_out) in &definition.stance_transitions {
            data.transitions.insert((equip_key(from, &definition.name), equip_key(to, &definition.name)), Transition { clip: clip(alias)?, blend_in: *blend_in, blend_out: *blend_out });
        }
        for transition in &definition.equip_transitions {
            data.transitions.insert((equip_key(&transition.stance, &transition.from), equip_key(&transition.stance, &transition.to)),
                Transition { clip: clip(&transition.alias)?, blend_in: transition.blend_in, blend_out: transition.blend_out });
        }
        let jump_items = definition.jump_aliases.each_ref().map(|alias| alias.as_ref().and_then(|alias| additive_items.iter().position(|item| &item.alias == alias)));
        check()?;
        let mut animation_sounds=super::midnight::AnimationSoundBank::default();
        for (alias,&index) in &clips.loaded {animation_sounds.prepare(0,base+index,rig,alias)?;}
        Ok(PreparedEquipment { name: definition.name.clone(), data, stances, jump_items, animation_sounds })
    })();
    if result.is_err() {
        clips.animations.truncate(first_new_clip);
        clips.loaded.retain(|_, index| *index < first_new_clip);
    }
    result
}
