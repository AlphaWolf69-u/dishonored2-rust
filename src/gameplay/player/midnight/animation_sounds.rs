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
}
impl AnimationSoundBank {
    pub(crate) fn prepare(&mut self, target: usize, clip: usize, rig: &crate::assets::animation_rig::Rig,
        alias: &str) -> Result<(), String> {
        if self.clips[target].contains_key(&clip) { return Ok(()); }
        let mut events = Vec::new();
        for sound in rig.animation_sound_events(alias)? {
            // 875ba0 keeps signed -1 after a missing/empty joint diagnostic;
            // the entity's audio object is then selected by709110.
            let joint = rig.skeleton.joint_names.iter().position(|name| name == sound.joint.as_ref())
                .map(u16::try_from).transpose().map_err(|_| "animation sound joint exceeds source index")?;
            events.push(AnimationSound { frame: sound.frame, joint, event: sound.event.clone() });
        }
        events.sort_by_key(|event| event.frame);
        self.clips[target].insert(clip, events.into());
        Ok(())
    }
    pub(super) fn get(&self, target: usize, clip: usize) -> Arc<[AnimationSound]> {
        self.clips[target].get(&clip).cloned().unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct AnimationSoundReceipt {
    pub target: usize,
    pub emitter: Emitter,
    pub sound: AnimationSound,
}

pub(super) fn cross(instances: &[Instance], nodes: &[Node; TARGETS], output: &mut Vec<AnimationSoundReceipt>) {
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
        }
    }
}

impl Midnight {
    pub fn drain_animation_sounds(&mut self) -> impl Iterator<Item = AnimationSoundReceipt> + '_ {
        self.animation_sound_receipts.drain(..)
    }
}
