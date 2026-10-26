//! The player's character proxy (hknpCharacterProxy, ctor FUN_140084b70 /
//! FUN_140c82680): contact manifold, surface constraints, the sub-stepped
//! integrate FUN_140085a40 and the support test FUN_140c82a50.
//!
//! The world is the level's static collision, so every body is static
//! (priority 2, zero velocity); dynamic-body pushes (FUN_140c847a0) and
//! trigger bookkeeping have nothing to act on and are not ported.
use super::simplex_solver::{self, SolverInput, SurfaceConstraint};
use crate::gameplay::collision_filter::QueryFilter;
use bevy_math::{Mat3, Vec3};

pub const INVALID_BODY: u32 = 0x00ff_ffff;
const EPSILON: f32 = f32::EPSILON;

/// One world query result (hknpCollisionResult as the proxy reads it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldHit {
    pub position: Vec3,
    /// Out of the surface, toward the queried shape.
    pub normal: Vec3,
    /// Cast fraction for cast hits; separation for closest-point hits.
    pub value: f32,
    pub body: u32,
    pub key: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub distance: f32,
    pub position: Vec3,
    pub normal: Vec3,
    pub body: u32,
    pub key: u32,
}

/// The two hknp world queries the player uses (world+0x478), in game
/// coordinates (Z up, metres).
pub trait PlayerWorld {
    /// `castShape`: hits of `shape` (player-space points, rotated by
    /// `rotation`, placed at `from`) along `displacement`, with first-contact
    /// fractions in [0, 1]; when `tolerance > 0`, also the closest points of
    /// the start pose within `tolerance` into `start`.
    fn cast_shape(&self, filter: QueryFilter,
        shape: &[Vec3],
        rotation: Mat3,
        from: Vec3,
        displacement: Vec3,
        tolerance: f32,
        cast: &mut Vec<WorldHit>,
        start: Option<&mut Vec<WorldHit>>,
    );
    fn cast_ray(&self, filter: QueryFilter, from: Vec3, direction: Vec3, length: f32) -> Option<RayHit>;
    /// The physics material flags of a hit's body and shape key
    /// (FUN_1400922d0 + FUN_140092c10 → arkPhysicsMaterial +0x80).
    fn surface(&self, hit: &WorldHit) -> SurfaceFlags;
    /// Resolved arkPhysicsMaterial declaration name for a shape hit. Worlds
    /// without material metadata leave the native material pointer unresolved.
    fn material(&self, _hit: &WorldHit) -> Option<&str> { None }
}

/// arkPhysicsMaterial flag byte +0x80 bits the player sensors read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceFlags {
    pub allow_mantle: bool,
    pub allow_vault: bool,
}

/// hknpCharacterProxyCinfo values as arkPhysicsPlayer fills them
/// (FUN_1401b2290 from arkPhysicsPlayerSettings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProxySettings {
    pub dynamic_friction: f32,
    pub static_friction: f32,
    /// 0.01 (DAT_141ab3810), overwriting the cinfo default.
    pub keep_distance: f32,
    /// 0.1 (DAT_141ab3b0c).
    pub keep_contact_tolerance: f32,
    pub contact_angle_sensitivity: f32,
    pub user_planes: i32,
    pub max_character_speed_for_solver: f32,
    pub character_strength: f32,
    pub character_mass: f32,
    /// cos(maxSlope), proxy+0x130.
    pub max_slope_cosine: f32,
    pub penetration_recovery_speed: f32,
    pub max_cast_iterations: i32,
    pub refresh_manifold_in_check_support: bool,
    /// collisionFilterInfo (cinfo +0x88 → proxy +0x3c); both proxy casts
    /// run the world's arkCollisionFilter with it.
    pub collision_filter: u32,
}

/// hknp SupportedState.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SupportState {
    #[default]
    Unsupported = 0,
    Sliding = 1,
    Supported = 2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceInfo {
    pub state: SupportState,
    pub normal: Vec3,
    pub velocity: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ManifoldPoint {
    position: Vec3,
    normal: Vec3,
    distance: f32,
    fraction: f32,
    body: u32,
    key: u32,
}

impl ManifoldPoint {
    fn start(hit: &WorldHit) -> Self {
        Self { position: hit.position, normal: hit.normal, distance: hit.value, fraction: 0.0, body: hit.body, key: hit.key }
    }

    fn cast(hit: &WorldHit, displacement: Vec3) -> Self {
        Self {
            position: hit.position,
            normal: hit.normal,
            distance: -hit.normal.dot(displacement) * hit.value,
            fraction: hit.value,
            body: hit.body,
            key: hit.key,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CharacterProxy {
    pub position: Vec3,
    pub rotation: Mat3,
    pub velocity: Vec3,
    /// +0xc0, the displacement of the last sub-step.
    pub displacement: Vec3,
    pub up: Vec3,
    pub settings: ProxySettings,
    shape: Vec<Vec3>,
    manifold: Vec<ManifoldPoint>,
}

impl CharacterProxy {
    pub fn new(position: Vec3, rotation: Mat3, shape: Vec<Vec3>, settings: ProxySettings) -> Self {
        Self {
            position,
            rotation,
            velocity: Vec3::ZERO,
            displacement: Vec3::ZERO,
            up: Vec3::Z,
            settings,
            shape,
            manifold: Vec::new(),
        }
    }

    /// FUN_140c84530: a new shape clears the manifold.
    pub fn set_shape(&mut self, shape: Vec<Vec3>) {
        self.shape = shape;
        self.manifold.clear();
    }

    pub fn contact_count(&self) -> usize {
        self.manifold.len()
    }

    /// Diagnostic: (position, normal, distance, triangle key) per contact.
    pub fn contacts(&self) -> Vec<(Vec3, Vec3, f32, u32)> {
        self.manifold.iter().map(|m| (m.position, m.normal, m.distance, m.key)).collect()
    }

    /// FUN_140c8ae90.
    fn metric(&self, a: &ManifoldPoint, b: &ManifoldPoint) -> f32 {
        let sensitivity = self.settings.contact_angle_sensitivity;
        // 10.0 (DAT_141abf158); static bodies have no surface velocity.
        (1.0 - a.normal.dot(b.normal)) * sensitivity * sensitivity * 10.0 + (a.distance - b.distance).powi(2)
    }

    /// FUN_140c8b110.
    fn find_surface(&self, point: &ManifoldPoint, among: &[ManifoldPoint], tolerance: f32) -> Option<usize> {
        if point.body == INVALID_BODY {
            return None;
        }
        let mut best: Option<(usize, f32)> = None;
        for (index, other) in among.iter().enumerate() {
            let metric = self.metric(point, other);
            if metric < tolerance && best.is_none_or(|(_, m)| metric < m) {
                best = Some((index, metric));
            }
        }
        best.map(|(index, _)| index)
    }

    /// FUN_140c8a250.
    fn update_manifold(&mut self, start: &[WorldHit], cast: &[WorldHit], displacement: Vec3) {
        if start.is_empty() {
            self.manifold.clear();
        } else {
            let mut fresh: Vec<ManifoldPoint> = start.iter().map(ManifoldPoint::start).collect();
            let min_distance = fresh.iter().map(|p| p.distance).fold(f32::MAX, f32::min);
            let mut index = self.manifold.len();
            while index > 0 {
                index -= 1;
                // 1.1 (DAT_141b01ad4).
                match self.find_surface(&self.manifold[index], &fresh, 1.1) {
                    None => {
                        self.manifold.swap_remove(index);
                    }
                    Some(k) => {
                        self.manifold[index] = fresh.swap_remove(k);
                    }
                }
            }
            if let Some(j) = fresh.iter().position(|p| p.distance == min_distance) {
                let point = fresh[j];
                match self.find_surface(&point, &self.manifold, 0.1) {
                    None => self.manifold.push(point),
                    Some(k) => self.manifold[k] = point,
                }
            }
        }
        if let Some(hit) = cast.first() {
            let point = ManifoldPoint::cast(hit, displacement);
            if self.find_surface(&point, &self.manifold, 0.1).is_none() {
                self.manifold.push(point);
            }
        }
        let mut i = self.manifold.len();
        while i > 1 {
            i -= 1;
            if (0..i).rev().any(|j| self.metric(&self.manifold[i], &self.manifold[j]) < 0.1) {
                self.manifold.swap_remove(i);
            }
        }
    }

    /// FUN_140c8a9e0 → FUN_140c8aae0 + FUN_140c8ad10.
    fn surface_constraints(&self) -> Vec<SurfaceConstraint> {
        let settings = &self.settings;
        let mut constraints = Vec::with_capacity(self.manifold.len() + 10 + settings.user_planes.max(0) as usize);
        for point in &self.manifold {
            let mut constraint = SurfaceConstraint {
                normal: point.normal,
                distance: point.distance - settings.keep_distance,
                velocity: Vec3::ZERO,
                static_friction: settings.static_friction,
                extra_up_static_friction: 0.0,
                extra_down_static_friction: 0.0,
                dynamic_friction: settings.dynamic_friction,
                priority: 0,
            };
            if point.body != INVALID_BODY {
                // Static bodies (flags bit 0): no motion, priority 2.
                constraint.priority = 2;
            }
            if constraint.distance < -EPSILON {
                constraint.velocity -= point.normal * (settings.penetration_recovery_speed * constraint.distance);
                constraint.distance = 0.0;
            }
            constraints.push(constraint);
            let d = constraint.normal.dot(self.up);
            if 0.01 < d && d < settings.max_slope_cosine {
                let mut wall = constraint;
                wall.normal -= self.up * d;
                let scale = 1.0 / wall.normal.length();
                wall.normal *= scale;
                wall.distance *= scale;
                constraints.push(wall);
            }
        }
        constraints
    }

    fn solver_input<'a>(&self, velocity: Vec3, delta_time: f32, min_delta_time: f32, constraints: &'a [SurfaceConstraint]) -> SolverInput<'a> {
        SolverInput {
            position: Vec3::ZERO,
            velocity,
            max_surface_velocity: Vec3::splat(self.settings.max_character_speed_for_solver),
            up: self.up,
            delta_time,
            min_delta_time,
            constraints,
        }
    }

    /// FUN_140085a40 with stepInfo {0, dt, dt, 1/dt}.
    pub fn integrate(&mut self, world: &dyn PlayerWorld, delta_time: f32) {
        let settings = self.settings;
        let mut remaining = delta_time;
        let mut output_velocity = Vec3::ZERO;
        self.displacement = self.velocity * delta_time;
        let mut cast = Vec::new();
        let mut start = Vec::new();
        let mut iteration = 0;
        // 1.1920929e-7 (DAT_141ad2734).
        while remaining > EPSILON && iteration < settings.max_cast_iterations {
            cast.clear();
            start.clear();
            world.cast_shape(QueryFilter::World(self.settings.collision_filter), &self.shape,
                self.rotation,
                self.position,
                self.displacement,
                settings.keep_distance + settings.keep_contact_tolerance,
                &mut cast,
                Some(&mut start),
            );
            cast.sort_by(|a, b| a.value.total_cmp(&b.value));
            start.sort_by(|a, b| a.value.total_cmp(&b.value));
            self.update_manifold(&start, &cast, self.displacement);
            let constraints = self.surface_constraints();
            let speed2 = self.velocity.length_squared();
            // 0.5 (DAT_141ab3764).
            let min_delta_time = if speed2 == 0.0 { 0.0 } else { 0.5 * settings.keep_distance / speed2.sqrt() };
            let output = simplex_solver::solve(&self.solver_input(self.velocity, remaining, min_delta_time, &constraints));
            let mut displacement = output.position;
            output_velocity = output.velocity;
            let mut time = output.delta_time;
            // 1.4210855e-14 (0x1423cce40), 0.001.
            if displacement.length_squared() > 1.421_085_5e-14
                && !(self.displacement - displacement).abs().cmple(Vec3::splat(0.001)).all()
            {
                cast.clear();
                world.cast_shape(QueryFilter::World(self.settings.collision_filter), &self.shape, self.rotation, self.position, displacement, 0.0, &mut cast, None);
                cast.sort_by(|a, b| a.value.total_cmp(&b.value));
                for hit in &cast {
                    let point = ManifoldPoint::cast(hit, displacement);
                    if self.find_surface(&point, &self.manifold, 0.1).is_some() {
                        continue;
                    }
                    self.manifold.push(point);
                    let length = displacement.length();
                    let approach = -point.normal.dot(displacement) / length;
                    let fraction = if approach > 0.0 {
                        point.fraction - settings.keep_distance * (1.0 / approach) / length
                    } else {
                        point.fraction
                    };
                    let scale = fraction.clamp(0.0, 1.0);
                    time *= scale;
                    displacement *= scale;
                    break;
                }
            }
            remaining -= time;
            self.position += displacement;
            self.displacement = displacement;
            iteration += 1;
        }
        self.velocity = output_velocity;
    }

    /// FUN_140c82a50 (refreshManifoldInCheckSupport is false for the player).
    pub fn check_support(&self, direction: Vec3) -> SurfaceInfo {
        // The caller's surface info starts as {SUPPORTED, (0,0,1)}; only the
        // solver path clears its normal.
        let unsupported = SurfaceInfo { state: SupportState::Unsupported, normal: Vec3::ZERO, velocity: Vec3::ZERO };
        if self.manifold.is_empty() {
            return SurfaceInfo { normal: Vec3::Z, ..unsupported };
        }
        let mut constraints = self.surface_constraints();
        let saved: Vec<Vec3> = constraints.iter().map(|c| c.velocity).collect();
        for constraint in &mut constraints {
            constraint.velocity = Vec3::ZERO;
        }
        // 1/60 (0x3c888889).
        let step = f32::from_bits(0x3c88_8889);
        let output = simplex_solver::solve(&self.solver_input(direction, step, step, &constraints));
        if (output.velocity - direction).abs().cmple(Vec3::splat(0.001)).all() {
            return unsupported;
        }
        let state = if output.velocity.length_squared() < 0.001 {
            SupportState::Supported
        } else {
            let c = output.velocity.normalize().dot(direction);
            if self.settings.max_slope_cosine.powi(2) <= 1.0 - c * c {
                SupportState::Supported
            } else {
                SupportState::Sliding
            }
        };
        let mut normal = Vec3::ZERO;
        let mut velocity = Vec3::ZERO;
        let mut count = 0;
        for (index, constraint) in constraints.iter().enumerate() {
            // -0.08 (DAT_141ec3358).
            if output.interactions[index].touched && direction.dot(constraint.normal) < -0.08 {
                count += 1;
                normal += constraint.normal;
                velocity += saved[index];
            }
        }
        if count < 1 {
            return unsupported;
        }
        SurfaceInfo { state, normal: normal / normal.length(), velocity: velocity / count as f32 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::player::shape::{PostureShape, ShapeSettings};
    use crate::gameplay::shape_cast;

    /// A flat floor at z = 0 and an optional wall at x = `wall`.
    pub(crate) struct Planes {
        pub triangles: Vec<[Vec3; 3]>,
    }

    impl PlayerWorld for Planes {
        fn surface(&self, _hit: &WorldHit) -> SurfaceFlags {
            SurfaceFlags { allow_mantle: true, allow_vault: true }
        }

        fn cast_shape(&self, _: QueryFilter, shape: &[Vec3], rotation: Mat3, from: Vec3, displacement: Vec3, tolerance: f32, cast: &mut Vec<WorldHit>, start: Option<&mut Vec<WorldHit>>) {
            let points: Vec<Vec3> = shape.iter().map(|p| from + rotation * *p).collect();
            let mut start = start;
            for (key, triangle) in self.triangles.iter().enumerate() {
                if let Some((fraction, contact)) = shape_cast::cast(&points, displacement, triangle, 0.001) {
                    cast.push(WorldHit { position: contact.position, normal: contact.normal, value: fraction, body: 0, key: key as u32 });
                }
                if let Some(start) = start.as_deref_mut().filter(|_| tolerance > 0.0) {
                    let contact = shape_cast::closest(&points, triangle);
                    if contact.distance <= tolerance {
                        start.push(WorldHit { position: contact.position, normal: contact.normal, value: contact.distance, body: 0, key: key as u32 });
                    }
                }
            }
        }
        fn cast_ray(&self, _: QueryFilter, _: Vec3, _: Vec3, _: f32) -> Option<RayHit> {
            None
        }
    }

    pub(crate) fn floor() -> Vec<[Vec3; 3]> {
        vec![
            [Vec3::new(-50.0, -50.0, 0.0), Vec3::new(50.0, -50.0, 0.0), Vec3::new(50.0, 50.0, 0.0)],
            [Vec3::new(-50.0, -50.0, 0.0), Vec3::new(50.0, 50.0, 0.0), Vec3::new(-50.0, 50.0, 0.0)],
        ]
    }

    pub(crate) fn settings() -> ProxySettings {
        ProxySettings {
            dynamic_friction: 1.0,
            static_friction: 0.0,
            keep_distance: 0.01,
            keep_contact_tolerance: 0.1,
            contact_angle_sensitivity: 10.0,
            user_planes: 4,
            max_character_speed_for_solver: 10.0,
            character_strength: 10000.0,
            character_mass: 55.0,
            max_slope_cosine: 65f32.to_radians().cos(),
            penetration_recovery_speed: 10.0,
            max_cast_iterations: 5,
            refresh_manifold_in_check_support: false,
            collision_filter: 0,
        }
    }

    fn standing() -> Vec<Vec3> {
        PostureShape::new(ShapeSettings { radius: 0.4, height: 1.8, step_height: 0.5, gravity_strength: 3.0, ..Default::default() }).body
    }

    #[test]
    fn falling_proxy_stops_keep_distance_above_the_floor_and_is_supported() {
        let world = Planes { triangles: floor() };
        let mut proxy = CharacterProxy::new(Vec3::new(0.0, 0.0, 0.3), Mat3::IDENTITY, standing(), settings());
        for _ in 0..30 {
            proxy.velocity = Vec3::new(0.0, 0.0, -5.0);
            proxy.integrate(&world, 1.0 / 60.0);
        }
        assert!((proxy.position.z - 0.01).abs() < 0.003, "{:?}", proxy.position);
        let support = proxy.check_support(-Vec3::Z);
        assert_eq!(support.state, SupportState::Supported);
        assert!(support.normal.distance(Vec3::Z) < 1e-4);
    }

    #[test]
    fn walking_into_a_wall_slides_along_it() {
        let mut triangles = floor();
        triangles.push([Vec3::new(1.0, -50.0, -1.0), Vec3::new(1.0, 50.0, -1.0), Vec3::new(1.0, 0.0, 50.0)]);
        let world = Planes { triangles };
        let mut proxy = CharacterProxy::new(Vec3::new(0.0, 0.0, 0.01), Mat3::IDENTITY, standing(), settings());
        for _ in 0..120 {
            proxy.velocity = Vec3::new(3.0, 3.0, 0.0);
            proxy.integrate(&world, 1.0 / 60.0);
        }
        assert!(proxy.position.x < 1.0 - 0.4 + 0.001, "{:?}", proxy.position);
        assert!(proxy.position.x > 1.0 - 0.4 - 0.05, "{:?}", proxy.position);
        assert!(proxy.position.y > 5.0, "{:?}", proxy.position);
    }

    fn stairs(rise: f32, run: f32, count: usize) -> Vec<[Vec3; 3]> {
        let mut triangles = floor();
        for k in 0..count {
            let x = 1.0 + k as f32 * run;
            let (z0, z1) = (k as f32 * rise, (k + 1) as f32 * rise);
            triangles.push([Vec3::new(x, -50.0, z0), Vec3::new(x, 50.0, z0), Vec3::new(x, 0.0, z1)]);
            triangles.push([Vec3::new(x, -50.0, z0), Vec3::new(x, 50.0, z1), Vec3::new(x, -50.0, z1)]);
            triangles.push([Vec3::new(x, 50.0, z0), Vec3::new(x, 50.0, z1), Vec3::new(x, 0.0, z1)]);
            triangles.push([Vec3::new(x, -50.0, z1), Vec3::new(x + run, -50.0, z1), Vec3::new(x + run, 50.0, z1)]);
            triangles.push([Vec3::new(x, -50.0, z1), Vec3::new(x + run, 50.0, z1), Vec3::new(x, 50.0, z1)]);
        }
        triangles
    }

    #[test]
    fn stairs_are_climbed_at_run_and_sprint_speed() {
        for speed in [4.14, 7.13] {
            let world = Planes { triangles: stairs(0.2, 0.3, 10) };
            let mut proxy = CharacterProxy::new(Vec3::new(0.0, 0.0, 0.01), Mat3::IDENTITY, standing(), settings());
            for _ in 0..240 {
                // Gravity as the physics step composes it while supported is 0;
                // a downward component keeps contact like the airborne case.
                proxy.velocity = Vec3::new(speed, 0.0, 0.0);
                proxy.integrate(&world, 1.0 / 120.0);
            }
            assert!(proxy.position.z > 1.9, "speed {speed}: {:?}", proxy.position);
        }
    }

    #[test]
    fn the_foot_cone_climbs_a_step_lower_than_step_height() {
        let mut triangles = floor();
        // A 0.3 m step face at x = 1 and its top.
        triangles.push([Vec3::new(1.0, -50.0, 0.0), Vec3::new(1.0, 50.0, 0.0), Vec3::new(1.0, 0.0, 0.3)]);
        triangles.push([Vec3::new(1.0, -50.0, 0.3), Vec3::new(50.0, -50.0, 0.3), Vec3::new(50.0, 50.0, 0.3)]);
        triangles.push([Vec3::new(1.0, -50.0, 0.3), Vec3::new(50.0, 50.0, 0.3), Vec3::new(1.0, 50.0, 0.3)]);
        let world = Planes { triangles };
        let mut proxy = CharacterProxy::new(Vec3::new(0.0, 0.0, 0.01), Mat3::IDENTITY, standing(), settings());
        for _ in 0..120 {
            proxy.velocity = Vec3::new(3.6, 0.0, 0.0);
            proxy.integrate(&world, 1.0 / 60.0);
        }
        assert!(proxy.position.x > 3.0, "{:?}", proxy.position);
        assert!(proxy.position.z > 0.29, "{:?}", proxy.position);
    }
}
