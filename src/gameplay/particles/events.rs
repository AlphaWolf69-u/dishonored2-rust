//! Named particle-stage subscriptions and world-space event publication.
//! 1f38e0 excludes self subscriptions; 1f2040 sends current position/velocity.
use super::{ParticleState, SpawnInput};
use crate::{
    assets::fx::{particle_declaration::ParticleDeclaration, particle_stage::ParticleEvent},
    float::{add, multiply},
    gameplay::reference_frame::ReferenceFrame,
};

pub struct Subscription {
    receivers: Vec<usize>,
    threshold: i32,
    life: Option<f32>,
}
pub struct Delivery {
    pub receiver: usize,
    pub input: SpawnInput,
}
pub struct StageEvents {
    birth: Subscription,
    death: Subscription,
    life: Vec<Subscription>,
    pub pending: Vec<Delivery>,
}
impl StageEvents {
    pub fn new(declaration: &ParticleDeclaration, sender: usize) -> Self {
        let subscription = |event: &ParticleEvent| Subscription {
            receivers: declaration
                .stages
                .iter()
                .enumerate()
                .filter_map(|(i, stage)| {
                    (i != sender
                        && stage.enabled
                        && !stage.burst_event.is_empty()
                        && stage.burst_event == event.name)
                        .then_some(i)
                })
                .collect(),
            // 1efcc0 /1f2270 retain a signed threshold; no clamping is applied.
            threshold: crate::float::truncate_i32(multiply(
                event.frequency,
                f32::from_bits(0x46ff_fe00),
            )),
            life: event.life,
        };
        let events = &declaration.stages[sender].events;
        Self {
            birth: subscription(&events.birth),
            death: subscription(&events.death),
            life: events.life.iter().map(subscription).collect(),
            pending: Vec::new(),
        }
    }
    pub fn birth(
        &mut self,
        particle: &ParticleState,
        seed: &mut u32,
        pose: ReferenceFrame,
        worldspace: bool,
    ) {
        if Self::admitted(&self.birth, seed) {
            Self::publish(&self.birth, &mut self.pending, particle, pose, worldspace);
        }
    }
    pub fn integrated(
        &mut self,
        particle: &ParticleState,
        previous_age: f32,
        alive: bool,
        seed: &mut u32,
        pose: ReferenceFrame,
        worldspace: bool,
    ) {
        if !alive {
            if Self::admitted(&self.death, seed) {
                Self::publish(&self.death, &mut self.pending, particle, pose, worldspace);
            }
            return;
        }
        // 1f5620 does not sample life-event frequency. Crossing is inclusive.
        for event in &self.life {
            if event
                .life
                .is_some_and(|life| previous_age <= life && life <= particle.age)
            {
                Self::publish(event, &mut self.pending, particle, pose, worldspace);
            }
        }
    }
    fn admitted(event: &Subscription, seed: &mut u32) -> bool {
        !event.receivers.is_empty()
            && (((crate::random::lcg_step(seed) >> 10) & 0x7fff) as i32) < event.threshold
    }
    fn publish(
        event: &Subscription,
        pending: &mut Vec<Delivery>,
        particle: &ParticleState,
        pose: ReferenceFrame,
        worldspace: bool,
    ) {
        let transform = |v: [f32; 3]| {
            std::array::from_fn(|i| {
                add(
                    add(
                        multiply(v[0], pose.basis[0][i]),
                        multiply(v[1], pose.basis[1][i]),
                    ),
                    multiply(v[2], pose.basis[2][i]),
                )
            })
        };
        let input = if worldspace {
            SpawnInput {
                position: particle.position,
                directions: [particle.velocity, [0.; 3]],
            }
        } else {
            SpawnInput {
                position: std::array::from_fn(|i| {
                    add(transform(particle.position)[i], pose.position[i])
                }),
                directions: [transform(particle.velocity), [0.; 3]],
            }
        };
        pending.extend(
            event
                .receivers
                .iter()
                .map(|&receiver| Delivery { receiver, input }),
        );
    }
}
