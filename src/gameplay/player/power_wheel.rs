//! The wheel state delegates to the real game-logic/movie owner (692010).
use super::{Player, states::StateId};
impl Player {
    pub fn set_power_wheel_status(&mut self, ready: bool, active: bool) {
        self.machine.wheel_status(ready, active);
        self.swimming_machine.wheel_status(ready, active);
    }
    pub fn take_power_wheel_request(&mut self) -> bool {
        let normal = self.machine.take_wheel_request();
        let swimming = self.swimming_machine.take_wheel_request();
        normal || swimming
    }
    pub fn power_wheel_state_active(&self) -> bool {
        self.machine.is_active(StateId::POWER_WHEEL)
            || self.swimming_machine.is_active(StateId::POWER_WHEEL)
    }
}
