//! Triangle index over the level's static collision decoded from the game's
//! authored Havok physics world, with owner activation for collected pickups.
//! Coordinates are Bevy Y-up metres; the player queries it through
//! `player::character_proxy::PlayerWorld`.
use super::collision_filter::{filter_info, layer, QueryFilter};
use crate::entities::physics_material::PhysicsMaterial;
use bevy_math::Vec3;
use std::{collections::HashMap, sync::Arc};

/// `material` value of a triangle whose physics material is not resolved.
pub const NO_MATERIAL: u32 = u32::MAX;

const CELL: f32 = 4.0;

#[derive(Clone, Copy)]
struct Triangle {
    a: Vec3,
    b: Vec3,
    c: Vec3,
    min: Vec3,
    max: Vec3,
    owner: u32,
    material: u32,
    /// The arkCollisionFilterInfo word of the triangle's body and shape key.
    filter: u32,
}

pub struct CollisionWorld {
    triangles: Vec<Triangle>,
    index: TriangleIndex,
    /// Component-owned collision, updated independently of the immutable map.
    component_triangles: Vec<Option<Triangle>>,
    component_generations: Vec<u32>,
    component_owners: Vec<Option<u32>>,
    component_index: ExpandedIndex,
    free_component_boxes: Vec<(usize, u32)>,
    owner_ids: HashMap<Arc<str>, Vec<u32>>,
    owner_names: Vec<Option<Arc<str>>>,
    owner_active: Vec<bool>,
    materials: Vec<PhysicsMaterial>,
}

/// Host broad phase replacing Havok's compressed-mesh tree. Once built, its
/// lists are immutable: pack them without changing triangle/query order.
enum TriangleIndex {
    Packed {
        cells: hashbrown::HashMap<(i32, i32, i32), std::ops::Range<usize>>,
        indices: Box<[u32]>,
        large: Box<[u32]>,
    },
    /// Same-build memory/CPU comparison; never selected during normal play.
    Expanded(ExpandedIndex),
}

/// A component-owned fake interaction body. Handles are valid until removal.
#[derive(Clone, Copy, Debug)]
pub struct ComponentBox { first: usize, owner: u32, generation: u32 }

#[derive(Default)]
struct ExpandedIndex {
    /// foldhash (hashbrown's default): looked up per cell of every query.
    cells: hashbrown::HashMap<(i32, i32, i32), Vec<usize>>,
    large: Vec<usize>,
}

impl CollisionWorld {
    fn triangle(&self, index: usize) -> Option<Triangle> {
        if index < self.triangles.len() { self.triangles.get(index).copied() }
        else { self.component_triangles.get(index - self.triangles.len()).copied().flatten() }
    }

    /// 14077af40 creates the fake interaction box; 14077be20 publishes its
    /// transformed pose and 14077b610 removes it on component stop.
    pub fn add_interaction_box(&mut self, entity: &str, corners: [Vec3; 8]) -> ComponentBox {
        let (first, owner) = self.free_component_boxes.pop().unwrap_or_else(|| {
            let first = self.component_triangles.len();
            self.component_triangles.resize(first + 12, None);
            self.component_generations.push(0);
            self.component_owners.push(None);
            let owner = self.owner_active.len() as u32;
            self.owner_active.push(false);
            self.owner_names.push(None);
            (first, owner)
        });
        self.owner_active[owner as usize] = true;
        self.component_owners[first / 12] = Some(owner);
        let name = self.owner_ids.get_key_value(entity).map(|(name, _)| name.clone())
            .unwrap_or_else(|| Arc::<str>::from(entity));
        self.owner_names[owner as usize] = Some(name.clone());
        self.owner_ids.entry(name).or_default().push(owner);
        let handle = ComponentBox { first, owner, generation: self.component_generations[first / 12] };
        self.update_interaction_box(handle, corners);
        handle
    }

    pub fn update_interaction_box(&mut self, handle: ComponentBox, corners: [Vec3; 8]) {
        if self.component_generations.get(handle.first / 12).copied() != Some(handle.generation)
            || self.component_owners.get(handle.first / 12).copied().flatten() != Some(handle.owner) { return; }
        const FACES: [[usize; 3]; 12] = [
            [0, 3, 1], [0, 2, 3], [4, 5, 7], [4, 7, 6],
            [0, 1, 5], [0, 5, 4], [2, 6, 7], [2, 7, 3],
            [0, 4, 6], [0, 6, 2], [1, 3, 7], [1, 7, 5],
        ];
        for (face, points) in FACES.into_iter().enumerate() {
            let local = handle.first + face;
            let index = self.triangles.len() + local;
            if let Some(old) = self.component_triangles[local] {
                unindex_component(&mut self.component_index, index, old.min, old.max);
            }
            let [a, b, c] = points.map(|i| corners[i]);
            let triangle = Triangle {
                a, b, c, min: a.min(b).min(c), max: a.max(b).max(c), owner: handle.owner,
                material: NO_MATERIAL,
                filter: crate::gameplay::collision_filter::filter_info(crate::gameplay::collision_filter::layer::INTERACTABLE) & !0x800,
            };
            self.component_triangles[local] = Some(triangle);
            index_component(&mut self.component_index, index, triangle.min, triangle.max);
        }
    }

    pub fn remove_interaction_box(&mut self, handle: ComponentBox) {
        if self.component_generations.get(handle.first / 12).copied() != Some(handle.generation)
            || self.component_owners.get(handle.first / 12).copied().flatten() != Some(handle.owner) { return; }
        for local in handle.first..handle.first + 12 {
            if let Some(old) = self.component_triangles[local].take() {
                unindex_component(&mut self.component_index, self.triangles.len() + local, old.min, old.max);
            }
        }
        self.owner_active[handle.owner as usize] = false;
        self.component_owners[handle.first / 12] = None;
        self.component_generations[handle.first / 12] = handle.generation.wrapping_add(1);
        if let Some(name) = self.owner_names[handle.owner as usize].take() {
            if let Some(indices) = self.owner_ids.get_mut(name.as_ref()) {
                indices.retain(|&owner| owner != handle.owner);
                if indices.is_empty() { self.owner_ids.remove(name.as_ref()); }
            }
        }
        self.free_component_boxes.push((handle.first, handle.owner));
    }
    pub fn new(source: impl IntoIterator<Item = [Vec3; 3]>) -> Self {
        Self::with_owned(
            source.into_iter().map(|triangle| (triangle, u32::MAX)),
            Vec::new(),
        )
    }

    /// Synthetic geometry (probes, tests) takes the WORLD_LOW layer's word.
    pub fn with_owned(
        source: impl IntoIterator<Item = ([Vec3; 3], u32)>,
        owners: Vec<String>,
    ) -> Self {
        Self::with_materials(
            source
                .into_iter()
                .map(|(triangle, owner)| (triangle, owner, NO_MATERIAL, filter_info(layer::WORLD_LOW))),
            owners,
            Vec::new(),
        )
    }

    /// Triangles carry an owner, an index into `materials` (or
    /// `NO_MATERIAL`; see `physics_material`) and their filter word.
    pub fn with_materials(
        source: impl IntoIterator<Item = ([Vec3; 3], u32, u32, u32)>,
        owners: Vec<String>,
        materials: Vec<PhysicsMaterial>,
    ) -> Self {
        let mut owner_ids = HashMap::<Arc<str>, Vec<u32>>::new();
        let mut owner_names = Vec::with_capacity(owners.len());
        for (index, id) in owners.into_iter().enumerate() {
            let name = owner_ids.get_key_value(id.as_str()).map(|(name, _)| name.clone())
                .unwrap_or_else(|| Arc::<str>::from(id));
            owner_ids.entry(name.clone()).or_default().push(index as u32);
            owner_names.push(Some(name));
        }
        let owner_count = owner_names.len();
        let mut build_index = ExpandedIndex::default();
        let mut world = Self {
            triangles: Vec::new(),
            index: TriangleIndex::Expanded(ExpandedIndex::default()),
            component_triangles: Vec::new(),
            component_generations: Vec::new(),
            component_owners: Vec::new(),
            component_index: ExpandedIndex::default(),
            free_component_boxes: Vec::new(),
            owner_ids,
            owner_names,
            owner_active: vec![true; owner_count],
            materials,
        };
        for ([a, b, c], owner, material, filter) in source {
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
                material: if (material as usize) < world.materials.len() {
                    material
                } else {
                    NO_MATERIAL
                },
                filter,
            });
            let low = cell(min);
            let high = cell(max);
            let cells = (high.0 - low.0 + 1) as i64
                * (high.1 - low.1 + 1) as i64
                * (high.2 - low.2 + 1) as i64;
            if cells > 4096 {
                // Triangle IDs are also the public u32 physics shape keys.
                build_index.large.push(index);
                continue;
            }
            for x in low.0..=high.0 {
                for y in low.1..=high.1 {
                    for z in low.2..=high.2 {
                        build_index.cells.entry((x, y, z)).or_default().push(index);
                    }
                }
            }
        }
        let expanded_bytes = world.triangles.capacity() * std::mem::size_of::<Triangle>()
            + build_index.large.capacity() * std::mem::size_of::<usize>()
            + build_index.cells.values().map(|v| v.capacity() * std::mem::size_of::<usize>()).sum::<usize>()
            + build_index.cells.capacity() * std::mem::size_of::<((i32, i32, i32), Vec<usize>)>();
        if std::env::var("DH2_DEBUG_COLLISION_EXPANDED").as_deref() == Ok("1") {
            world.index = TriangleIndex::Expanded(build_index);
        } else {
            let count: usize = build_index.cells.values().map(Vec::len).sum();
            let mut indices = Vec::with_capacity(count);
            let mut cells = hashbrown::HashMap::with_capacity(build_index.cells.len());
            for (cell, list) in build_index.cells {
                let start = indices.len();
                indices.extend(list.into_iter().map(|id| u32::try_from(id).expect("collision shape key exceeds u32")));
                cells.insert(cell, start..indices.len());
            }
            let large = build_index.large.into_iter().map(|id| u32::try_from(id).expect("collision shape key exceeds u32")).collect();
            world.triangles.shrink_to_fit();
            world.index = TriangleIndex::Packed { cells, indices: indices.into_boxed_slice(), large };
        }
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            let bytes = match &world.index {
                TriangleIndex::Expanded(_) => expanded_bytes,
                TriangleIndex::Packed { cells, indices, large } => {
                    world.triangles.capacity() * std::mem::size_of::<Triangle>()
                        + (indices.len() + large.len()) * std::mem::size_of::<u32>()
                        + cells.capacity() * std::mem::size_of::<((i32, i32, i32), std::ops::Range<usize>)>()
                }
            };
            println!("MEMORY collision index: bytes={bytes} expanded_bytes={expanded_bytes} triangles={} cells={}", world.triangles.len(), match &world.index {
                TriangleIndex::Packed { cells, .. } => cells.len(), TriangleIndex::Expanded(index) => index.cells.len(),
            });
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
        match &self.index {
            TriangleIndex::Packed { large, .. } => large.len(),
            TriangleIndex::Expanded(index) => index.large.len(),
        }
    }

    pub fn nearby_triangle_count(&self, point: Vec3, radius: f32) -> usize {
        self.candidates(point - Vec3::splat(radius), point + Vec3::splat(radius), QueryFilter::ALL)
            .len()
    }

    /// First hit distance along a finite segment, testing both triangle sides
    /// of every layer.
    pub fn first_hit(&self, start: Vec3, end: Vec3) -> Option<f32> {
        self.first_hit_details(start, end, QueryFilter::ALL).map(|hit| hit.0)
    }

    /// A triangle's filter word (the key `triangles_in` reports).
    pub fn collision_filter(&self, triangle: u32) -> Option<u32> {
        self.triangle(triangle as usize).map(|t| t.filter)
    }

    /// The arkPhysicsMaterial the game resolves for a triangle (FUN_1400922d0
    /// + FUN_140092c10), keyed by the index `triangles_in` and
    /// `first_hit_details` report. `None` where the material is not decoded
    /// (placed-model colliders, unresolved codec paths).
    pub fn physics_material(&self, triangle: u32) -> Option<&PhysicsMaterial> {
        let material = self.triangle(triangle as usize)?.material;
        self.materials.get(material as usize)
    }

    /// Nearest hit used by first_hit: (distance, triangle index, owner,
    /// vertices). The index is the key `triangles_in` reports.
    pub fn first_hit_details(&self, start: Vec3, end: Vec3, filter: QueryFilter) -> Option<(f32, u32, u32, [Vec3; 3])> {
        let segment = end - start;
        let length = segment.length();
        if length <= 1e-5 {
            return None;
        }
        let direction = segment / length;
        self.candidates(start.min(end), start.max(end), filter)
            .into_iter()
            .filter_map(|index| {
                let triangle = self.triangle(index)?;
                let distance = segment_triangle_hit(start, direction, triangle)?;
                (distance > 0.001 && distance < length).then_some((
                    distance,
                    index as u32,
                    triangle.owner,
                    [triangle.a, triangle.b, triangle.c],
                ))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    pub fn owner_name(&self, owner: u32) -> Option<&str> {
        self.owner_names.get(owner as usize).and_then(Option::as_deref)
    }

    /// Active triangles `filter` accepts whose bounds overlap `min..max`:
    /// (index, vertices, owner), owner `u32::MAX` for level geometry.
    pub fn triangles_in(&self, min: Vec3, max: Vec3, filter: QueryFilter) -> Vec<(u32, [Vec3; 3], u32)> {
        self.candidates(min, max, filter)
            .into_iter()
            .map(|index| {
                let t = self.triangle(index).expect("indexed collision triangle");
                (index as u32, [t.a, t.b, t.c], t.owner)
            })
            .collect()
    }

    /// Reuse the broad-phase and triangle buffers across repeated sensor casts.
    pub fn triangles_in_reuse(&self, min: Vec3, max: Vec3, filter: QueryFilter,
        candidates: &mut Vec<usize>, triangles: &mut Vec<(u32, [Vec3; 3], u32)>) {
        self.candidates_into(min, max, filter, candidates);
        triangles.clear();
        triangles.extend(candidates.iter().map(|&index| {
            let t = self.triangle(index).expect("indexed collision triangle");
            (index as u32, [t.a, t.b, t.c], t.owner)
        }));
    }

    /// Matching triangle indices in ascending order, so results never depend
    /// on hash order (first_hit_details keeps the first of equal distances).
    fn candidates(&self, min: Vec3, max: Vec3, filter: QueryFilter) -> Vec<usize> {
        let mut found = Vec::new();
        self.candidates_into(min, max, filter, &mut found);
        found
    }

    fn candidates_into(&self, min: Vec3, max: Vec3, filter: QueryFilter, found: &mut Vec<usize>) {
        static TIMING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let started = (*TIMING.get_or_init(|| std::env::var("DH2_DEBUG_COLLISION_TIMING").as_deref() == Ok("1")))
            .then(std::time::Instant::now);
        let low = cell(min);
        let high = cell(max);
        found.clear();
        match &self.index {
            TriangleIndex::Packed { cells, indices, large } => {
                found.extend(large.iter().map(|&id| id as usize));
                for x in low.0..=high.0 {
                    for y in low.1..=high.1 {
                        for z in low.2..=high.2 {
                            if let Some(range) = cells.get(&(x, y, z)) {
                                found.extend(indices[range.clone()].iter().map(|&id| id as usize));
                            }
                        }
                    }
                }
            }
            TriangleIndex::Expanded(index) => {
                found.extend(index.large.iter().copied());
                for x in low.0..=high.0 {
                    for y in low.1..=high.1 {
                        for z in low.2..=high.2 {
                            if let Some(indices) = index.cells.get(&(x, y, z)) {
                                found.extend(indices.iter().copied());
                            }
                        }
                    }
                }
            }
        }
        // Fake interactable bodies have their own mutable broad phase. Most
        // player/world queries reject this layer and avoid touching it.
        if filter.accepts(crate::gameplay::collision_filter::filter_info(crate::gameplay::collision_filter::layer::INTERACTABLE) & !0x800) {
            found.extend(self.component_index.large.iter().copied());
            for x in low.0..=high.0 {
                for y in low.1..=high.1 {
                    for z in low.2..=high.2 {
                        if let Some(indices) = self.component_index.cells.get(&(x, y, z)) {
                            found.extend(indices.iter().copied());
                        }
                    }
                }
            }
        }
        found.retain(|&index| {
            let Some(triangle) = self.triangle(index) else { return false };
            (triangle.owner == u32::MAX || self.owner_active[triangle.owner as usize])
                && filter.accepts(triangle.filter)
                && (0..3).all(|axis| {
                    triangle.min[axis] <= max[axis] && triangle.max[axis] >= min[axis]
                })
        });
        found.sort_unstable();
        found.dedup();
        if let Some(started) = started {
            thread_local! {
                static COST: std::cell::Cell<(u64, u128, u64)> = const { std::cell::Cell::new((0, 0, 0)) };
            }
            let elapsed = started.elapsed().as_nanos();
            COST.with(|cost| {
                let (calls, ns, hits) = cost.get();
                let next = (calls + 1, ns + elapsed, hits + found.len() as u64);
                cost.set(next);
                if next.0 % 512 == 0 {
                    println!("PERF collision candidates: calls={} ns={} candidates={}", next.0, next.1, next.2);
                }
            });
        }
    }

    /// Highest walkable surface below/within one step above the proposed feet.
    pub fn floor_at(&self, feet: Vec3, step_up: f32, down: f32) -> Option<f32> {
        let min = feet + Vec3::new(-0.02, -down, -0.02);
        let max = feet + Vec3::new(0.02, step_up, 0.02);
        let mut best: Option<f32> = None;
        for index in self.candidates(min, max, QueryFilter::ALL) {
            let triangle = self.triangle(index).expect("indexed collision triangle");
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

fn index_component(index: &mut ExpandedIndex, id: usize, min: Vec3, max: Vec3) {
    let low = cell(min);
    let high = cell(max);
    let count = (high.0 - low.0 + 1) as i64 * (high.1 - low.1 + 1) as i64 * (high.2 - low.2 + 1) as i64;
    if count > 4096 { index.large.push(id); return; }
    for x in low.0..=high.0 {
        for y in low.1..=high.1 {
            for z in low.2..=high.2 { index.cells.entry((x, y, z)).or_default().push(id); }
        }
    }
}

fn unindex_component(index: &mut ExpandedIndex, id: usize, min: Vec3, max: Vec3) {
    let low = cell(min);
    let high = cell(max);
    let count = (high.0 - low.0 + 1) as i64 * (high.1 - low.1 + 1) as i64 * (high.2 - low.2 + 1) as i64;
    if count > 4096 { index.large.retain(|&v| v != id); return; }
    for x in low.0..=high.0 {
        for y in low.1..=high.1 {
            for z in low.2..=high.2 {
                let key = (x, y, z);
                if let Some(values) = index.cells.get_mut(&key) {
                    values.retain(|&v| v != id);
                    if values.is_empty() { index.cells.remove(&key); }
                }
            }
        }
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
    fn hit_index_maps_to_the_triangle_physics_material() {
        let wall = |x: f32| {
            [
                Vec3::new(x, 0.0, -1.0),
                Vec3::new(x, 2.0, -1.0),
                Vec3::new(x, 0.0, 1.0),
            ]
        };
        let mut grid =
            PhysicsMaterial::constructor_default("contactsystem/env.metal.grid.mantleoff");
        grid.flags = 0;
        let world = CollisionWorld::with_materials(
            [
                (wall(1.0), u32::MAX, 1, 0),
                (wall(2.0), u32::MAX, 0, 0),
                (wall(3.0), u32::MAX, NO_MATERIAL, 0),
                (wall(4.0), u32::MAX, 7, 0),
            ],
            Vec::new(),
            vec![
                PhysicsMaterial::constructor_default("contactsystem/env.stone"),
                grid,
            ],
        );
        let (distance, index, _, _) = world
            .first_hit_details(Vec3::new(0.0, 0.5, 0.0), Vec3::new(5.0, 0.5, 0.0), QueryFilter::ALL)
            .unwrap();
        assert_eq!((distance, index), (1.0, 0));
        let first = world.physics_material(index).unwrap();
        assert!(!first.allow_mantle() && !first.allow_vault());
        let keys: Vec<u32> = world
            .triangles_in(Vec3::new(1.5, 0.0, -1.0), Vec3::new(2.5, 2.0, 1.0), QueryFilter::ALL)
            .into_iter()
            .map(|(key, _, _)| key)
            .collect();
        assert_eq!(keys, [1]);
        assert!(world.physics_material(1).unwrap().allow_mantle());
        assert_eq!(world.physics_material(2), None);
        // Out-of-table material indices are stored as unresolved.
        assert_eq!(world.physics_material(3), None);
        assert_eq!(world.physics_material(99), None);
    }
    #[test]
    fn queries_see_only_the_triangles_their_filter_accepts() {
        let wall = |x: f32| [Vec3::new(x, 0.0, -1.0), Vec3::new(x, 2.0, -1.0), Vec3::new(x, 0.0, 1.0)];
        // eotw_p words: WORLD_HIGH detail (projectiles only) in front of
        // WORLD_LOW_HIGH collision that includes PLAYER.
        let world = CollisionWorld::with_materials(
            [(wall(1.0), u32::MAX, NO_MATERIAL, 0x2010002), (wall(2.0), u32::MAX, NO_MATERIAL, 0x1802c01)],
            Vec::new(),
            Vec::new(),
        );
        let (from, to) = (Vec3::new(0.0, 0.5, 0.0), Vec3::new(5.0, 0.5, 0.0));
        let player = QueryFilter::World(filter_info(layer::PLAYER) & 0xfbffffff);
        assert_eq!(world.first_hit_details(from, to, player).map(|h| h.1), Some(1));
        assert_eq!(world.first_hit_details(from, to, QueryFilter::ALL).map(|h| h.1), Some(0));
        let ground = QueryFilter::layers(0x802b, 1 << layer::PLAYER);
        assert_eq!(world.triangles_in(Vec3::new(0.0, 0.0, -1.0), Vec3::new(3.0, 2.0, 1.0), ground).len(), 1);
        assert_eq!(world.collision_filter(0), Some(0x2010002));
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
        let (distance, _, owner, vertices) = world.first_hit_details(from, to, QueryFilter::ALL).unwrap();
        assert_eq!(distance, 1.0);
        assert_eq!(world.owner_name(owner), Some("pickup_1"));
        assert_eq!(vertices, wall);
        assert!(world.set_owner_active("pickup_1", false));
        assert_eq!(world.first_hit(from, to), None);
        assert_eq!(world.first_hit_details(from, to, QueryFilter::ALL), None);
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
