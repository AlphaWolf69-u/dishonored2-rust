//! The actual FX manager gameplay families219370/21a730. Render simulation
//! emits source requests; canonical entity/explosion owners consume them.
use crate::{
    gameplay::{
        effects::{
            gameplay::{Request, Requests},
            standard_events::Schedule,
        },
        reference_frame::ReferenceFrame,
    },
    rendering::particles::world_sources::{WorldEntityDef, WorldExplosion},
};
use std::sync::Arc;
enum Kind {
    Entity(Arc<WorldEntityDef>),
    Explosion(Arc<WorldExplosion>),
}
pub(super) struct Event {
    schedule: Schedule,
    kind: Kind,
}
impl Event {
    pub(super) fn is_entity(&self) -> bool {
        matches!(self.kind, Kind::Entity(_))
    }
    pub(super) fn entity(source: Arc<WorldEntityDef>, offset: f32, seed: &mut u32) -> Self {
        let schedule = Schedule::new(
            &source.asset.entity_defs[source.index].timings,
            offset,
            seed,
        );
        Self {
            schedule,
            kind: Kind::Entity(source),
        }
    }
    pub(super) fn explosion(source: Arc<WorldExplosion>, offset: f32, seed: &mut u32) -> Self {
        let schedule = Schedule::new(&source.asset.explosions[source.index].timings, offset, seed);
        Self {
            schedule,
            kind: Kind::Explosion(source),
        }
    }
    pub(super) fn stop(&mut self) {
        self.schedule.stop();
    }
    pub(super) fn advance(
        &mut self,
        age: f32,
        seed: &mut u32,
        frame: ReferenceFrame,
        provenance: Option<super::world_effects::EntityEffectContext>,
        queue: &mut Requests,
    ) -> Result<bool, String> {
        let kind = &self.kind;
        self.schedule.advance_with_seed(age, seed, |seed| {
            match kind {
                Kind::Entity(source) => {
                    let asset = &source.asset.entity_defs[source.index];
                    let count = source.declarations.len();
                    if count == 0 {
                        return Err("admitted FX entity descriptor has no declaration slots".into());
                    }
                    let mut index =
                        ((crate::random::lcg_step(seed) >> 10) & 0x7fff) as usize % count;
                    for _ in 1..count {
                        if source.declarations[index].is_some() {
                            break;
                        }
                        index = (index + 1) % count;
                    }
                    //219370 consumes all three offset draws even after a NULL
                    //selection; valid admission guarantees one non-null slot.
                    let samples = std::array::from_fn::<_, 3, _>(|_| crate::random::lcg_unit(seed));
                    let offset = std::array::from_fn(|axis| {
                        let max = crate::gameplay::effects::normalize_small(asset.offset[0][axis]);
                        let min = crate::gameplay::effects::normalize_small(asset.offset[1][axis]);
                        crate::float::add(
                            crate::float::multiply(1. - samples[axis], min),
                            crate::float::multiply(samples[axis], max),
                        )
                    });
                    let axis = bevy_math::Mat3::from_cols_array_2d(&frame.basis);
                    let offset = bevy_math::Vec3::from_array(offset);
                    let position = bevy_math::Vec3::from_array(frame.position)
                        + if asset.worldspace {
                            offset
                        } else {
                            axis * offset
                        };
                    let Some(definition) = source.declarations[index].clone() else {
                        return Err("admitted FX entity selection is NULL".into());
                    };
                    queue.push_back(Request::Entity {
                        definition,
                        source_lease: source.lifetime.clone(),
                        frame: ReferenceFrame {
                            position: position.to_array(),
                            basis: if asset.use_owner_orientation {
                                frame.basis
                            } else {
                                ReferenceFrame::default().basis
                            },
                        },
                    });
                }
                Kind::Explosion(source) => queue.push_back(Request::Explosion {
                    source_lease: source.lifetime.clone(),
                    request: crate::gameplay::explosions::Request {
                        declaration: source.declaration.clone(),
                        inflictor: provenance.and_then(|context| context.owner),
                        attacker: provenance.and_then(|context| context.culprit),
                        position: bevy_math::Vec3::from_array(frame.position),
                        direction: bevy_math::Vec3::from_array(frame.basis[0]),
                        cannot_incapacitate_player_and_allies: provenance
                            .is_some_and(|context| context.cannot_incapacitate_player_and_allies),
                    },
                }),
            }
            Ok(())
        })
    }
}
