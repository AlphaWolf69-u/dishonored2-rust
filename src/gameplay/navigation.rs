//! Triangle index over the level's static collision decoded from the game's
//! authored Havok physics world, with owner activation for collected pickups.
//! Coordinates are Bevy Y-up metres; the player queries it through
//! `player::character_proxy::PlayerWorld`.
use bevy_math::Vec3;
use std::collections::{HashMap, HashSet};

const CELL: f32 = 4.0;

#[derive(Clone, Copy)]
struct Triangle {
    a: Vec3,
    b: Vec3,
    c: Vec3,
    min: Vec3,
    max: Vec3,
    owner: u32,
}

pub struct CollisionWorld {
    triangles: Vec<Triangle>,
    cells: HashMap<(i32, i32, i32), Vec<usize>>,
    large: Vec<usize>,
    owner_ids: HashMap<String, Vec<u32>>,
    owner_active: Vec<bool>,
}

impl CollisionWorld {
    pub fn new(source: impl IntoIterator<Item = [Vec3; 3]>) -> Self {
        Self::with_owned(
            source.into_iter().map(|triangle| (triangle, u32::MAX)),
            Vec::new(),
        )
    }

    pub fn with_owned(
        source: impl IntoIterator<Item = ([Vec3; 3], u32)>,
        owners: Vec<String>,
    ) -> Self {
        let mut owner_ids = HashMap::<String, Vec<u32>>::new();
        for (index, id) in owners.iter().enumerate() {
            owner_ids.entry(id.clone()).or_default().push(index as u32);
        }
        let mut world = Self {
            triangles: Vec::new(),
            cells: HashMap::new(),
            large: Vec::new(),
            owner_ids,
            owner_active: vec![true; owners.len()],
        };
        for ([a, b, c], owner) in source {
            if owner != u32::MAX && owner as usize >= world.owner_active.len() {
                continue;
            }
            if (b - a).cross(c - a).length_squared() < 1e-10 {
                continue;
            }
            let min = a.min(b).min(c);
            let max = a.max(b).max(c);
            let index = world.triangles.len();
            world.triangles.push(Triangle {
                a,
                b,
                c,
                min,
                max,
                owner,
            });
            let low = cell(min);
            let high = cell(max);
            let cells = (high.0 - low.0 + 1) as i64
                * (high.1 - low.1 + 1) as i64
                * (high.2 - low.2 + 1) as i64;
            if cells > 4096 {
                world.large.push(index);
                continue;
            }
            for x in low.0..=high.0 {
                for y in low.1..=high.1 {
                    for z in low.2..=high.2 {
                        world.cells.entry((x, y, z)).or_default().push(index);
                    }
                }
            }
        }
        world
    }

    pub fn set_owner_active(&mut self, id: &str, active: bool) -> bool {
        let Some(indices) = self.owner_ids.get(id) else {
            return false;
        };
        for &index in indices {
            self.owner_active[index as usize] = active;
        }
        true
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    pub fn large_triangle_count(&self) -> usize {
        self.large.len()
    }

    pub fn nearby_triangle_count(&self, point: Vec3, radius: f32) -> usize {
        self.candidates(point - Vec3::splat(radius), point + Vec3::splat(radius))
            .len()
    }

    /// First hit distance along a finite segment, testing both triangle sides.
    pub fn first_hit(&self, start: Vec3, end: Vec3) -> Option<f32> {
        self.first_hit_details(start, end).map(|hit| hit.0)
    }

    /// Diagnostic provenance of the same nearest hit used by first_hit.
    pub fn first_hit_details(&self, start: Vec3, end: Vec3) -> Option<(f32, u32, [Vec3; 3])> {
        let segment = end - start;
        let length = segment.length();
        if length <= 1e-5 {
            return None;
        }
        let direction = segment / length;
        self.candidates(start.min(end), start.max(end))
            .into_iter()
            .filter_map(|index| {
                let triangle = self.triangles[index];
                let distance = segment_triangle_hit(start, direction, triangle)?;
                (distance > 0.001 && distance < length).then_some((
                    distance,
                    triangle.owner,
                    [triangle.a, triangle.b, triangle.c],
                ))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    pub fn owner_name(&self, owner: u32) -> Option<&str> {
        self.owner_ids
            .iter()
            .find_map(|(name, indices)| indices.contains(&owner).then_some(name.as_str()))
    }

    /// Active triangles whose bounds overlap `min..max`: (index, vertices,
    /// owner), owner `u32::MAX` for level geometry.
    pub fn triangles_in(&self, min: Vec3, max: Vec3) -> Vec<(u32, [Vec3; 3], u32)> {
        let mut indices = self.candidates(min, max);
        // Ascending index keeps query results independent of hash order.
        indices.sort_unstable();
        indices
            .into_iter()
            .map(|index| {
                let t = self.triangles[index];
                (index as u32, [t.a, t.b, t.c], t.owner)
            })
            .collect()
    }

    fn candidates(&self, min: Vec3, max: Vec3) -> Vec<usize> {
        let low = cell(min);
        let high = cell(max);
        let mut found = HashSet::new();
        found.extend(self.large.iter().copied());
        for x in low.0..=high.0 {
            for y in low.1..=high.1 {
                for z in low.2..=high.2 {
                    if let Some(indices) = self.cells.get(&(x, y, z)) {
                        found.extend(indices.iter().copied());
                    }
                }
            }
        }
        found
            .into_iter()
            .filter(|&index| {
                let triangle = self.triangles[index];
                (triangle.owner == u32::MAX || self.owner_active[triangle.owner as usize])
                    && (0..3).all(|axis| {
                        triangle.min[axis] <= max[axis] && triangle.max[axis] >= min[axis]
                    })
            })
            .collect()
    }

    /// Highest walkable surface below/within one step above the proposed feet.
    pub fn floor_at(&self, feet: Vec3, step_up: f32, down: f32) -> Option<f32> {
        let min = feet + Vec3::new(-0.02, -down, -0.02);
        let max = feet + Vec3::new(0.02, step_up, 0.02);
        let mut best: Option<f32> = None;
        for index in self.candidates(min, max) {
            let triangle = self.triangles[index];
            let normal = (triangle.b - triangle.a)
                .cross(triangle.c - triangle.a)
                .normalize_or_zero();
            if normal.y.abs() < 65.0_f32.to_radians().cos() {
                continue;
            }
            let Some(height) = vertical_intersection(feet.x, feet.z, triangle) else {
                continue;
            };
            if height < feet.y - down || height > feet.y + step_up {
                continue;
            }
            best = Some(best.map_or(height, |previous| previous.max(height)));
        }
        best
    }
}

fn cell(point: Vec3) -> (i32, i32, i32) {
    (
        (point.x / CELL).floor() as i32,
        (point.y / CELL).floor() as i32,
        (point.z / CELL).floor() as i32,
    )
}

fn segment_triangle_hit(origin: Vec3, direction: Vec3, triangle: Triangle) -> Option<f32> {
    let ab = triangle.b - triangle.a;
    let ac = triangle.c - triangle.a;
    let p = direction.cross(ac);
    let determinant = ab.dot(p);
    if determinant.abs() < 1e-7 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let offset = origin - triangle.a;
    let u = offset.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = offset.cross(ab);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(ac.dot(q) * inverse)
}

fn vertical_intersection(x: f32, z: f32, triangle: Triangle) -> Option<f32> {
    let v0 = triangle.b - triangle.a;
    let v1 = triangle.c - triangle.a;
    let det = v0.x * v1.z - v1.x * v0.z;
    if det.abs() < 1e-7 {
        return None;
    }
    let dx = x - triangle.a.x;
    let dz = z - triangle.a.z;
    let u = (dx * v1.z - v1.x * dz) / det;
    let v = (v0.x * dz - dx * v0.z) / det;
    if u < -1e-5 || v < -1e-5 || u + v > 1.00001 {
        return None;
    }
    Some(triangle.a.y + u * v0.y + v * v1.y)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn segment_hit_finds_nearest_wall_from_either_side() {
        let world = CollisionWorld::new([
            [
                Vec3::new(0.0, 0.0, -1.0),
                Vec3::new(0.0, 2.0, -1.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
            [
                Vec3::new(0.0, 2.0, -1.0),
                Vec3::new(0.0, 2.0, 1.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
        ]);
        assert_eq!(
            world.first_hit(Vec3::new(-1.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 0.0)),
            Some(1.0)
        );
        assert_eq!(
            world.first_hit(Vec3::new(1.0, 1.0, 0.0), Vec3::new(-1.0, 1.0, 0.0)),
            Some(1.0)
        );
        assert_eq!(
            world.first_hit(Vec3::new(-1.0, 3.0, 0.0), Vec3::new(1.0, 3.0, 0.0)),
            None
        );
    }
    #[test]
    fn collected_entity_can_disable_its_authored_collider() {
        let wall = [
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 2.0, -1.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let mut world = CollisionWorld::with_owned([(wall, 0)], vec!["pickup_1".into()]);
        let from = Vec3::new(-1.0, 0.5, 0.0);
        let to = Vec3::new(1.0, 0.5, 0.0);
        assert_eq!(world.first_hit(from, to), Some(1.0));
        let (distance, owner, vertices) = world.first_hit_details(from, to).unwrap();
        assert_eq!(distance, 1.0);
        assert_eq!(world.owner_name(owner), Some("pickup_1"));
        assert_eq!(vertices, wall);
        assert!(world.set_owner_active("pickup_1", false));
        assert_eq!(world.first_hit(from, to), None);
        assert_eq!(world.first_hit_details(from, to), None);
        assert!(world.set_owner_active("pickup_1", true));
        assert_eq!(world.first_hit(from, to), Some(1.0));
    }
    #[test]
    fn one_entity_id_can_own_multiple_placed_colliders() {
        let a = [
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 2.0, -1.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let b = [
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(1.0, 2.0, -1.0),
            Vec3::new(1.0, 0.0, 1.0),
        ];
        let mut world = CollisionWorld::with_owned(
            [(a, 0), (b, 1)],
            vec!["assembly".into(), "assembly".into()],
        );
        assert_eq!(
            world.nearby_triangle_count(Vec3::new(0.5, 1.0, 0.0), 2.0),
            2
        );
        assert!(world.set_owner_active("assembly", false));
        assert_eq!(
            world.nearby_triangle_count(Vec3::new(0.5, 1.0, 0.0), 2.0),
            0
        );
    }
}
