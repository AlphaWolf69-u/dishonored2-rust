//!DamageZone39cae0/39cb40/39d1d0/39d2d0/39d680. Sensor and effects use
//!their existing owners; only source timers and damaged-entity rows live here.
use crate::{
    archive::Catalog,
    assets::{
        damage::Declaration,
        entities::{Value, scene_motion::atom},
    },
    gameplay::{
        component_state::{ComponentInstance, ComponentOwner},
        entity_lifetime::EntityId,
    },
};
use std::sync::Arc;
pub struct Definition {
    pub fx: Option<String>,
    pub damage: Option<Arc<Declaration>>,
    pub player_damage: Option<Arc<Declaration>>,
    pub layers: u32,
    pub occlusion_layers: u32,
    pub radius: f32,
    pub startup_delay: f32,
    pub active_duration: f32,
    pub interval: [f32; 2],
    pub entity_interval: [f32; 2],
    pub can_damage_spawner: bool,
}
impl Definition {
    pub fn read(catalog: &Catalog, fields: &[(String, Value)]) -> Result<Self, String> {
        let reference =
            |name| atom(fields, name).filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("NULL"));
        let number = |name, default| crate::gameplay::damage::number(fields, name, default);
        let bits =
            |name, default| crate::gameplay::player::decls::layer_mask(fields, name, default);
        Ok(Self {
            fx: reference("m_fx").map(str::to_owned),
            damage: reference("m_damage")
                .map(|name| Declaration::acquire(catalog, name))
                .transpose()?,
            player_damage: reference("m_damageAgainstPlayer")
                .map(|name| Declaration::acquire(catalog, name))
                .transpose()?,
            layers: bits("m_inLayers", 0x240)?,
            occlusion_layers: bits("m_occlusionLayers", 0x23)?,
            radius: number("m_radius", 10.)?,
            startup_delay: number("m_startupDelay", 2.)?,
            active_duration: number("m_activeDuration", 5.)?,
            interval: [number("m_minInterval", 0.5)?, number("m_maxInterval", 1.)?],
            entity_interval: [
                number("m_minIntervalPerEntity", 0.)?,
                number("m_maxIntervalPerEntity", 0.)?,
            ],
            can_damage_spawner: match atom(fields, "m_canDamageSpawner") {
                None | Some("true" | "1") => true,
                Some("false" | "0") => false,
                Some(value) => {
                    return Err(format!("invalid DamageZone m_canDamageSpawner {value}"));
                }
            },
        })
    }
    pub fn effect_paths(&self) -> impl Iterator<Item = &str> {
        self.fx.as_deref().into_iter()
    }
}
///Source sensor targets retain their original order. Generic sensor actors
///have no damaged-entity row; their real owner is represented by caller's key.
#[derive(Clone, Copy)]
pub struct Target<K> {
    pub key: K,
    pub entity: Option<EntityId>,
    pub is_player: bool,
    pub eligible: bool,
}
pub struct Selection<K> {
    pub key: K,
    pub available_count: usize,
}
struct Damaged {
    entity: EntityId,
    remaining: f32,
}
pub struct Component {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub sensor: crate::gameplay::occluded_sensor::Sensor,
    pub elapsed: f32,
    pub interval: f32,
    pub player_present: bool,
    damaged: Vec<Damaged>,
    pub initialized: bool,
    pub started: bool,
    ///Host accepted-frame guard prevents repeating39d680 after a backend error.
    pub update_marker: Option<i32>,
    pub gather_marker: Option<i32>,
}
impl ComponentInstance for Component {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
impl Component {
    pub fn new(definition: Arc<Definition>) -> Self {
        Self {
            owner: Default::default(),
            definition,
            sensor: crate::gameplay::occluded_sensor::Sensor::new(),
            elapsed: 0.,
            interval: 0.,
            player_present: false,
            damaged: Vec::new(),
            initialized: false,
            started: false,
            update_marker: None,
            gather_marker: None,
        }
    }
    pub fn active(&self) -> bool {
        self.definition.startup_delay < self.elapsed
            && self.elapsed < self.definition.startup_delay + self.definition.active_duration
    }
    pub fn expired(&self) -> bool {
        self.definition.startup_delay < self.elapsed
            && self.definition.startup_delay + self.definition.active_duration <= self.elapsed
    }
    pub fn advance(&mut self, seconds: f32) {
        self.elapsed += seconds;
        self.interval -= seconds;
        if self.definition.entity_interval[1] > 0. {
            for row in &mut self.damaged {
                row.remaining -= seconds
            }
        }
    }
    ///39d2d0: prefer the last new managed entity, then last eligible sensor
    ///actor, then last managed entity whose retained cooldown has expired.
    pub fn select<K: Copy>(
        &mut self,
        targets: impl IntoIterator<Item = Target<K>>,
        mut random: impl FnMut(f32, f32) -> Result<f32, String>,
    ) -> Result<Option<Selection<K>>, String> {
        self.player_present = false;
        let (mut new, mut other, mut old, mut count) = (None, None, None, 0usize);
        for target in targets.into_iter().filter(|target| target.eligible) {
            if let Some(entity) = target.entity {
                self.player_present |= target.is_player;
                if let Some(index) = self.damaged.iter().position(|row| row.entity == entity) {
                    if self.damaged[index].remaining <= 0. {
                        old = Some((index, target.key));
                    }
                } else {
                    new = Some((entity, target.key));
                    count += 1;
                }
            } else {
                other = Some(target.key);
                count += 1;
            }
        }
        let never = f32::from_bits(0x7149f2ca);
        let entity_delay = |random: &mut dyn FnMut(f32, f32) -> Result<f32, String>| {
            if self.definition.entity_interval[1] > 0. {
                random(
                    self.definition.entity_interval[0],
                    self.definition.entity_interval[1],
                )
            } else {
                Ok(never)
            }
        };
        let selected = if let Some((entity, key)) = new {
            let remaining = entity_delay(&mut random)?;
            self.damaged.push(Damaged { entity, remaining });
            key
        } else if let Some(key) = other {
            key
        } else if let Some((index, key)) = old {
            self.damaged[index].remaining = entity_delay(&mut random)?;
            key
        } else {
            return Ok(None);
        };
        Ok(Some(Selection {
            key: selected,
            available_count: count,
        }))
    }
    pub fn finish_selection(
        &mut self,
        count: usize,
        mut random: impl FnMut(f32, f32) -> Result<f32, String>,
    ) -> Result<(), String> {
        self.interval = random(self.definition.interval[0], self.definition.interval[1])?;
        if count > 0 {
            let remaining =
                self.definition.active_duration - (self.elapsed - self.definition.startup_delay);
            let cap = if 0. <= remaining { remaining } else { 0. } / count as f32;
            if cap < self.interval {
                self.interval = cap;
            }
        }
        Ok(())
    }
}
