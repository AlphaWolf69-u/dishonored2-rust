//! WhaleOilTank741ed0/7425e0. Actual Health and actor owners supply inputs.
use crate::gameplay::damage::{DamageType, Parameters};

#[derive(Default)]
pub(crate) struct Runtime {
    pub energy: i32,
    pub thrown: bool,
    ///BC is set by132; it prevents a second delayed explosion.
    pub immediate: bool,
    pub remaining: f32,
    pub damage: Option<Parameters>,
    pub started: bool,
}
impl Runtime {
    pub fn start(&mut self, starting: i32, maximum: i32) {
        self.energy = starting.clamp(0, maximum.max(0));
        self.started = true;
    }
    ///741ed0 copies the real incoming parameters before checking immunity.
    pub fn damaged(
        &mut self,
        params: Parameters,
        health: f32,
        window_source: bool,
        delayed_types: DamageType,
        delay: f32,
    ) -> bool {
        if self.energy <= 0 {
            self.thrown = false;
            return false;
        }
        self.damage = Some(params);
        //Source's WINDOW return precedes resetting BD.
        if window_source {
            return false;
        }
        let immolate = params.damage_type.0 & (1 << 22) != 0;
        let arm = health - params.amount <= 0. || self.thrown || immolate;
        if arm {
            self.remaining =
                if !self.immediate && (params.damage_type.intersects(delayed_types) || immolate) {
                    delay
                } else {
                    0.
                };
        }
        self.thrown = false;
        arm
    }
    ///132 only changes BC/B8; it does not fabricate new damage parameters.
    pub fn immediate_explosion(&mut self) {
        if self.energy > 0 {
            self.immediate = true;
            self.remaining = 0.;
        }
    }
    ///7425e0 updates even a zero timer; exactly once per accepted entity frame.
    pub fn advance(&mut self, delta: f32) -> bool {
        self.remaining -= delta;
        self.remaining < 0.
    }
}
