//! arkCpntBreakable7ed000/7ed920/7ee970/7ee990 state and ordered effects.
//! Real component, health and spawned chunk handles are supplied by their
//! canonical owners. Health metadata alone never constructs this class.
use super::{
    damage::{DamageResult, DamageType, Parameters},
    entity_lifetime::EntityId,
};
use crate::assets::breakable::{Definition, Fx};
use bevy_math::Vec3;
use std::{collections::VecDeque, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub struct Health {
    pub current: f32,
    pub infinite: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum Message {
    Damaged(DamageResult),
    DamageAfterDeath,
    Incapacitated,
}
/// An exact source operation. The live host executes these against the same
/// entity/render/physics/FX owners and acknowledges only after actual success.
#[derive(Clone, Debug)]
pub enum Effect {
    /// 7ed000 detaches actual bound children, optionally updating their
    /// cannot-hit-player property when the real culprit is a player.
    HideAndDetachChildren {
        owner: EntityId,
        culprit: Option<EntityId>,
    },
    StopOwnerRigid {
        owner: EntityId,
    },
    SpawnFx {
        component: EntityId,
        target: EntityId,
        step: usize,
        chunk: Option<usize>,
        fx: Fx,
        params: Parameters,
        cannot_incapacitate_player_and_allies: bool,
        hold_owner: bool,
    },
    RemoveOrGhost {
        entity: EntityId,
        delay: f32,
    },
    Remove {
        entity: EntityId,
        delay: f32,
    },
    /// 7ee0e0 copies the owner's actual pose, linear/angular velocity and
    /// decals; sets d4bit4, enables the registered actor and shows its model.
    ReleaseChunk {
        owner: EntityId,
        entity: EntityId,
        culprit: Option<EntityId>,
    },
    ChunkImpulse {
        entity: EntityId,
        point: Vec3,
        direction: Vec3,
        scale: f32,
        falloff: f32,
    },
    KeepBindInfo {
        owner: EntityId,
        entity: EntityId,
    },
    Damage {
        entity: EntityId,
        delay: f32,
        params: Parameters,
    },
    SoundOcclusion {
        owner: EntityId,
        strength: f32,
    },
    BrokenCallback {
        owner: EntityId,
        step: usize,
        will_be_removed: bool,
    },
    BrokenMessage {
        owner: EntityId,
    },
}
/// Source878230 center: the actual model's transformed bounds center, or the
/// entity origin only when it really has no model. The host provides it.
pub fn chunk_impulse(center: Vec3, point: Vec3, direction: Vec3, scale: f32, falloff: f32) -> Vec3 {
    let attenuation = if falloff > 0.0 {
        (1.0 - (point - center).length() / falloff).max(0.0)
    } else {
        1.0
    };
    direction * (scale * attenuation)
}
pub struct Runtime {
    pub definition: Arc<Definition>,
    pub current_step: usize,
    pub thrown: bool,
    pub cannot_incapacitate_player_and_allies: bool,
    pub culprit: Option<EntityId>,
    pub broken: bool,
    pub initialized: bool,
    pub started: bool,
    chunks: Vec<Vec<Option<EntityId>>>,
    pending: VecDeque<Effect>,
    stopping: bool,
}
impl Runtime {
    /// Constructor7ec630. This does not fabricate any preconstructed entity.
    pub fn new(definition: Arc<Definition>) -> Self {
        let chunks = definition
            .steps
            .iter()
            .map(|step| vec![None; step.chunks.len()])
            .collect();
        Self {
            definition,
            current_step: 0,
            thrown: false,
            cannot_incapacitate_player_and_allies: false,
            culprit: None,
            broken: false,
            initialized: false,
            started: false,
            chunks,
            pending: VecDeque::new(),
            stopping: false,
        }
    }
    pub fn chunk(&self, step: usize, chunk: usize) -> Option<EntityId> {
        self.chunks.get(step)?.get(chunk).copied().flatten()
    }
    /// Initialize7ed430 records each distinct actor returned by the shared
    /// factory. Its source entitydef must exist, and a slot cannot be reused.
    pub fn install_chunk(
        &mut self,
        step: usize,
        chunk: usize,
        entity: EntityId,
    ) -> Result<(), String> {
        if self.started || self.stopping {
            return Err("breakable chunk initialization is closed".into());
        }
        let authored = self
            .definition
            .steps
            .get(step)
            .and_then(|step| step.chunks.get(chunk))
            .ok_or("breakable chunk index absent")?;
        if authored.entity.is_none() {
            return Err("null breakable chunk cannot acquire an actor".into());
        }
        if self.chunks.iter().flatten().any(|old| *old == Some(entity)) {
            return Err("breakable chunks cannot share an actor generation".into());
        }
        let slot = &mut self.chunks[step][chunk];
        if slot.is_some() {
            return Err("breakable chunk already initialized".into());
        }
        *slot = Some(entity);
        Ok(())
    }
    pub fn finish_initialize(&mut self) -> Result<(), String> {
        for (step, chunk, _) in self.definition.chunks() {
            if self.chunk(step, chunk).is_none() {
                return Err("breakable initialization is missing an actual authored chunk".into());
            }
        }
        self.initialized = true;
        Ok(())
    }
    pub fn start(&mut self) -> Result<(), String> {
        if !self.initialized || self.stopping {
            return Err("breakable Start preceded actual chunk initialization".into());
        }
        self.current_step = 0;
        self.started = true;
        Ok(())
    }
    /// 7edfa0 marks the real thrown component. allowThrowHits is a separate
    /// entity flag and does not stand in for this runtime bit.
    pub fn set_thrown(&mut self, value: bool) {
        self.thrown = value;
    }
    pub fn should_break(&self, health: Health, damage: f32, last_only: bool) -> bool {
        self.current_step < self.definition.steps.len()
            && !health.infinite
            && (health.current - damage
                <= self.definition.steps[self.current_step].health_threshold
                || self.thrown)
            && (!last_only || self.current_step + 1 == self.definition.steps.len())
    }
    /// Query7ee050 uses the SAME real Health calculation as ordinary damage.
    /// The host supplies its checked health receiver's calculation result.
    pub fn predicts_break(
        &self,
        health: Health,
        result: DamageResult,
        accepts_type: bool,
        last_only: bool,
    ) -> bool {
        //717e40 is the type filter, independent of the calculation's direction
        //filter: a thrown object's override can still accept zero calculated damage.
        accepts_type && self.should_break(health, result.health_damage, last_only)
    }
    pub fn message(
        &mut self,
        owner: EntityId,
        message: Message,
        health: Health,
        params: Parameters,
    ) -> Result<bool, String> {
        if !self.started || self.stopping {
            return Err("breakable callback has no started component".into());
        }
        let advance = match message {
            Message::Damaged(result) => {
                result.health_damage > 0.0 && self.should_break(health, 0., false)
            }
            Message::DamageAfterDeath | Message::Incapacitated => {
                self.current_step < self.definition.steps.len()
            }
        };
        if advance {
            self.advance_step(owner, params)?;
        }
        if matches!(message, Message::Incapacitated)
            || matches!(message,Message::Damaged(result) if result.health_damage>0.)
        {
            self.thrown = false;
        }
        Ok(advance)
    }
    fn advance_step(&mut self, owner: EntityId, params: Parameters) -> Result<(), String> {
        let index = self.current_step;
        let step = &self.definition.steps[index];
        // Validate required hit data before transferring any live chunk lease.
        if step
            .chunks
            .iter()
            .enumerate()
            .any(|(i, _)| self.chunks[index][i].is_some())
            && params.hit_location.is_none()
        {
            return Err("breakable chunk impulse has no actual source hit location".into());
        }
        if step.hide_and_detach {
            self.pending.push_back(Effect::HideAndDetachChildren {
                owner,
                culprit: params.attacker,
            });
            self.pending.push_back(Effect::StopOwnerRigid { owner });
        }
        if step.owner_fx.declaration.is_some() {
            self.pending.push_back(Effect::SpawnFx {
                component: owner,
                target: owner,
                step: index,
                chunk: None,
                fx: step.owner_fx.clone(),
                params,
                cannot_incapacitate_player_and_allies: self.cannot_incapacitate_player_and_allies
                    || params.damage_type.0 & 0x100000000000 != 0,
                hold_owner: true,
            });
        } else if step.remove_entity {
            self.pending.push_back(Effect::RemoveOrGhost {
                entity: owner,
                delay: 0.,
            });
        }
        for (chunk_index, definition) in step.chunks.iter().enumerate() {
            // 7ee0e0 transfers the pointer out of the runtime array before
            // enabling it. Stop subsequently destroys only unreleased chunks.
            let target = if let Some(entity) = self.chunks[index][chunk_index].take() {
                self.pending.push_back(Effect::ReleaseChunk {
                    owner,
                    entity,
                    culprit: params.attacker,
                });
                self.pending.push_back(Effect::ChunkImpulse {
                    entity,
                    point: params.hit_location.expect("validated source location"),
                    direction: params.hit_direction,
                    scale: self.definition.impulse_on_chunks,
                    falloff: self.definition.impulse_falloff,
                });
                if definition.keep_bind_info {
                    self.pending
                        .push_back(Effect::KeepBindInfo { owner, entity });
                }
                entity
            } else {
                owner
            };
            if params.damage_type.intersects(DamageType::INSTANT_KILL) {
                self.pending.push_back(Effect::Damage {
                    entity: target,
                    delay: 0.01,
                    params,
                });
            }
            if definition.remove_after >= 0. {
                self.pending.push_back(Effect::Remove {
                    entity: target,
                    delay: definition.remove_after,
                });
            }
            if definition.fx.declaration.is_some() {
                self.pending.push_back(Effect::SpawnFx {
                    component: owner,
                    target,
                    step: index,
                    chunk: Some(chunk_index),
                    fx: definition.fx.clone(),
                    params,
                    cannot_incapacitate_player_and_allies: self
                        .cannot_incapacitate_player_and_allies
                        || params.damage_type.0 & 0x100000000000 != 0,
                    hold_owner: false,
                });
            }
        }
        self.pending.push_back(Effect::SoundOcclusion {
            owner,
            strength: step.sound_occlusion,
        });
        self.pending.push_back(Effect::BrokenCallback {
            owner,
            step: index,
            will_be_removed: true,
        });
        self.pending.push_back(Effect::BrokenMessage { owner });
        self.broken = true;
        self.culprit = params.attacker;
        self.current_step += 1;
        Ok(())
    }
    pub fn pending(&self) -> Option<&Effect> {
        self.pending.front()
    }
    pub fn acknowledge(&mut self) -> Result<(), String> {
        self.pending
            .pop_front()
            .map(|_| ())
            .ok_or("breakable acknowledged no pending source effect".into())
    }
    /// Stop7edb50 detaches listeners; destructor7ecc90 destroys only entities
    /// still retained in its preconstructed chunk arrays. Released debris is
    /// independently owned by the live world until authored removal/map exit.
    pub fn stop(&mut self) {
        self.started = false;
    }
    pub fn begin_retirement(&mut self) {
        self.started = false;
        self.stopping = true;
    }
    pub fn retained_chunks(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.chunks.iter().flatten().copied().flatten()
    }
    pub fn retire_chunk(&mut self, entity: EntityId) -> Result<(), String> {
        let slot = self
            .chunks
            .iter_mut()
            .flatten()
            .find(|slot| **slot == Some(entity))
            .ok_or("breakable retained chunk generation absent")?;
        *slot = None;
        Ok(())
    }
    pub fn stopped(&self) -> bool {
        self.stopping && self.pending.is_empty() && self.retained_chunks().next().is_none()
    }
}
