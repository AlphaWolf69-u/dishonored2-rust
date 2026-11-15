//! The renderer's occlusion query manager (14031d7e0's input 0xa4719e7a):
//! 256 slots of 0x24 bytes. Allocation 140292cc0, enable/disable
//! 140292f80, request 140292fb0, result 140292d10 (six-frame average
//! 140292d80), per-frame draw 1402926d0. Each issued slot draws the unit
//! quad of `renderprog/occlusionquery` (OcclusionQuery_0/_1.material) inside
//! a GPU occlusion query; the next frame turns the passed-sample count into
//! a visible fraction of the quad.
use crate::float::{add, divide, multiply, subtract};
use crate::rendering::render_shadow::ShadowMatrix;

pub const SLOTS: usize = 256;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum SlotState {
    #[default]
    Free,
    /// 1: drawn inside a GPU query whose count is awaited.
    Issued,
    /// 2: allocated or re-enabled, not yet drawn.
    Pending,
}

#[derive(Clone, Copy, Default)]
struct Slot {
    state: SlotState,
    /// +4
    result: f32,
    /// +8, +0xc, +0x10
    radius: f32,
    offset: f32,
    position: [f32; 3],
    /// +0x21: drawn with the debug material and colour target.
    debug: bool,
    /// +0x22: the radius is a screen fraction x 0.01, not a world size.
    screen_radius: bool,
    /// +0x2400: the last six results and their write cursor/count.
    history: [f32; 6],
    count: i32,
    cursor: i32,
}

/// One query draw: the quad parameters `OcclusionQuery/Transform1`
/// (x axis, y axis) and `OcclusionQuery/Transform2` (projected centre).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueryDraw {
    pub slot: usize,
    pub transform1: [f32; 4],
    pub transform2: [f32; 4],
    pub debug: bool,
}

/// The pixel viewport (+0x7878..+0x7884) and resolution scales
/// (+0x78c0/+0x78c4) of the view.
#[derive(Clone, Copy)]
pub struct QueryViewport {
    pub width: u32,
    pub height: u32,
    pub scale: [f32; 2],
}

#[derive(Clone)]
pub struct OcclusionQueries {
    slots: Box<[Slot; SLOTS]>,
    /// +0x24000: each slot's last arrived passed-sample count.
    counts: Box<[u64; SLOTS]>,
}
impl Default for OcclusionQueries {
    fn default() -> Self {
        Self { slots: Box::new([Slot::default(); SLOTS]), counts: Box::new([0; SLOTS]) }
    }
}

/// A system's manager slot; dropping it frees the slot (140292f80 with
/// false, as 14031d7e0 does for a system that no longer queries).
pub struct QuerySlot(usize);
impl QuerySlot {
    pub fn allocate(manager: &mut OcclusionQueries) -> Option<Self> {
        manager.allocate().map(Self)
    }
    pub fn index(&self) -> usize {
        self.0
    }
}
impl Drop for QuerySlot {
    fn drop(&mut self) {
        if let Ok(mut manager) = shared().lock() {
            manager.enable(self.0, false);
        }
    }
}

/// The renderer owns one manager; FX systems and the query pass share it.
pub fn shared() -> &'static std::sync::Mutex<OcclusionQueries> {
    static SHARED: std::sync::OnceLock<std::sync::Mutex<OcclusionQueries>> = std::sync::OnceLock::new();
    SHARED.get_or_init(Default::default)
}

impl OcclusionQueries {
    /// 140292cc0: the first free slot, marked pending.
    pub fn allocate(&mut self) -> Option<usize> {
        let slot = self.slots.iter().position(|slot| slot.state == SlotState::Free)?;
        self.slots[slot].state = SlotState::Pending;
        Some(slot)
    }

    /// 140292f80: disabling frees the slot; enabling a free one marks it
    /// pending and leaves an issued one alone.
    pub fn enable(&mut self, slot: usize, enabled: bool) {
        let Some(slot) = self.slots.get_mut(slot) else { return };
        if !enabled {
            slot.state = SlotState::Free;
        } else if slot.state == SlotState::Free {
            slot.state = SlotState::Pending;
        }
    }

    /// 140292fb0: only a positive radius updates the request.
    pub fn request(&mut self, slot: usize, position: [f32; 3], radius: f32, offset: f32, screen_radius: bool, debug: bool) {
        let Some(slot) = self.slots.get_mut(slot) else { return };
        if 0. < radius {
            slot.screen_radius = screen_radius;
            slot.position = position;
            slot.debug = debug;
            slot.radius = radius;
            slot.offset = offset;
        }
    }

    /// 140292d10: the last result, or with `averaged` the mean of the last
    /// six (140292d80); None until the slot has been drawn once.
    pub fn result(&self, slot: usize, averaged: bool) -> Option<f32> {
        let slot = self.slots.get(slot).filter(|slot| slot.state == SlotState::Issued)?;
        if !averaged {
            return Some(slot.result);
        }
        let mut sum = 0_f32;
        for k in 0..6 {
            sum = add(sum, slot.history[((slot.count + k).rem_euclid(6)) as usize]);
        }
        Some(divide(sum, 6.))
    }

    /// The slot's last arrived count.
    pub fn count(&self, slot: usize) -> u64 {
        self.counts.get(slot).copied().unwrap_or(0)
    }

    /// A GPU count for `slot` arrived.
    pub fn record_count(&mut self, slot: usize, count: u64) {
        if let Some(entry) = self.counts.get_mut(slot) {
            *entry = count;
        }
    }

    /// 1402926d0's per-slot work for one frame, using each issued slot's
    /// last count; the returned draws are issued this frame and their slots
    /// become issued.
    pub fn prepare(
        &mut self,
        view: &ShadowMatrix,
        projection: &ShadowMatrix,
        viewport: QueryViewport,
    ) -> Vec<QueryDraw> {
        let width = viewport.width as f32;
        let height = viewport.height as f32;
        let aspect = divide(width, height);
        let row = |m: &ShadowMatrix, r: usize, v: [f32; 4]| {
            add(add(add(multiply(v[1], m[r][1]), multiply(v[0], m[r][0])), multiply(v[2], m[r][2])), multiply(v[3], m[r][3]))
        };
        let mut draws = Vec::new();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.state == SlotState::Free || !(0. < slot.radius) {
                continue;
            }
            let p = [slot.position[0], slot.position[1], slot.position[2], 1.];
            let vx = row(view, 0, p);
            let vy = row(view, 1, p);
            let vz = add(row(view, 2, p), slot.offset);
            let vw = row(view, 3, p);
            let v = [vx, vy, vz, vw];
            let inverse_w = divide(1., row(projection, 3, v));
            let depth = multiply(row(projection, 2, v), inverse_w);
            let radius = if slot.screen_radius {
                multiply(slot.radius, f32::from_bits(0x3c23_d70a))
            } else {
                let r = slot.radius;
                let w = add(add(add(multiply(r, projection[3][0]), multiply(projection[3][1], 0.)), multiply(vz, projection[3][2])), projection[3][3]);
                let x = add(add(add(multiply(r, projection[0][0]), multiply(projection[0][1], 0.)), multiply(vz, projection[0][2])), projection[0][3]);
                multiply(divide(1., w), x)
            };
            if slot.state == SlotState::Issued {
                let count = self.counts[index] as f32;
                let area = multiply(
                    multiply(
                        multiply(multiply(multiply(multiply(multiply(aspect, height), radius), radius), width), f32::from_bits(0x3f78_51ec)),
                        viewport.scale[0],
                    ),
                    viewport.scale[1],
                );
                let mut fraction = subtract(multiply(divide(count, area), f32::from_bits(0x3fb3_3333)), f32::from_bits(0x3ecc_cccd));
                fraction = if 0. <= fraction { if 1. < fraction { 1. } else { fraction } } else { 0. };
                slot.result = fraction;
                slot.cursor += 1;
                let count = slot.count + 1;
                if 5 < slot.cursor {
                    slot.cursor = 0;
                }
                slot.count = if 5 < count { 0 } else { count };
                slot.history[slot.cursor as usize] = fraction;
            }
            let x = multiply(add(add(add(multiply(vy, projection[0][1]), multiply(vx, projection[0][0])), multiply(vz, projection[0][2])), multiply(vw, projection[0][3])), inverse_w);
            let y = multiply(add(add(add(multiply(vy, projection[1][1]), multiply(vx, projection[1][0])), multiply(vz, projection[1][2])), multiply(vw, projection[1][3])), inverse_w);
            draws.push(QueryDraw {
                slot: index,
                transform1: [radius, 0., 0., multiply(radius, aspect)],
                transform2: [x, y, depth, 0.],
                debug: slot.debug,
            });
            slot.state = SlotState::Issued;
        }
        draws
    }
}
