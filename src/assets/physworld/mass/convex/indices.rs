//! Three-dimensional face indexing and plane groups (140b94be0).
use super::{
    domain::Domain,
    topology::{Edge, Topology},
};
use crate::gameplay::convex_support::faces::reciprocal_sqrt_estimate;

pub(super) struct Indexed {
    /// Arena identities in the original linked-list traversal order.
    pub vertices: Vec<usize>,
    pub faces: Vec<usize>,
    pub planes: Vec<[f32; 4]>,
}

/// The source forms edges from the first vertex, normalizes their cross,
/// and separately retains its length for descending-area seed ordering.
fn plane(points: [[f32; 4]; 3]) -> ([f32; 4], f32) {
    let a: [f32; 3] = std::array::from_fn(|i| points[2][i] - points[0][i]);
    let b: [f32; 3] = std::array::from_fn(|i| points[1][i] - points[0][i]);
    let normal = [
        a[2] * b[1] - b[2] * a[1],
        a[0] * b[2] - b[0] * a[2],
        a[1] * b[0] - b[1] * a[0],
    ];
    let squared = (normal[1] * normal[1] + normal[0] * normal[0]) + normal[2] * normal[2];
    let seed = reciprocal_sqrt_estimate(squared);
    let inverse = if squared <= 0. {
        0.
    } else {
        ((3. - seed * squared * seed) * seed) * 0.5
    };
    let area = inverse * squared;
    let normal = normal.map(|value| {
        if 0. - squared < 0. {
            inverse * value
        } else {
            value
        }
    });
    let distance =
        0. - ((normal[1] * points[0][1] + normal[0] * points[0][0]) + normal[2] * points[0][2]);
    ([normal[0], normal[1], normal[2], distance], area)
}

pub(super) fn build(hull: &mut Topology, domain: &Domain) -> Result<Indexed, String> {
    let mut indexed = Indexed {
        vertices: Vec::new(),
        faces: Vec::new(),
        planes: Vec::new(),
    };
    indexed
        .vertices
        .try_reserve_exact(hull.vertices.len())
        .map_err(|e| format!("allocating indexed hull vertices: {e}"))?;
    indexed
        .faces
        .try_reserve_exact(hull.faces.len())
        .map_err(|e| format!("allocating indexed hull faces: {e}"))?;
    let mut next = hull.first_vertex;
    while let Some(index) = next {
        let vertex = hull.vertices[index]
            .as_mut()
            .ok_or("indexed hull vertex is retired")?;
        vertex.index = indexed.vertices.len();
        indexed.vertices.push(index);
        next = vertex.next;
    }
    let mut planes = Vec::new();
    let mut areas = Vec::new();
    planes
        .try_reserve_exact(hull.faces.len())
        .map_err(|e| format!("allocating hull face planes: {e}"))?;
    areas
        .try_reserve_exact(hull.faces.len())
        .map_err(|e| format!("allocating hull face areas: {e}"))?;
    next = hull.first_face;
    while let Some(index) = next {
        let face = hull.faces[index]
            .as_mut()
            .ok_or("indexed hull face is retired")?;
        face.index = indexed.faces.len();
        face.group = None;
        let mut points = [[0.; 4]; 3];
        for (corner, vertex) in face.vertices.into_iter().enumerate() {
            points[corner] = domain.export(
                hull.vertices[vertex]
                    .as_ref()
                    .ok_or("indexed hull face vertex is retired")?
                    .point
                    .integer,
            );
        }
        let (plane, area) = plane(points);
        planes.push(plane);
        areas.push((index, area));
        indexed.faces.push(index);
        next = face.next;
    }
    crate::sorting::midpoint_by_key(&mut areas, |(_, area)| std::cmp::Reverse(*area));
    indexed
        .planes
        .try_reserve_exact(areas.len())
        .map_err(|e| format!("allocating hull plane groups: {e}"))?;
    let mut pending = Vec::new();
    pending
        .try_reserve_exact(
            areas
                .len()
                .checked_mul(3)
                .ok_or("hull plane stack size overflow")?,
        )
        .map_err(|e| format!("allocating hull plane stack: {e}"))?;
    for (index, _) in areas {
        let face = hull.faces[index]
            .as_ref()
            .ok_or("hull plane seed face is retired")?;
        if face.group.is_some() {
            continue;
        }
        let seed = planes[face.index];
        let group = indexed.planes.len();
        indexed.planes.push(seed);
        pending.clear();
        for adjacent in face.adjacent {
            pending.push(adjacent.ok_or("hull plane seed has an open edge")?);
        }
        hull.faces[index].as_mut().unwrap().group = Some(group);
        while let Some(Edge { face: index, edge }) = pending.pop() {
            let face = hull.faces[index]
                .as_ref()
                .ok_or("hull plane neighbor face is retired")?;
            if face.group.is_some() {
                continue;
            }
            let plane = planes[face.index];
            let squared = (plane[1] * plane[1] + plane[0] * plane[0]) + plane[2] * plane[2];
            let invalid = plane[..3].iter().any(|value| value.is_nan())
                || f32::from_bits(0x38d1_b717) <= (squared - 1.).abs();
            let dot = (seed[1] * plane[1] + seed[0] * plane[0]) + seed[2] * plane[2];
            // Default BuildInfo+4 from 140b88010, not a guessed angle epsilon.
            if invalid || f32::from_bits(0x3f7f_ff58) <= dot {
                for next in [(edge + 1) % 3, (edge + 2) % 3] {
                    pending.push(face.adjacent[next].ok_or("hull plane group has an open edge")?);
                }
                hull.faces[index].as_mut().unwrap().group = Some(group);
            }
        }
    }
    Ok(indexed)
}
