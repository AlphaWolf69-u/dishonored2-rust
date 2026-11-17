//! RuneCharms sensor20: source3c4850/3c4af0 registration,3c4c60 view
//! projection and3c4f20 holstered heartbeat. It is not a proximity query.
use super::{Fields, Rumble, Target};
use crate::{
    assets::entities::scene_motion::{atom, child},
    gameplay::{damage::number, entity_lifetime::EntityId},
};
use bevy_math::Vec3;

#[derive(Clone, Debug)]
pub struct Definition {
    pub sound: Option<String>,
    pub period: f32,
    pub distance: f32,
    pub message_duration: f32,
    pub rumble: Rumble,
}
impl Definition {
    pub fn read(fields: &Fields) -> Result<Self, String> {
        let rumble = child(fields, "m_rumbleFeedback");
        Ok(Self {
            sound: atom(fields, "m_heartSound")
                .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
                .map(str::to_owned),
            period: number(fields, "m_heartPeriod", 4.)?,
            distance: number(fields, "m_distanceForHeartBeat", 15.)?,
            message_duration: number(fields, "m_maxDurationForMessage", 10.)?,
            rumble: Rumble {
                low: number(rumble, "m_lowIntensity", 0.3)?,
                high: number(rumble, "m_highIntensity", f32::from_bits(0x3d3c6a7f))?,
                duration: number(rumble, "m_rumbleDuration", 0.2)?,
            },
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub target: Target,
    pub kind: u32,
    pub item: i32,
    pub on_screen: bool,
}
pub struct Sensor {
    pub rows: Vec<Row>,
    pub nearest_distance: f32,
    last_ambient_beat: f64,
    message: u8,
    remaining: f32,
}
impl Default for Sensor {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            nearest_distance: f32::from_bits(0x7149f2ca),
            last_ambient_beat: 0.,
            message: 0,
            remaining: 0.,
        }
    }
}
impl Sensor {
    ///76e8b0 starts only actual Rune/Charm item types and excludes NPC loot.
    pub fn register_pickable(
        &mut self,
        entity: EntityId,
        item_type: u32,
        item: i32,
        invisible_npc_loot: bool,
    ) {
        if invisible_npc_loot {
            return;
        }
        let kind = match item_type {
            5 => 1,
            7 => 0,
            8 => 2,
            _ => return,
        };
        self.register(entity, kind, item);
    }
    ///4a4de0 uses the same registered owner for its actual kind3 component.
    pub fn register(&mut self, entity: EntityId, kind: u32, item: i32) {
        self.rows.push(Row {
            target: Target {
                entity,
                angle: 0.,
                distance: 0.,
            },
            kind,
            item,
            on_screen: false,
        });
    }
    ///3c4af0 removes one matching registered record, moving the last row.
    pub fn unregister(&mut self, entity: EntityId) {
        if let Some(index) = self.rows.iter().position(|row| row.target.entity == entity) {
            self.rows.swap_remove(index);
        }
    }
    ///The caller supplies actual bounds/view projection. A stale generation
    ///returns None and is removed in source order; errors retain the row.
    pub fn update(
        &mut self,
        eye: Vec3,
        forward: Vec3,
        mut bounds: impl FnMut(EntityId) -> Result<Option<[[f32; 3]; 2]>, String>,
        mut project: impl FnMut(Vec3) -> Option<[f32; 2]>,
    ) -> Result<(), String> {
        self.nearest_distance = f32::from_bits(0x7149f2ca);
        for index in (0..self.rows.len()).rev() {
            let Some(bounds) = bounds(self.rows[index].target.entity)? else {
                self.rows.remove(index);
                continue;
            };
            let center = (Vec3::from(bounds[0]) + Vec3::from(bounds[1])) * 0.5;
            let offset = center - eye;
            //3c4c60 uses FLT_MIN as the denominator floor, preserving the
            //source zero-length result instead of normalizing a zero vector.
            let squared = offset.length_squared();
            let inverse = 1. / squared.max(f32::MIN_POSITIVE).sqrt();
            let distance = squared * inverse;
            let direction = offset * inverse;
            let dot = forward.dot(direction);
            let row = &mut self.rows[index];
            row.target.distance = distance;
            row.target.angle = if dot <= -1. {
                std::f32::consts::PI
            } else if dot >= 1. {
                0.
            } else {
                dot.acos()
            };
            row.on_screen =
                project(center).is_some_and(|xy| xy[0].abs() <= 1. && xy[1].abs() <= 1.);
            self.nearest_distance = self.nearest_distance.min(distance);
        }
        Ok(())
    }
    ///3c4f20 returns the authored ambient pulse only when Heart is holstered.
    pub fn ambient_beat(
        &mut self,
        definition: &Definition,
        heart_equipped: bool,
        now: f64,
    ) -> bool {
        if !heart_equipped
            && self.nearest_distance <= definition.distance
            && now - self.last_ambient_beat > f64::from(definition.period)
        {
            self.last_ambient_beat = now;
            true
        } else {
            false
        }
    }
    ///3c4b30/3c46c0: changes to contextual message13; Some(true) shows it,
    ///Some(false) withdraws it. State2/3 suppress repeats until leaving range.
    pub fn message(
        &mut self,
        definition: &Definition,
        heart_equipped: bool,
        dt: f32,
    ) -> Option<bool> {
        let near = self.nearest_distance <= definition.distance;
        match self.message {
            0 if near && !heart_equipped => {
                self.message = 1;
                self.remaining = definition.message_duration;
                Some(true)
            }
            1 if !near => {
                self.message = 0;
                Some(false)
            }
            1 if heart_equipped => {
                self.message = 3;
                Some(false)
            }
            1 => {
                self.remaining -= dt;
                if self.remaining <= 0. {
                    self.message = 2;
                    Some(false)
                } else {
                    None
                }
            }
            _ if !near => {
                self.message = 0;
                None
            }
            _ => None,
        }
    }
}
