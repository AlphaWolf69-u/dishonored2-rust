//! The Flash runtime used by the game's Iggy UI (`Engine\flash`).
pub mod avm2;

use avm2::Vm;

/// `flash.utils.getTimer`: milliseconds since the VM (player) was created.
pub fn host_timer(vm: &Vm) -> i32 {
    vm.started.elapsed().as_millis() as i32
}

/// `Math.random`. Iggy's generator is not yet traced, so this uses an
/// xorshift sequence; only the [0, 1) range contract is relied upon.
pub fn host_random(vm: &mut Vm) -> f64 {
    let mut x = vm.random;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    vm.random = x;
    (x >> 11) as f64 / (1u64 << 53) as f64
}
