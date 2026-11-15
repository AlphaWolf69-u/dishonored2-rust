//! hkfxEffects::PhysicsCollide's moving-body part ("Dynamic CD", on by
//! default: 140fbea70 sets +0x74): the per-body lists of nearby dynamic
//! world bodies (begin pass 140fbf850 "Get new dynamic hits", end pass
//! 140fc01d0 "Remove dynamic hits"), the CD pass's dynamic section
//! (140fc1370, 1350-3315) with its cache of the nearest dynamic surface in
//! the hit body's own space, and the DynamicContact channels it writes.
//!
//! bevy_backend: the rewrite has no hknp world for its moving bodies. The
//! machinery world publishes each dynamic body with its collision shape as
//! a [`MovingBody`]: the hknpBody fields and hknpMotion record the pass
//! reads. The closest-point query (cada50 with hkfxClosestHitCollector) runs
//! the rewrite's convex closest-point port over the body's query shapes.
use super::{
    physics_collide::{length, rcp_estimate, rsqrt_estimate, CollideTuning, StaticCache},
    solve_contacts::BodyState,
    solve_convex_contacts::ConvexShape,
    spatial_partition::SpatialPartition,
    world_collide::{contact_material, Contact, SurfaceMaterial, WorldCollide, NO_MATERIAL},
};
use crate::assets::physworld::query_geometry::QueryShapes;
use crate::float::{add, multiply, subtract};
use crate::gameplay::{collision_filter::QueryFilter, shape_point_query::ShapePointQuery, shape_transform::ShapeMatrix};
use std::sync::Arc;

/// The invalid body id (low 24 bits set).
pub const NO_BODY: u32 = 0x00ff_ffff;
const FAR: f32 = f32::from_bits(0x7f7f_ffee);
const HUGE: f32 = f32::from_bits(0x5f7f_fff0);
const EPSILON: f32 = f32::from_bits(0x3400_0000);
/// 141f16020: a running timer re-evaluates its body after this long.
const REEVALUATE: f32 = f32::from_bits(0x3eaa_7efa);
/// 141ab3b34: an idle neighbour timer.
const IDLE: f32 = -1.;
/// The neighbour list link's "added this step" bit.
const NEW: u32 = 0x4000_0000;
const LINK: u32 = 0xbfff_ffff;

fn maxps(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

fn minps(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// (y + x) + z of the squares, the shuffle sums of 140fc1370.
fn squared(d: [f32; 3]) -> f32 {
    add(add(multiply(d[1], d[1]), multiply(d[0], d[0])), multiply(d[2], d[2]))
}

/// (c.y d.y + d.x c.x) + c.z d.z: a basis column against a vector.
fn column_dot(c: [f32; 4], d: [f32; 3]) -> f32 {
    add(add(multiply(c[1], d[1]), multiply(d[0], c[0])), multiply(c[2], d[2]))
}

/// The hknpMotion fields the pass reads (+0x00 centre of mass, +0x10
/// orientation, +0x40 linear velocity, +0x50 angular velocity in motion
/// space).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MotionRecord {
    pub centre: [f32; 4],
    pub orientation: [f32; 4],
    pub linear: [f32; 4],
    pub angular: [f32; 4],
}

impl MotionRecord {
    /// The angular velocity in world axes (1412b7990's quaternion rotation).
    pub fn world_angular(&self) -> [f32; 4] {
        let q = self.orientation;
        let w = self.angular;
        let dot = add(add(multiply(w[1], q[1]), multiply(w[0], q[0])), multiply(w[2], q[2]));
        let half = subtract(multiply(q[3], q[3]), 0.5);
        let cross = [
            subtract(multiply(w[2], q[1]), multiply(q[2], w[1])),
            subtract(multiply(w[0], q[2]), multiply(q[0], w[2])),
            subtract(multiply(w[1], q[0]), multiply(q[1], w[0])),
            subtract(multiply(w[3], q[3]), multiply(q[3], w[3])),
        ];
        std::array::from_fn(|i| {
            let r = add(add(multiply(dot, q[i]), multiply(half, w[i])), multiply(cross[i], q[3]));
            add(r, r)
        })
    }

    /// The motion's velocity at `p`: (w x (p - c)) + v.
    pub fn point_velocity(&self, p: [f32; 4]) -> [f32; 4] {
        let w = self.world_angular();
        let d: [f32; 4] = std::array::from_fn(|i| subtract(p[i], self.centre[i]));
        [
            add(subtract(multiply(d[2], w[1]), multiply(w[2], d[1])), self.linear[0]),
            add(subtract(multiply(d[0], w[2]), multiply(w[0], d[2])), self.linear[1]),
            add(subtract(multiply(d[1], w[0]), multiply(w[1], d[0])), self.linear[2]),
            add(subtract(multiply(d[3], w[3]), multiply(w[3], d[3])), self.linear[3]),
        ]
    }
}

/// One dynamic world body as PhysicsCollide reads it: the hknpBody id
/// (+0x70), IS_ACTIVE (+0x44 bit 3), transform (+0x00), packed AABB
/// (c85490), material (+0x6a) and filter (+0x6c), its motion and shape.
pub struct MovingBody {
    pub id: u32,
    pub active: bool,
    pub transform: ShapeMatrix,
    pub motion: MotionRecord,
    pub bounds: [[f32; 4]; 2],
    pub shapes: Arc<QueryShapes>,
    /// Index into [`MovingWorld::materials`], 0xffff without one.
    pub material: u16,
    pub filter: u32,
}

/// The dynamic bodies, indexed by the low 24 bits of their ids.
#[derive(Default)]
pub struct MovingWorld {
    pub bodies: Vec<Option<MovingBody>>,
    pub materials: Vec<SurfaceMaterial>,
}

impl MovingWorld {
    /// A live body (index in range, added to the world, +0x70 equal to the
    /// id and a broadphase id).
    pub fn body(&self, id: u32) -> Option<&MovingBody> {
        if id & NO_BODY == NO_BODY {
            return None;
        }
        self.bodies.get((id & NO_BODY) as usize)?.as_ref().filter(|body| body.id == id)
    }
}

/// One neighbour list entry (+0xa0, 12 bytes): the body, its conservative
/// distance and the re-evaluation timer.
#[derive(Clone, Copy, Debug)]
struct Neighbour {
    id: u32,
    distance: f32,
    timer: f32,
}

/// The neighbour lists (140fcef60 at PhysicsCollide +0x128 +0x80..+0xb8):
/// per body a list head, a shared link array whose free entries chain to
/// the next index, and the entries; +0xb0 entries per body (5).
pub struct Neighbours {
    heads: Vec<i32>,
    next: Vec<u32>,
    entries: Vec<Neighbour>,
    free: i32,
    count: i32,
    /// +0xbc: "Dynamic neighbor structure too small" was reported.
    reported: bool,
}

impl Neighbours {
    /// 140fcf3b0 with the default five entries per body.
    pub fn new(capacity: usize) -> Self {
        let entries = capacity * 5;
        Self {
            heads: vec![LINK as i32; capacity],
            next: (1..=entries as u32).collect(),
            entries: vec![Neighbour { id: NO_BODY, distance: 0., timer: 0. }; entries],
            free: 0,
            count: 0,
            reported: false,
        }
    }

    fn walk(&self, body: usize) -> impl Iterator<Item = usize> + '_ {
        let mut n = self.heads[body];
        std::iter::from_fn(move || {
            (n >= 0).then(|| {
                let current = n as usize;
                n = (self.next[current] & LINK) as i32;
                current
            })
        })
    }

    /// 140fcf5f0: a released body's entries return to the free list.
    pub fn release(&mut self, bodies: &[i32]) {
        for &body in bodies {
            let mut n = self.heads[body as usize];
            while n >= 0 {
                let following = self.next[n as usize] as i32;
                self.next[n as usize] = self.free as u32;
                self.count -= 1;
                self.free = n;
                n = following;
            }
            self.heads[body as usize] = LINK as i32;
        }
    }

    /// 140fc01d0 "Remove dynamic hits": dead entries leave their lists,
    /// the others lose their "new" bit.
    pub fn remove_dead(&mut self, bodies: &[i32]) {
        for &body in bodies {
            let mut previous = -0x4000_0001i32;
            let mut n = self.heads[body as usize];
            while n >= 0 {
                let i = n as usize;
                self.next[i] &= LINK;
                if self.entries[i].id & NO_BODY == NO_BODY {
                    // 140fcf580.
                    let following = self.next[i];
                    if previous < 0 {
                        self.heads[body as usize] = following as i32;
                    } else {
                        self.next[previous as usize] = following;
                    }
                    self.next[i] = self.free as u32;
                    self.count -= 1;
                    self.free = n;
                    n = following as i32;
                } else {
                    previous = n;
                    n = self.next[i] as i32;
                }
            }
        }
    }
}

/// The nearest-mesh-point cache of a contact record (+0x10..+0x1f): its
/// distance bound, whether it is valid, the face byte, and the body and
/// shape key it belongs to. Only compressed-mesh leaves fill it (140fe2ab0
/// type 5); the rewrite's moving bodies are convex, so it stays invalid.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Track {
    distance: f32,
    valid: u8,
    face: u8,
    body: u32,
    key: u32,
}

const NO_TRACK: Track = Track { distance: 0., valid: 0, face: 0, body: NO_BODY, key: u32::MAX };

impl Track {
    /// 1412b7990: the bound shrinks by the relative speed times dt.
    fn advance(&mut self, p: [f32; 4], v: [f32; 4], dt: f32, motion: &MotionRecord) {
        if self.valid == 0 {
            return;
        }
        let body = motion.point_velocity(p);
        let relative: [f32; 3] = std::array::from_fn(|i| subtract(v[i], body[i]));
        let squared = squared(relative);
        let speed = if squared <= 0. { 0. } else { multiply(rsqrt_estimate(squared), squared) };
        let distance = subtract(self.distance, multiply(speed, dt));
        if distance > 0. {
            self.distance = distance;
        } else {
            self.valid = 0;
            self.distance = 0.;
        }
    }
}

/// A body's dynamic cache (+0x30 entry, +0x40 sphere, +0x50 plane record):
/// the cached body, its material, flags, the validity sphere and plane in
/// the body's space, and the track.
#[derive(Clone, Copy, Debug)]
struct DynamicCache {
    body: u32,
    key: u16,
    flags: u32,
    sphere: [f32; 4],
    plane: [f32; 4],
    track: Track,
}

const NO_CACHE: DynamicCache = DynamicCache { body: NO_BODY, key: 0xffff, flags: 0, sphere: [0.; 4], plane: [0.; 4], track: NO_TRACK };

/// hkfxClosestHitCollector (reset 140fe29d0, addHit 140fe31e0 with its
/// +0xd0 limit 0): the nearest hit, the second nearest distance, and the
/// early-out distance (the second nearest, at most the query distance).
#[derive(Clone, Copy)]
struct Hit {
    position: [f32; 4],
    normal: [f32; 4],
    distance: f32,
    body: u32,
    material: u16,
    connectivity: bool,
}

struct Collector {
    best: Option<Hit>,
    best_distance: f32,
    second: f32,
    early: f32,
}

impl Collector {
    fn new() -> Self {
        Self { best: None, best_distance: FAR, second: FAR, early: HUGE }
    }

    fn add(&mut self, hit: Hit) {
        if hit.distance < self.best_distance {
            self.second = self.best_distance;
            self.best = Some(hit);
            self.best_distance = hit.distance;
        } else if hit.distance < self.second {
            self.second = hit.distance;
        }
        self.early = minps(self.early, self.second);
    }
}

/// The per-stage moving-body state PhysicsCollide owns.
pub struct DynamicCollide {
    neighbours: Neighbours,
    cache: Vec<DynamicCache>,
    query: ShapePointQuery,
    found: Vec<i32>,
}

/// The inputs of one dynamic CD pass.
pub struct DynamicStep<'a> {
    pub dt: f32,
    pub tuning: CollideTuning,
    /// The pipeline's +0x30 and +0x34 radii.
    pub radii: [f32; 2],
    /// The solver's +0x28 friction, +0x2c restitution, +0x38 friction
    /// threshold.
    pub own: [f32; 2],
    pub friction_threshold: f32,
    /// The pipeline's plane lock (+0x90).
    pub plane: [f32; 4],
    /// The collide mode (+0x28).
    pub mode: i32,
    pub world: &'a MovingWorld,
    /// The stage layer's filter word.
    pub filter: u32,
    /// SolveConvexContacts' shape records and per-body index when it has
    /// at least two (140fc1370 bVar85).
    pub convex: Option<(&'a [ConvexShape], &'a [i32])>,
}

impl DynamicCollide {
    pub fn new(capacity: usize, enabled_shape_types: u32) -> Result<Self, String> {
        Ok(Self {
            neighbours: Neighbours::new(capacity),
            cache: vec![NO_CACHE; capacity],
            query: ShapePointQuery::new(enabled_shape_types)?,
            found: Vec::new(),
        })
    }

    /// Slot 8 (140fbf420): new bodies have no dynamic cache.
    pub fn allocate(&mut self, bodies: &[i32]) {
        for &body in bodies {
            self.cache[body as usize] = NO_CACHE;
        }
    }

    /// Slot 9 (140fbf580).
    pub fn release(&mut self, bodies: &[i32]) {
        self.neighbours.release(bodies);
    }

    /// The end pass (140fc01d0).
    pub fn end(&mut self, bodies: &[i32]) {
        self.neighbours.remove_dead(bodies);
    }

    /// The begin pass (140fbf850 "Get new dynamic hits"): every active
    /// dynamic body the stage's filter accepts near the system adds itself
    /// to the lists of the bodies whose swept box (grown by twice the
    /// radius) overlaps its AABB. approximate_: the world bodies come in
    /// index order rather than the broadphase's AABB query order.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &mut self,
        step: &DynamicStep<'_>,
        partition: &SpatialPartition,
        allocated: &dyn Fn(i32) -> bool,
        linear: &[[[f32; 4]; 2]],
        statics: &mut [StaticCache],
    ) {
        let Some((low, high)) = partition.bounds() else { return };
        let filter = QueryFilter::World(step.filter);
        let reach = multiply(step.radii[1], 2.);
        let mut too_small = false;
        for body in step.world.bodies.iter().flatten() {
            let b = body.bounds;
            if !(0..3).all(|i| b[0][i] <= high[i] && low[i] <= b[1][i]) || !filter.accepts(body.filter) || !body.active {
                continue;
            }
            let v = body.motion.linear;
            let speed = add(add(multiply(v[1], v[1]), multiply(v[0], v[0])), multiply(v[2], v[2]));
            let limit = step.tuning.maximum_body_speed;
            partition.query([b[0][0], b[0][1], b[0][2]], [b[1][0], b[1][1], b[1][2]], &mut self.found);
            if !(speed < multiply(limit, limit)) {
                continue;
            }
            for &particle in &self.found {
                let i = particle as usize;
                let [p, v] = linear[i];
                let ahead: [f32; 4] = std::array::from_fn(|k| add(multiply(v[k], step.dt), p[k]));
                let overlaps = (0..3).all(|k| {
                    let (lo, hi) = (minps(p[k], ahead[k]), maxps(p[k], ahead[k]));
                    subtract(lo, reach) <= b[1][k] && b[0][k] <= add(hi, reach)
                });
                if !overlaps || !allocated(particle) {
                    continue;
                }
                let n = &mut self.neighbours;
                let present = n.walk(i).find(|&e| n.entries[e].id == body.id);
                if too_small || n.entries.len() as i32 <= n.free {
                    too_small = true;
                }
                if present.is_none() && n.free < n.entries.len() as i32 {
                    // A static cache on this body is dropped; static
                    // caches only hold level bodies in the rewrite.
                    let _ = &statics;
                    let new = n.free as usize;
                    n.free = (n.next[new] & LINK) as i32;
                    n.next[new] = n.heads[i] as u32 | NEW;
                    n.heads[i] = new as i32;
                    n.count += 1;
                    n.entries[new] = Neighbour { id: body.id, distance: 0., timer: IDLE };
                }
                if step.mode == 4 {
                    if let Some(e) = present {
                        n.entries[e].distance = 0.;
                    }
                }
            }
        }
        if too_small && !self.neighbours.reported {
            self.neighbours.reported = true;
            eprintln!("hkfxPhysicsCollideEffect: Dynamic neighbor structure too small");
        }
    }

    /// The CCD pass's dynamic part (140fc1370 stage 1, +0x72 mode 1,
    /// 9569-9680), before its static part: a body with neighbours that
    /// integration moved beyond its dynamic sphere (in world axes through
    /// the cached body's transform) is put back on it, and its cache is
    /// marked for re-evaluation. `radius` is the pipeline's +0x30 radius.
    pub fn ccd(
        &mut self,
        radius: f32,
        bodies: &[i32],
        solver_state: &dyn Fn(i32) -> i32,
        world: &MovingWorld,
        linear: &mut [[[f32; 4]; 2]],
    ) {
        use crate::gameplay::constraint_solver::inverse_length;
        for &body in bodies {
            let i = body as usize;
            if solver_state(body) == 0x80 || self.neighbours.heads[i] < 0 {
                continue;
            }
            let e = &mut self.cache[i];
            let r = e.sphere[3];
            if !(0. <= r) {
                continue;
            }
            let body_radius = multiply(radius, linear[i][1][3]);
            let [cx, cy, cz, _] = e.sphere;
            let mut centre = [cx, cy, cz];
            if e.body != NO_BODY {
                if let Some(b) = world.bodies.get((e.body & NO_BODY) as usize).and_then(Option::as_ref) {
                    let x = &b.transform;
                    centre = std::array::from_fn(|k| {
                        add(add(add(multiply(cy, x[1][k]), multiply(cx, x[0][k])), multiply(cz, x[2][k])), x[3][k])
                    });
                }
            }
            let [n0, n1, n2, w] = e.plane;
            let distance = add(add(w, multiply(n1, cy)), add(multiply(n2, cz), multiply(n0, cx)));
            let mut allowed = r;
            if 0. < r {
                allowed = if e.body == NO_BODY || r < distance {
                    maxps(subtract(r, multiply(body_radius, 0.5)), 0.)
                } else {
                    maxps(multiply(distance, f32::from_bits(0x3f33_3333)), maxps(EPSILON, r))
                };
            }
            let p = linear[i][0];
            let offset: [f32; 3] = std::array::from_fn(|k| subtract(p[k], centre[k]));
            let squared = squared(offset);
            if multiply(allowed, allowed) < squared {
                let inverse = inverse_length(squared);
                let offset = if allowed < multiply(inverse, squared) {
                    offset.map(|lane| multiply(multiply(allowed, inverse), lane))
                } else {
                    offset
                };
                linear[i][0] = [add(offset[0], centre[0]), add(offset[1], centre[1]), add(offset[2], centre[2]), p[3]];
                e.flags |= 1;
            }
        }
    }

    /// One query of a body's shape (cada50 into a fresh closest-hit
    /// collector) at the query distance `q`.
    fn closest(query: &mut ShapePointQuery, body: &MovingBody, p: [f32; 4], q: f32) -> Result<Collector, String> {
        let mut collector = Collector::new();
        collector.early = q;
        let mut threshold = q;
        query.closest_leaves(&body.shapes, body.transform, [p[0], p[1], p[2]], &mut threshold, &mut |leaf| {
            collector.add(Hit {
                position: leaf.position,
                normal: leaf.normal,
                distance: leaf.distance,
                body: body.id,
                material: body.material,
                connectivity: leaf.connectivity,
            });
            collector.early
        })?;
        Ok(collector)
    }

    /// The CD pass's dynamic section (140fc1370, 1350-3315) for `bodies`.
    #[allow(clippy::too_many_arguments)]
    pub fn collide(
        &mut self,
        step: &DynamicStep<'_>,
        partition: &SpatialPartition,
        bodies: &[i32],
        linear: &[[[f32; 4]; 2]],
        states: &mut [BodyState],
        statics: &mut [StaticCache],
        collide: &mut WorldCollide,
    ) -> Result<(), String> {
        let DynamicStep { dt, tuning, radii, own, friction_threshold, plane, mode, world, .. } = *step;
        let budget = ((bodies.len() as f32 * tuning.reevaluation_factor) as i32).max(1);
        let mut reevaluated = 0;
        let wake = {
            let x = multiply(multiply(friction_threshold, dt), own[0]);
            let x = maxps(0., if x < HUGE { x } else { HUGE });
            multiply(x, x)
        };
        for &particle in bodies {
            let i = particle as usize;
            // The moved-body set (+0xd8) only marks teleported bodies; the
            // rewrite's machinery never teleports, so its stamp never moves.
            let moved = false;
            let mut e = self.cache[i];
            let shift = |cache: &mut DynamicCache| cache.flags = ((cache.flags >> 1) & 1) | (cache.flags & !3);
            // The early out: asleep, with no neighbour but a sleeping cached
            // body.
            let usable = e.body & NO_BODY == NO_BODY || world.body(e.body).is_some();
            if usable && e.flags & 1 == 0 {
                let head = self.neighbours.heads[i];
                let mut sleeping = false;
                if head >= 0
                    && e.body & NO_BODY != NO_BODY
                    && (self.neighbours.next[head as usize] as i32) < 0
                    && self.neighbours.entries[head as usize].id == e.body
                {
                    sleeping = world.body(e.body).is_some_and(|b| !b.active);
                }
                if !(states[i].state < 0x80 || (head >= 0 && !sleeping)) {
                    shift(&mut self.cache[i]);
                    continue;
                }
            }
            let [p, v] = linear[i];
            let scale = v[3];
            let mut r30 = multiply(scale, radii[0]);
            let rq = multiply(scale, radii[1]);
            if let Some((shapes, index)) = step.convex {
                let shape = &shapes[if index.is_empty() { 0 } else { index[i] as usize }];
                let o = shape.outer;
                let k = if o == 0. {
                    0.
                } else {
                    let r = rcp_estimate(o);
                    multiply(multiply(subtract(2., multiply(r, o)), r), radii[1])
                };
                r30 = multiply(multiply(k, scale), shape.inner);
            }
            let old_key = e.key;
            let ahead: [f32; 3] = std::array::from_fn(|k| add(multiply(dt, v[k]), p[k]));
            let had = e.body != NO_BODY;
            let s = partition.sphere(i);
            let low: [f32; 3] = std::array::from_fn(|k| subtract(s[k], s[3]));
            let high: [f32; 3] = std::array::from_fn(|k| add(s[3], s[k]));
            let maximum = if p[3] < tuning.young_age { tuning.young_maximum } else { tuning.maximum_distance };
            let use_cache = !moved && usable;
            let mut main = Collector::new();
            main.early = FAR;
            let mut cached_distance = FAR;
            let mut cache_valid = false;
            let mut overlap = false;
            let mut local = [0f32; 3];
            let mut cached_velocity = [0f32; 4];
            let cached_id = e.body;
            let mut cached_motion = None::<MotionRecord>;
            // The cached body: still a neighbour, valid and overlapping?
            if e.body & NO_BODY != NO_BODY {
                let valid = world.body(e.body);
                let found = self.neighbours.walk(i).find(|&n| self.neighbours.entries[n].id == e.body);
                let mut handled = false;
                if let (Some(_), Some(b)) = (found, valid) {
                    overlap = (0..3).all(|k| low[k] <= b.bounds[1][k] && b.bounds[0][k] <= high[k]);
                    if overlap {
                        handled = true;
                        let t = b.transform[3];
                        let d: [f32; 3] = std::array::from_fn(|k| subtract(p[k], t[k]));
                        local = [column_dot(b.transform[0], d), column_dot(b.transform[1], d), column_dot(b.transform[2], d)];
                        cached_motion = Some(b.motion);
                        let radius = e.sphere[3];
                        let mut contained = false;
                        if !(0. <= radius) {
                            let n = e.plane;
                            let dd: [f32; 3] = std::array::from_fn(|k| subtract(e.sphere[k], local[k]));
                            let (a, bb, c) = (multiply(n[0], e.sphere[0]), multiply(n[1], e.sphere[1]), multiply(n[2], e.sphere[2]));
                            let centre = add(add(n[3], bb), add(c, a));
                            let side = add(add(add(n[3], multiply(local[1], n[1])), multiply(local[2], n[2])), multiply(local[0], n[0]));
                            let behind = 0. <= side || multiply(centre, centre) <= squared(dd);
                            contained = !behind;
                            cache_valid = !behind;
                        }
                        let mut look_ahead = false;
                        if mode != 2 && e.body != NO_BODY {
                            let sq = add(add(multiply(v[1], v[1]), multiply(v[0], v[0])), multiply(v[2], v[2]));
                            let speed = if sq <= 0. { 0. } else { multiply(rsqrt_estimate(sq), sq) };
                            look_ahead = add(add(multiply(multiply(speed, dt), 0.5), r30), rq) < radius;
                        }
                        let mut seed = contained;
                        if !contained {
                            if e.flags & 1 != 0 || radius <= 0. {
                                cache_valid = false;
                            } else {
                                let m = maxps(EPSILON, subtract(radius, r30));
                                let m = multiply(m, m);
                                let c: [f32; 3] = std::array::from_fn(|k| {
                                    let x = &b.transform;
                                    add(add(add(multiply(e.sphere[1], x[1][k]), multiply(e.sphere[0], x[0][k])), multiply(e.sphere[2], x[2][k])), t[k])
                                });
                                let near = squared(std::array::from_fn(|k| subtract(c[k], p[k]))) < m;
                                let near_ahead = squared(std::array::from_fn(|k| subtract(c[k], ahead[k]))) < m;
                                if near && (!look_ahead || near_ahead) {
                                    seed = true;
                                    cache_valid = true;
                                }
                            }
                        }
                        if seed {
                            // The cached plane as a hit, without a normal.
                            let n = e.plane;
                            let distance = add(add(add(n[3], multiply(local[1], n[1])), multiply(local[2], n[2])), multiply(local[0], n[0]));
                            cached_distance = distance;
                            main.add(Hit {
                                position: [0.; 4],
                                normal: [0.; 4],
                                distance,
                                body: e.body,
                                material: 0xffff,
                                connectivity: false,
                            });
                        }
                    }
                }
                if !handled {
                    e.body = NO_BODY;
                    e.key = 0xffff;
                    e.flags = 0;
                    e.plane = [0.; 4];
                    e.track = NO_TRACK;
                    e.sphere = [0.; 4];
                    if states[i].state > 3 {
                        states[i].state = 3;
                    }
                    states[i].timer = 0.;
                }
            }
            // The track's body.
            if e.track.body & NO_BODY != NO_BODY {
                match world.body(e.track.body) {
                    None => e.track = NO_TRACK,
                    Some(b) => e.track.advance(p, v, dt, &b.motion),
                }
            }
            let mut inactive_limit = FAR;
            let new_track = NO_TRACK;
            let mut capped = 0.;
            let head = self.neighbours.heads[i];
            if head >= 0 {
                let displacement = length(squared(std::array::from_fn(|k| subtract(ahead[k], p[k]))));
                let mut q = add(multiply(displacement, tuning.displacement_factor), multiply(rq, tuning.radius_factor));
                if use_cache && e.body != NO_BODY {
                    let d: [f32; 3] = std::array::from_fn(|k| subtract(local[k], e.sphere[k]));
                    let reach = add(length(squared(d)), e.sphere[3].abs());
                    q = minps(q, maxps(multiply(rq, 0.125), reach));
                }
                let q = add(q, rq);
                let q = if q < maximum { q } else { maximum };
                let q = maxps(tuning.minimum_distance, q);
                main.early = q;
                capped = subtract(q, subtract(rq, r30));
                let mut n = head;
                while n >= 0 {
                    let index = n as usize;
                    let entry = self.neighbours.entries[index];
                    let mut keep = None::<(&MovingBody, bool)>;
                    if entry.id == cached_id {
                        if e.body & NO_BODY != NO_BODY {
                            let b = world.bodies[(entry.id & NO_BODY) as usize].as_ref().ok_or("a cached moving body was removed")?;
                            if !cache_valid && reevaluated < budget {
                                reevaluated += 1;
                                let found = Self::closest(&mut self.query, b, p, q)?;
                                cached_distance = q;
                                if let Some(hit) = found.best {
                                    main.add(hit);
                                    if found.second < main.second {
                                        main.second = found.second;
                                        main.early = minps(main.early, found.second);
                                    }
                                    cached_distance = found.best_distance;
                                }
                            }
                            let slot = &mut self.neighbours.entries[index];
                            slot.distance = subtract(cached_distance, r30);
                            slot.timer = 0.;
                            keep = Some((b, cached_distance < 0. || cached_distance.is_nan()));
                        }
                    } else if let Some(b) = world.body(entry.id) {
                        if (0..3).all(|k| low[k] <= b.bounds[1][k] && b.bounds[0][k] <= high[k]) {
                            let mut distance = entry.distance;
                            let body_velocity = b.motion.point_velocity(p);
                            if 0. < distance {
                                let relative: [f32; 3] = std::array::from_fn(|k| multiply(subtract(body_velocity[k], v[k]), dt));
                                distance = subtract(distance, length(squared(relative)));
                                self.neighbours.entries[index].distance = distance;
                            }
                            let timer = entry.timer;
                            let running = 0. <= timer;
                            if running {
                                self.neighbours.entries[index].timer = add(dt, timer);
                            }
                            let need = REEVALUATE < timer || distance <= 0. || !b.active;
                            let mut behind = false;
                            if statics[i].hit {
                                let n = statics[i].plane;
                                let (lo, hi) = (b.bounds[0], b.bounds[1]);
                                let side = |k: usize| if 0. <= n[k] { 1. } else { -1. };
                                let extent = |k: usize| multiply(multiply(multiply(side(k), subtract(hi[k], lo[k])), 0.5), n[k]);
                                let middle = |k: usize| multiply(multiply(add(lo[k], hi[k]), 0.5), n[k]);
                                let top = add(add(add(add(add(add(extent(1), extent(0)), extent(2)), n[3]), middle(1)), middle(2)), middle(0));
                                behind = top < 0.;
                            }
                            if !need || reevaluated >= budget || behind {
                                keep = Some((b, false));
                            } else {
                                reevaluated += 1;
                                let found = Self::closest(&mut self.query, b, p, q)?;
                                let (best, distance) = match found.best {
                                    None => (found.best_distance, q),
                                    Some(hit) => {
                                        main.add(hit);
                                        if found.second < main.second {
                                            main.second = found.second;
                                            main.early = minps(main.early, found.second);
                                        }
                                        (found.best_distance, found.best_distance)
                                    }
                                };
                                // ucomiss: unordered compares as below / equal.
                                let close = !(best >= multiply(rq, f32::from_bits(0x3f8c_cccd)));
                                let slow = !close && running;
                                // ucomiss / sete: unordered compares equal.
                                let same = main.best_distance == best || main.best_distance.is_nan() || best.is_nan();
                                if overlap && (same || close || slow) && states[i].state >= 3 {
                                    if cached_velocity == [0.; 4] {
                                        if let Some(motion) = cached_motion {
                                            cached_velocity = motion.point_velocity(p);
                                        }
                                    }
                                    let d: [f32; 3] = std::array::from_fn(|k| subtract(cached_velocity[k], body_velocity[k]));
                                    if slow || squared(d) > wake {
                                        states[i].state = 2;
                                        states[i].timer = 0.;
                                    }
                                }
                                let slot = &mut self.neighbours.entries[index];
                                slot.distance = subtract(distance, r30);
                                slot.timer = if close { 0. } else { IDLE };
                                keep = Some((b, !(best >= 0.)));
                            }
                        }
                    }
                    // A sleeping body that does not touch drops out.
                    let kept = match keep {
                        Some((b, penetrating)) if !b.active && !penetrating => {
                            inactive_limit = minps(inactive_limit, main.best_distance);
                            statics[i].flags |= 2;
                            false
                        }
                        Some(_) => true,
                        None => false,
                    };
                    if !kept {
                        let slot = &mut self.neighbours.entries[index];
                        if !(0. > slot.timer) && states[i].state >= 3 {
                            states[i].state = 2;
                            states[i].timer = 0.;
                        }
                        slot.id = NO_BODY;
                        if entry.id == e.track.body {
                            e.track = NO_TRACK;
                        }
                    }
                    n = (self.neighbours.next[index] & LINK) as i32;
                }
            }
            // Store the nearest hit in the cache.
            let second = main.second;
            match main.best {
                None => {
                    let r = maxps(rq, EPSILON);
                    let speed = length(add(add(multiply(v[1], v[1]), multiply(v[0], v[0])), multiply(v[2], v[2])));
                    e.plane = [0.; 4];
                    e.body = NO_BODY;
                    e.key = 0xffff;
                    e.flags &= 3;
                    e.sphere = [p[0], p[1], p[2], add(r, multiply(speed, dt))];
                }
                Some(hit) if !cache_valid || main.best_distance < cached_distance => {
                    let b = world.bodies[(hit.body & NO_BODY) as usize].as_ref().ok_or("a hit moving body was removed")?;
                    e.track = new_track;
                    e.flags &= 3;
                    let x = &b.transform;
                    let t = x[3];
                    let d: [f32; 3] = std::array::from_fn(|k| subtract(p[k], t[k]));
                    let at = [column_dot(x[0], d), column_dot(x[1], d), column_dot(x[2], d)];
                    e.body = hit.body;
                    e.key = hit.material;
                    let hn = [hit.normal[0], hit.normal[1], hit.normal[2]];
                    let n = [column_dot(x[0], hn), column_dot(x[1], hn), column_dot(x[2], hn)];
                    let dh: [f32; 3] = std::array::from_fn(|k| subtract(hit.position[k], t[k]));
                    let w = subtract(
                        0.,
                        add(add(multiply(column_dot(x[1], dh), n[1]), multiply(column_dot(x[0], dh), n[0])), multiply(column_dot(x[2], dh), n[2])),
                    );
                    e.plane = [n[0], n[1], n[2], w];
                    let (a, bb, c) = (multiply(n[0], at[0]), multiply(n[1], at[1]), multiply(n[2], at[2]));
                    let gap = add(add(w, bb), add(c, a));
                    // The convex leaf (getLeafShape at the hit key, 1490-2470).
                    e.flags |= 4;
                    let mut radius = gap;
                    if hit.connectivity {
                        e.flags |= 8;
                        // The track's face byte is 0 for convex leaves.
                        radius = maxps(gap, minps(second, gap));
                    }
                    let radius = if 0. <= gap && EPSILON <= radius && gap <= capped { minps(radius, capped) } else { gap };
                    e.sphere = [at[0], at[1], at[2], radius];
                }
                Some(_) => {
                    if 0. < main.best_distance && second != FAR {
                        let mut limit = main.best_distance;
                        if 0. <= main.best_distance && EPSILON <= second && main.best_distance <= FAR {
                            limit = minps(second, FAR);
                        }
                        let d: [f32; 3] = std::array::from_fn(|k| subtract(e.sphere[k], local[k]));
                        let r = maxps(0., subtract(e.sphere[3], length(squared(d))));
                        e.sphere = [local[0], local[1], local[2], minps(limit, r)];
                    }
                    if e.track.body & NO_BODY == NO_BODY {
                        e.track = new_track;
                    }
                }
            }
            if inactive_limit < FAR {
                e.sphere[3] = maxps(0., minps(e.sphere[3], inactive_limit));
            }
            // The DynamicContact channels.
            let has = e.body != NO_BODY;
            let changed = !(had == has && old_key == e.key);
            if had || has {
                collide.dynamic_contacts[i] = Contact::default();
                if changed {
                    collide.dynamic_materials[i] = NO_MATERIAL;
                }
            }
            if has {
                let b = world.bodies[(e.body & NO_BODY) as usize].as_ref().ok_or("a cached moving body was removed")?;
                let x = &b.transform;
                let n = e.plane;
                let world_normal: [f32; 4] =
                    std::array::from_fn(|k| add(add(multiply(n[1], x[1][k]), multiply(n[0], x[0][k])), multiply(n[2], x[2][k])));
                let t = x[3];
                let w = subtract(
                    n[3],
                    add(add(multiply(t[1], world_normal[1]), multiply(t[0], world_normal[0])), multiply(t[2], world_normal[2])),
                );
                let mut contact = Contact {
                    plane: [world_normal[0], world_normal[1], world_normal[2], w],
                    linear_velocity: b.motion.linear,
                    angular_velocity: b.motion.world_angular(),
                    centre_of_mass: b.motion.centre,
                };
                let l = plane;
                if l[0] != 0. || l[1] != 0. || l[2] != 0. {
                    let c = &mut contact;
                    let dot = add(add(multiply(l[1], c.plane[1]), multiply(l[0], c.plane[0])), multiply(l[2], c.plane[2]));
                    c.plane = std::array::from_fn(|k| subtract(c.plane[k], multiply(dot, l[k])));
                    let u = c.linear_velocity;
                    let dot = add(add(multiply(u[1], l[1]), multiply(u[0], l[0])), multiply(u[2], l[2]));
                    c.linear_velocity = std::array::from_fn(|k| subtract(u[k], multiply(dot, l[k])));
                    let a = c.angular_velocity;
                    let dot = add(add(multiply(l[1], a[1]), multiply(l[0], a[0])), multiply(l[2], a[2]));
                    c.angular_velocity = std::array::from_fn(|k| multiply(dot, l[k]));
                    let m = c.centre_of_mass;
                    let (x0, x1, x2, x3) = (multiply(m[0], l[0]), multiply(m[1], l[1]), multiply(m[2], l[2]), l[3]);
                    let sums = [
                        add(add(add(x3, x1), x2), x0),
                        add(add(add(x2, x0), x3), x1),
                        add(add(add(x1, x3), x0), x2),
                        add(add(add(x0, x2), x1), x3),
                    ];
                    c.centre_of_mass = std::array::from_fn(|k| subtract(m[k], multiply(sums[k], l[k])));
                }
                collide.dynamic_contacts[i] = contact;
                if changed {
                    let material = world.materials.get(usize::from(e.key)).copied().filter(|_| e.key != 0xffff);
                    collide.dynamic_materials[i] = contact_material(material, own, 3 <= states[i].state)?;
                }
            }
            shift(&mut e);
            self.cache[i] = e;
        }
        Ok(())
    }
}

/// Runs tests/fixtures/motion_track_cases.txt (1412b7990 executed by
/// analysis/probe_motion_track.py) and returns (cases, mismatches).
#[doc(hidden)]
pub fn compare_motion_track_fixture(text: &str) -> (usize, Vec<String>) {
    let mut failures = Vec::new();
    let mut cases = 0;
    for line in text.lines().filter(|line| !line.starts_with('#') && !line.is_empty()) {
        let w: Vec<u32> = line.split_whitespace().map(|word| u32::from_str_radix(word, 16).unwrap()).collect();
        let f = |i: usize| f32::from_bits(w[i]);
        let four = |i: usize| [f(i), f(i + 1), f(i + 2), f(i + 3)];
        let mut track = Track { distance: f(0), valid: 1, face: 0, body: 0, key: 0 };
        let motion = MotionRecord { centre: four(10), orientation: four(14), linear: four(18), angular: four(22) };
        track.advance(four(1), four(5), f(9), &motion);
        cases += 1;
        if track.distance.to_bits() != w[26] || u32::from(track.valid) != w[27] {
            failures.push(format!("{line}: got {:08x} {}", track.distance.to_bits(), track.valid));
        }
    }
    (cases, failures)
}

#[cfg(test)]
mod tests {
    #[test]
    fn motion_track_matches_executed_original() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/motion_track_cases.txt")).unwrap();
        let (cases, failures) = super::compare_motion_track_fixture(&text);
        assert!(cases > 0 && failures.is_empty(), "{} of {cases}: {:?}", failures.len(), &failures[..failures.len().min(5)]);
    }
}
