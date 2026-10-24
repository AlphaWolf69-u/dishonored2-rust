//! Entity LOD levels (arkEntityLodManager at game +0x30f318, ticked by
//! 14064cd50): a warm-up queue, then per-frame levels from view distance and
//! last-frame render visibility. Components see level transitions through
//! slot 0x68 (14087bb70); level 6 and above is "not simulated nearby".

/// `lod_forcevisibledistance`, `lod_dist_level0..3`, `lod_dist_far`.
#[derive(Clone, Copy, Debug)]
pub struct LodDistances {
    pub force_visible: f32,
    pub levels: [f32; 4],
    pub far: f32,
}

/// The top render view (game +0x1f52b8): +0xa8 and +0xc0 positions and
/// +0x9c horizontal field of view in radians.
#[derive(Clone, Copy, Debug)]
pub struct LodView {
    pub positions: [[f32; 3]; 2],
    pub horizontal_fov: f32,
}

struct Squared {
    second_view: bool,
    force_visible: f32,
    fov_scale: f32,
    levels: [f32; 4],
    far: f32,
}

fn squared(view: &LodView, distances: &LodDistances) -> Squared {
    let [a, b] = view.positions;
    let s = (view.horizontal_fov * 0.5).tan();
    Squared {
        // 14064cff0: the second position only counts when it differs.
        second_view: a != b,
        force_visible: distances.force_visible * distances.force_visible,
        fov_scale: s * s,
        levels: distances.levels.map(|v| v * v),
        far: distances.far * distances.far,
    }
}

fn near_distance(point: [f32; 3], view: &LodView, s: &Squared) -> f32 {
    let distance = |p: [f32; 3]| {
        let d: [f32; 3] = std::array::from_fn(|i| point[i] - p[i]);
        d[1] * d[1] + d[0] * d[0] + d[2] * d[2]
    };
    let mut d = distance(view.positions[0]);
    if s.second_view {
        let e = distance(view.positions[1]);
        if e <= d {
            d = e;
        }
    }
    d
}

fn distance_level(d: f32, s: &Squared) -> i32 {
    let d = d * s.fov_scale;
    match s.levels.iter().position(|&level| d < level) {
        Some(level) => level as i32,
        None if s.far < d => 5,
        None => 4,
    }
}

/// 14064c1a0 for an entity with a render entity. `bounds` is its world AABB
/// (+0x1840/+0x1850) and `flags_1c30` its owner flags (bit 0: empty bounds,
/// 1401fc110); `visible` is render +0x1c31 bits 2|3, set by the main view
/// pass (14031ad30) in either of the last two frame parities.
pub fn render_level(
    bounds: [[f32; 3]; 2],
    flags_1c30: u8,
    visible: bool,
    hidden: bool,
    view: &LodView,
    distances: &LodDistances,
) -> i32 {
    let s = squared(view, distances);
    let [min, max] = bounds;
    let center: [f32; 3] = std::array::from_fn(|i| (max[i] + min[i]) * 0.5);
    let mut d = near_distance(center, view, &s);
    let mut extent = max[0] - min[0];
    if extent <= max[1] - min[1] {
        extent = max[1] - min[1];
    }
    d -= extent * 0.5 * extent * 0.5;
    if ((flags_1c30 & 1 == 0 && visible) || d <= s.force_visible) && !hidden {
        distance_level(d, &s)
    } else {
        6 + (s.far < d) as i32
    }
}

/// 14064c3a0 for an entity without a render entity: origin (+0x300) only.
pub fn origin_level(origin: [f32; 3], view: &LodView, distances: &LodDistances) -> i32 {
    let s = squared(view, distances);
    distance_level(near_distance(origin, view, &s), &s)
}

/// Entity +0x294 (level), +0x298 (requested) and +0x29c (cap).
#[derive(Clone, Copy, Debug)]
pub struct EntityLod {
    pub level: i32,
    pub requested: i32,
    pub cap: i32,
    /// 14064c820 queue entry: frames left before 14064cd70 lists it.
    warmup: Option<i32>,
}

impl EntityLod {
    /// 14087caa0: level 0, cap 8, queued for `lod_dist_warmup_framecount`.
    pub fn spawned(warmup: i32) -> Self {
        Self { level: 0, requested: 0, cap: 8, warmup: Some(warmup) }
    }

    /// 14064cd70: count the warm-up down. True once the entity is listed.
    pub fn listed(&mut self) -> bool {
        match &mut self.warmup {
            Some(frames) => {
                *frames -= 1;
                if *frames < 1 {
                    self.warmup = None;
                    return true;
                }
                false
            }
            None => true,
        }
    }

    /// 14087bb70: returns (old, new) when the effective level changes.
    pub fn request(&mut self, requested: i32) -> Option<(i32, i32)> {
        let old = self.level;
        self.requested = requested;
        self.level = requested.min(self.cap);
        (old != self.level).then_some((old, self.level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distances() -> LodDistances {
        LodDistances { force_visible: 10., levels: [30., 40., 50., 70.], far: 50. }
    }

    #[test]
    fn live_dust_shutter_is_level_seven() {
        // Live Dust capture: view (-18.43, -8.52, -9.09), 113.6 deg, shutter
        // _1_2 bounds, render flags 0x34/0xb2 (not visible): level 7.
        let view = LodView {
            positions: [[-18.4304, -8.522762, -9.087629]; 2],
            horizontal_fov: 1.982609,
        };
        let bounds = [[-49.6, -85.28, -21.44], [-48.74, -85.18, -17.91]];
        assert_eq!(render_level(bounds, 0x34, false, false, &view, &distances()), 7);
        assert_eq!(render_level(bounds, 0x34, true, false, &view, &distances()), 5);
        assert_eq!(render_level(bounds, 0x34, true, true, &view, &distances()), 7);
    }

    #[test]
    fn levels_follow_squared_thresholds() {
        let view = LodView { positions: [[0.; 3]; 2], horizontal_fov: std::f32::consts::FRAC_PI_2 };
        let at = |x: f32| render_level([[x, 0., 0.], [x, 0., 0.]], 1, true, false, &view, &distances());
        // Empty bounds (bit 0) are not "visible"; force-visible distance still applies.
        assert_eq!(at(5.), 0);
        assert_eq!(at(20.), 6);
        let boxed = |x: f32| {
            render_level([[x - 0.1, -0.1, 0.], [x + 0.1, 0.1, 1.]], 0, true, false, &view, &distances())
        };
        assert_eq!([boxed(29.), boxed(35.), boxed(45.), boxed(60.), boxed(71.)], [0, 1, 2, 3, 5]);
        assert_eq!(origin_level([69., 0., 0.], &view, &distances()), 3);
        let mut lod = EntityLod::spawned(3);
        assert!(!lod.listed() && !lod.listed() && lod.listed());
        assert_eq!(lod.request(7), Some((0, 7)));
        assert_eq!(lod.request(9), Some((7, 8)));
        assert_eq!(lod.request(8), None);
    }
}
