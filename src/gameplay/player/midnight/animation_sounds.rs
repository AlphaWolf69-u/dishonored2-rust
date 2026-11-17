//! AnimSet callbacks over the actual Midnight clip clock. 6ee4f0 passes
//! truncated frames to 1c7db0: (old,new], or the wrapping union. The queue
//! retains the source declaration until the existing audio FIFO accepts it.
use super::*;

#[derive(Clone, Debug)]
pub struct AnimationSound {
    pub frame: i16,
    pub joint: Option<u16>,
    pub event: Arc<crate::assets::wwise::SoundEventDeclaration>,
}

#[derive(Default, Debug)]
pub struct AnimationSoundBank {
    clips: [std::collections::HashMap<usize, Arc<[AnimationSound]>>; TARGETS],
    effects:[std::collections::HashMap<usize,Arc<[crate::assets::animation_rig::animset_effects::Effect]>>;TARGETS],
    revision: u64,
}
impl AnimationSoundBank {
    pub(crate) fn prepare(&mut self, target: usize, clip: usize, rig: &crate::assets::animation_rig::Rig,
        alias: &str) -> Result<(), String> {
        if self.clips[target].contains_key(&clip)||self.effects[target].contains_key(&clip) { return Ok(()); }
        let mut events = Vec::new();
        for sound in rig.animation_sound_events(alias)? {
            // 875ba0 keeps signed -1 after a missing/empty joint diagnostic;
            // the entity's audio object is then selected by709110.
            let joint = rig.skeleton.joint_names.iter().position(|name| name == sound.joint.as_ref())
                .map(u16::try_from).transpose().map_err(|_| "animation sound joint exceeds source index")?;
            events.push(AnimationSound { frame: sound.frame, joint, event: sound.event.clone() });
        }
        events.sort_by_key(|event| event.frame);
        if !events.is_empty() {
            self.clips[target].insert(clip, events.into());
            self.revision=self.revision.wrapping_add(1);
        }
        let effects=rig.animation_effect_events(alias)?;
        if !effects.is_empty() {
            let mut effects=effects.to_vec();
            effects.sort_by_key(|effect|effect.frame);
            self.effects[target].insert(clip,effects.into());
            self.revision=self.revision.wrapping_add(1);
        }
        Ok(())
    }
    pub fn revision(&self)->u64 {self.revision}
    pub fn is_empty(&self)->bool {self.clips.iter().all(std::collections::HashMap::is_empty)
        &&self.effects.iter().all(std::collections::HashMap::is_empty)}
    pub fn visit(&self,mut visit:impl FnMut(usize,&AnimationSound)) {
        for (target,clips) in self.clips.iter().enumerate() {
            for sound in clips.values().flat_map(|sounds|sounds.iter()) {visit(target,sound);}
        }
    }
    pub fn visit_effects(&self,mut visit:impl FnMut(usize,&crate::assets::animation_rig::animset_effects::Effect)) {
        for (target,clips) in self.effects.iter().enumerate() {
            for effect in clips.values().flat_map(|effects|effects.iter()) {visit(target,effect);}
        }
    }
    pub(crate) fn visit_effect_clip(&self,target:usize,clip:usize,mut visit:impl FnMut(&crate::assets::animation_rig::animset_effects::Effect)) {
        for effect in self.effects[target].get(&clip).into_iter().flat_map(|effects|effects.iter()) {visit(effect);}
    }
    pub(crate) fn visit_clip(&self,target:usize,clip:usize,mut visit:impl FnMut(&AnimationSound)) {
        for sound in self.clips[target].get(&clip).into_iter().flat_map(|sounds|sounds.iter()) {visit(sound);}
    }
    pub(crate) fn absorb(&mut self,other:Self) {
        for (target,clips) in other.clips.into_iter().enumerate() {
            for (clip,sounds) in clips {
                if let std::collections::hash_map::Entry::Vacant(entry)=self.clips[target].entry(clip) {
                    entry.insert(sounds);self.revision=self.revision.wrapping_add(1);
                }
            }
        }
        for (target,clips) in other.effects.into_iter().enumerate() {
            for (clip,effects) in clips {
                if let std::collections::hash_map::Entry::Vacant(entry)=self.effects[target].entry(clip) {
                    entry.insert(effects);self.revision=self.revision.wrapping_add(1);
                }
            }
        }
    }
    pub(super) fn get(&self, target: usize, clip: usize) -> Arc<[AnimationSound]> {
        self.clips[target].get(&clip).cloned().unwrap_or_default()
    }
    pub(super) fn get_effects(&self,target:usize,clip:usize)->Arc<[crate::assets::animation_rig::animset_effects::Effect]> {
        self.effects[target].get(&clip).cloned().unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct AnimationSoundReceipt {
    pub target: usize,
    pub emitter: Emitter,
    pub sound: AnimationSound,
}
#[derive(Clone,Debug)]
pub struct AnimationEffectReceipt {
    pub target:usize,
    pub emitter:Emitter,
    pub effect:crate::assets::animation_rig::animset_effects::Effect,
}

pub(super) fn cross(instances: &[Instance], nodes: &[Node; TARGETS], output: &mut Vec<AnimationSoundReceipt>,effects:&mut Vec<AnimationEffectReceipt>) {
    // 6ff7a0 visits positive-weight clips backwards.242bb0 selects the
    // latest positive previous frame of this exact animation not ahead of
    // the current frame; a newly admitted/restarted clip has no interval.
    for (target, node) in nodes.iter().enumerate() {
        for current in node.data.iter().rev().filter(|clip| clip.weight > 0.0) {
            let mut previous: Option<&ClipInfo> = None;
            for candidate in node.previous.iter().rev().filter(|candidate|
                candidate.weight > 0.0 && candidate.anim == current.anim && candidate.frame <= current.frame) {
                if previous.is_none_or(|old| old.frame < candidate.frame) { previous = Some(candidate); }
            }
            let Some(previous) = previous else { continue };
            let old = previous.frame as i32;
            let new = current.frame as i32;
            if old == new { continue; }
            let Some((instance, clip)) = instances.iter().rev().find_map(|instance|
                instance.clips[target].iter().find(|clip| clip.anim == current.anim)
                    .map(|clip| (instance, clip))) else { continue };
            let emitter = match target {
                0 => Emitter::PlayerBody, 1 => Emitter::PlayerLeftArm, 2 => Emitter::PlayerRightArm,
                3 | 4 => match instance.equipment_emitters[target - 3] { Some(emitter) => emitter, None => continue },
                _ => unreachable!(),
            };
            for sound in clip.sounds.iter() {
                let key = i32::from(sound.frame);
                if if old <= new { old < key && key <= new } else { old < key || key <= new } {
                    output.push(AnimationSoundReceipt { target, emitter, sound: sound.clone() });
                }
            }
            for effect in clip.effects.iter() {
                let key=i32::from(effect.frame);
                if old<key&&key<=new {effects.push(AnimationEffectReceipt{target,emitter,effect:effect.clone()});}
            }
        }
    }
}

impl Midnight {
    pub fn drain_animation_effects(&mut self)->impl Iterator<Item=AnimationEffectReceipt>+'_ {
        self.animation_effect_receipts.drain(..)
    }
    pub fn drain_animation_sounds(&mut self) -> impl Iterator<Item = AnimationSoundReceipt> + '_ {
        self.animation_sound_receipts.drain(..)
    }
}
