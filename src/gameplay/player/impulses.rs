//! arkPhysicsPlayer +0x220 inline impulse list: 1b2290 constructor,
//! 7fcb80/1b30f0 admission, 1b4760 evaluation, 1b5be0 ordered removal.
use bevy_math::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Impulse {
    direction: Vec3,
    strength: f32,
    duration: f32,
    elapsed: f32,
}
const EMPTY: Impulse = Impulse { direction: Vec3::ZERO, strength: 0., duration: 0., elapsed: 0. };

/// Native list flags0x440 select its four embedded records, rather than an
/// unbounded heap list. Immutable authored scalar pairs are projected into
/// each live record here instead of borrowing a raw declaration pointer.
/// Admission, expiry and summation therefore need no allocation or new owner.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Impulses {
    records: [Impulse; 4],
    count: usize,
}
impl Default for Impulses {
    fn default() -> Self { Self { records: [EMPTY; 4], count: 0 } }
}
impl Impulses {
    pub fn add(&mut self, direction: Vec3, strength: f32, duration: f32) -> bool {
        // 7fcb80 accepts the direction unchanged, and only positive pairs.
        if !(strength > 0. && duration > 0.) { return false; }
        // Native admission removes slot3 when the inline capacity is full,
        // then 1b4460 inserts at zero. Keep precisely that newest-first order.
        let retained = self.count.min(3);
        self.records.copy_within(0..retained, 1);
        self.records[0] = Impulse { direction, strength, duration, elapsed: 0. };
        self.count = retained + 1;
        true
    }

    pub fn update(&mut self, dt: f32) -> Vec3 {
        let mut sum = Vec3::ZERO;
        // 1b4760 evaluates from the oldest record toward slot0. Evaluation
        // precedes elapsed-time advancement, including an expiring last step.
        for index in (0..self.count).rev() {
            let impulse = &mut self.records[index];
            let fraction = impulse.elapsed / impulse.duration;
            let strength = (1. - fraction * fraction * fraction) * impulse.strength;
            sum += impulse.direction * strength;
            impulse.elapsed += dt;
            if impulse.elapsed >= impulse.duration {
                self.records.copy_within(index + 1..self.count, index);
                self.count -= 1;
                self.records[self.count] = EMPTY;
            }
        }
        sum
    }
}
