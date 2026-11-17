//! Native PlayerFX NearDeath820130, Blink81f330/81f360 and Swimming821350.
//! Playback and every sound/particle/FOV tail belong to the shared FX manager.
use super::*;
use crate::entities::{Value, scene_motion::{atom, child}};

pub(super) struct Runtime {
    paths: [Option<Arc<str>>; 7],
    owners: [Arc<str>; 7],
    active: [bool; 7],
    scope: Option<Arc<str>>,
    pub ready: bool,
    closed: bool,
    health_threshold: f32,
    breath_threshold: f32,
    underwater: bool,
    blinking: bool,
}
impl Runtime {
    pub fn new(fields: &[(String, Value)]) -> Result<Self, String> {
        let path = |family: &str, field: &str| atom(child(fields, family), field)
            .filter(|path| !path.is_empty() && *path != "NULL").map(Arc::from);
        let number = |field: &str| -> Result<f32, String> {
            //81ffc0 initializes both native thresholds to30.
            atom(child(fields, "m_nearDeath"), field).map_or(Ok(30.), |value|
                value.parse().map_err(|_| format!("invalid near-death {field}: {value}")))
        };
        Ok(Self { paths: [path("m_nearDeath", "m_healthScreenFX"),
            path("m_nearDeath", "m_breathScreenFX"), path("m_blink", "m_screenFX"),
            path("m_blink", "m_endFX"), path("m_swimming", "m_enterWaterFX"),
            path("m_swimming", "m_underWaterFX"), path("m_swimming", "m_leaveWaterFX")],
            owners: std::array::from_fn(|_| Arc::from("")), active: [false; 7],
            scope: None, ready: false, closed: false,
            health_threshold: number("m_healthThreshold")?, breath_threshold: number("m_breathThreshold")?,
            underwater: false, blinking: false })
    }
    pub fn prepare(&mut self, world: &mut World, player: crate::gameplay::entity_lifetime::EntityId) -> Result<(), String> {
        if self.closed || self.ready { return Ok(()); }
        if self.scope.is_none() {
            let scope: Arc<str> = Arc::from(format!("player-fx-manager:{player:?}"));
            self.owners = std::array::from_fn(|index| Arc::from(format!("{scope}:{index}")));
            world.resource_mut::<super::super::usable_effects::EffectRequests>().reserve(scope.clone(),
                self.paths.iter().flatten().map(|path| path.to_string()).collect());
            self.scope = Some(scope);
        }
        self.ready = super::super::usable_effects::reservation_ready(world, self.scope.as_deref().unwrap())?;
        Ok(())
    }
    fn set(&mut self, requests: &mut super::super::usable_effects::EffectRequests, index: usize, active: bool) {
        if self.active[index] == active { return; }
        if let Some(path) = &self.paths[index] {
            if active { requests.start_player(&self.owners[index], path); }
            else { requests.stop_player(&self.owners[index]); }
        }
        self.active[index] = active;
    }
    fn pulse(&mut self, requests: &mut super::super::usable_effects::EffectRequests, index: usize) {
        if let Some(path) = &self.paths[index] {
            requests.start_entity_once(self.owners[index].clone(), path,
                super::super::usable_effects::EntityPlacement {
                    target: crate::gameplay::event_graph::EntityReference::Player,
                    tag: String::new(), local: Default::default(), attached: true,
                });
        }
    }
    pub fn advance(&mut self, requests: &mut super::super::usable_effects::EffectRequests, player: &Player) {
        if !self.ready || self.closed { return; }
        let underwater = player.is_underwater();
        self.set(requests, 0, player.health() < self.health_threshold);
        self.set(requests, 1, underwater && player.breath() < self.breath_threshold);
        let blinking = player.machine.is_active(player::states::StateId::BLINKING);
        if blinking != self.blinking {
            self.set(requests, 2, blinking);
            if !blinking { self.pulse(requests, 3); }
            self.blinking = blinking;
        }
        //821280/8212a0 admit ordinary swimming and retain the exit FX tail.
        if underwater != self.underwater {
            self.set(requests, 4, underwater);
            self.set(requests, 5, underwater);
            if underwater { requests.stop_entity(self.owners[6].clone()); }
            else { self.pulse(requests, 6); }
            self.underwater = underwater;
        }
    }
    pub fn retire(&mut self, requests: &mut super::super::usable_effects::EffectRequests) {
        if self.closed { return; }
        if self.scope.is_none() { self.closed = true; return; }
        for index in 0..7 { self.set(requests, index, false); requests.stop_entity(self.owners[index].clone()); }
        if let Some(scope) = self.scope.take() { requests.release_reservation(scope); }
        self.closed = true;
    }
}

pub(in crate::app) fn update(
    actions: Option<ResMut<PlayerActions>>,
    state: Res<super::super::player::PlayerState>,
    effects: Option<ResMut<super::super::usable_effects::EffectRequests>>,
) {
    let (Some(mut actions), Some(mut effects), Some(player)) = (actions, effects, state.player.as_ref()) else { return };
    actions.fx.advance(&mut effects, player);
}
