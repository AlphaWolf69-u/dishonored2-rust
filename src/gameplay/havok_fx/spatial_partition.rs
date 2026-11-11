//! hkfxEffects::SpatialPartition (begin 140fd72f0, end 140fd75f0): the
//! bodies' bounding spheres and their spatially sorted list (+0x50 +0x5a0),
//! whose order PhysicsCollide's cache sharing walks.
//!
//! The list order is the leaf order hkcdSimdTreeUtils::Build (140c1e380,
//! serial 140c1fb80) leaves after a full rebuild (the partition's build
//! context +0x580 has mode 0, chunk_000668:5391); its leaf callback
//! (140fd96b0) then stores each body's list position at +0x2a8.
use crate::float::{add, divide, multiply, subtract};

/// One list entry: the sphere centre and the body index bits
/// (body | 0x3f000000) in the fourth lane.
pub type Leaf = [f32; 4];

const FAR: f32 = f32::from_bits(0x7f7f_ffee);

fn minps(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|i| if a[i] < b[i] { a[i] } else { b[i] })
}

fn maxps(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|i| if a[i] > b[i] { a[i] } else { b[i] })
}

fn bounds(leaves: &[Leaf]) -> ([f32; 4], [f32; 4]) {
    leaves.iter().fold(([FAR; 4], [-FAR; 4]), |(low, high), leaf| (minps(low, *leaf), maxps(high, *leaf)))
}

/// The largest extent's axis; ties go to z, then y (141e89850).
fn split_axis(low: [f32; 4], high: [f32; 4]) -> usize {
    let e: [f32; 3] = std::array::from_fn(|i| subtract(high[i], low[i]));
    let xy = if e[1] > e[0] { e[1] } else { e[0] };
    let largest = if e[2] > xy { e[2] } else { xy };
    let mask = (0..3).fold(0, |mask, i| mask | (usize::from(largest <= e[i]) << i));
    [0, 0, 1, 1, 2, 2, 2, 2][mask]
}

/// The split loop of 140c1fb80 (140c204c0..140c206b5): elements below
/// `m` (comiss ja) end up first; returns their count.
fn partition(leaves: &mut [Leaf], axis: usize, m: f32) -> usize {
    if leaves.is_empty() {
        return 0;
    }
    let (mut i, mut j) = (0, leaves.len() - 1);
    while i <= j && m > leaves[i][axis] {
        i += 1;
    }
    if i < j {
        while j > i && !(m > leaves[j][axis]) {
            j -= 1;
        }
        while i < j {
            leaves.swap(i, j);
            i += 1;
            while i < j && m > leaves[i][axis] {
                i += 1;
            }
            j -= 1;
            while j > i && !(m > leaves[j][axis]) {
                j -= 1;
            }
        }
    }
    i
}

/// Reorders `leaves` as the mode-0 build does.
pub fn build_order(leaves: &mut [Leaf]) {
    let n = leaves.len();
    if n <= 4 {
        return;
    }
    if n <= 0x20 {
        let (low, high) = bounds(leaves);
        let axis = split_axis(low, high);
        // Insertion sort; comiss x, a[j-1] / jae stops the shift.
        for k in 1..n {
            let x = leaves[k];
            let mut j = k;
            while j > 0 && !(x[axis] >= leaves[j - 1][axis]) {
                leaves[j] = leaves[j - 1];
                j -= 1;
            }
            leaves[j] = x;
        }
        return;
    }
    let mut sum = [0f32; 4];
    for leaf in leaves.iter() {
        sum = std::array::from_fn(|i| add(sum[i], leaf[i]));
    }
    let (low, high) = bounds(leaves);
    let inverse = divide(1., n as f32);
    let axis = split_axis(low, high);
    let mean = multiply(inverse, sum[axis]);
    let lower = multiply(add(mean, low[axis]), 0.5);
    let upper = multiply(add(mean, high[axis]), 0.5);
    let mut left = partition(leaves, axis, mean);
    if left < 2 || n - left < 2 {
        left = n >> 1;
    }
    let (head, tail) = leaves.split_at_mut(left);
    let mut a = partition(head, axis, lower);
    if a < 1 || left - a < 1 {
        a = left >> 1;
    }
    let mut b = partition(tail, axis, upper);
    if b < 1 || tail.len() - b < 1 {
        b = tail.len() >> 1;
    }
    let (first, second) = head.split_at_mut(a);
    let (third, fourth) = tail.split_at_mut(b);
    for child in [first, second, third, fourth] {
        build_order(child);
    }
}

/// +0x50 +0x560 / +0x570: a new body's current and previous sphere, which
/// differ so that it starts in the list.
const NEW_SPHERE: [f32; 4] = [0., 0., 0., -f32::from_bits(0x5f7f_fff0)];
const NEW_PREVIOUS: [f32; 4] = [0., 0., 0., f32::from_bits(0x5f7f_fff0)];
/// 140fd8d00: at most this many bodies move between the list and the
/// static tree per step (+0x550).
const TRANSFERS: usize = 0x200;
const NOT_LISTED: i32 = 0x7fff_ffff;

fn leaf_body(leaf: &Leaf) -> usize {
    (leaf[3].to_bits() & 0xc0ff_ffff) as usize
}

fn leaf_of(body: usize) -> Leaf {
    [0., 0., 0., f32::from_bits(body as u32 | 0x3f00_0000)]
}

/// The partition with its constructor defaults (140fd6a00: +0x29 tracks
/// changes, +0x30 velocity factor 1, +0x34 extra radius 0, +0x38 scale 1,
/// +0x3c limit FLT_MAX, +0x2a and +0x2b off). The static tree itself only
/// matters to self contacts; here it is the membership `in_tree`.
pub struct SpatialPartition {
    /// +0x58 / +0x68 per body.
    spheres: Vec<[f32; 4]>,
    previous: Vec<[f32; 4]>,
    in_tree: Vec<bool>,
    /// +0x50 +0x2a8: each body's list position, or 0x7fffffff in the tree.
    pub index: Vec<i32>,
    /// +0x50 +0x5a0.
    pub list: Vec<Leaf>,
}

impl SpatialPartition {
    pub fn new(capacity: usize) -> Self {
        Self {
            spheres: vec![NEW_SPHERE; capacity],
            previous: vec![NEW_PREVIOUS; capacity],
            in_tree: vec![false; capacity],
            index: vec![NOT_LISTED; capacity],
            list: Vec::new(),
        }
    }

    /// Slot 8 (140fd7020): new bodies join the end of the list.
    pub fn allocate(&mut self, bodies: &[i32]) {
        for &body in bodies {
            let body = body as usize;
            self.spheres[body] = NEW_SPHERE;
            self.previous[body] = NEW_PREVIOUS;
            self.in_tree[body] = false;
            self.index[body] = self.list.len() as i32;
            self.list.push(leaf_of(body));
        }
    }

    /// Slot 9 (140fd7180): a listed body is swap-removed, a tree body
    /// leaves the tree.
    pub fn release(&mut self, bodies: &[i32]) {
        for &body in bodies {
            let body = body as usize;
            self.spheres[body] = NEW_SPHERE;
            self.previous[body] = NEW_PREVIOUS;
            if self.in_tree[body] {
                self.in_tree[body] = false;
                continue;
            }
            let hole = self.index[body];
            if hole == NOT_LISTED {
                continue;
            }
            let last = self.list.len() - 1;
            self.index[leaf_body(&self.list[last])] = hole;
            self.index[body] = NOT_LISTED;
            self.list.swap_remove(hole as usize);
        }
    }

    /// The end of a step (140fd75f0 -> 140fd8d00, mode 0 rebuild):
    /// spheres of `bodies` (in allocated order) from `linear`, transfers
    /// out of and into the static tree, then the list's build order.
    pub fn end(&mut self, dt: f32, pipeline_radius: f32, bodies: &[i32], linear: &[[[f32; 4]; 2]]) {
        use crate::gameplay::constraint_solver::inverse_length;
        if bodies.is_empty() {
            return;
        }
        std::mem::swap(&mut self.previous, &mut self.spheres);
        self.spheres.clone_from(&self.previous);
        // 140fd86b0: r = min((|v| x 1 x dt + scale x (radius + 0)) x 1, 0x5f7ffff0).
        let radius = add(pipeline_radius, 0.);
        let mut moved = Vec::new();
        for &body in bodies {
            let index = body as usize;
            let [p, v] = linear[index];
            let squared = add(add(multiply(v[1], v[1]), multiply(v[0], v[0])), multiply(v[2], v[2]));
            let speed = multiply(inverse_length(squared), squared);
            let reach = multiply(add(multiply(multiply(speed, 1.), dt), multiply(v[3], radius)), 1.);
            let limit = f32::from_bits(0x5f7f_fff0);
            let r = if reach < limit { reach } else { limit };
            let sphere = [p[0], p[1], p[2], r];
            self.spheres[index] = sphere;
            if sphere != self.previous[index] && self.in_tree[index] {
                moved.push(index);
            }
        }
        // Transfer: at most 0x200 tree bodies back to the list, by index.
        moved.truncate(TRANSFERS);
        moved.sort_unstable();
        let mut transfers = moved.len();
        for &body in &moved {
            self.in_tree[body] = false;
            self.list.push(leaf_of(body));
        }
        // Prepare: unchanged bodies go to the tree while transfers last.
        let list = std::mem::take(&mut self.list);
        for leaf in list {
            let body = leaf_body(&leaf);
            if transfers < TRANSFERS && self.spheres[body] == self.previous[body] {
                self.in_tree[body] = true;
                self.index[body] = NOT_LISTED;
                transfers += 1;
            } else {
                let s = self.spheres[body];
                self.list.push([s[0], s[1], s[2], leaf[3]]);
            }
        }
        build_order(&mut self.list);
        for (position, leaf) in self.list.iter().enumerate() {
            self.index[leaf_body(leaf)] = position as i32;
        }
    }
}
