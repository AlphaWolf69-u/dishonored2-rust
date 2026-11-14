//! Uncached convex mass dispatch (140ca38b0 -> 140ce1360).
//! Construction topology and rounded-hull samples are transient; only the
//! principal frame is returned to the existing body/motion owner.
mod domain;
mod topology;
mod indices;
mod properties;
use domain::Domain;
use topology::{Topology, Point};

struct Spatial { topology: Topology, domain: Domain, indexed: indices::Indexed }
impl Spatial {
    fn acquire(points: &[[f32; 4]]) -> Result<Option<Self>, String> {
        let domain = Domain::from_points(points)?;
        let mut working = Vec::new();
        working.try_reserve_exact(points.len()).map_err(|error| format!("convex mass input: {error}"))?;
        working.extend(points.iter().map(|&source| Point { integer: domain.quantize(source), source }));
        let Some(mut topology) = Topology::spatial(&mut working)? else { return Ok(None); };
        let indexed = indices::build(&mut topology, &domain)?;
        Ok(Some(Self { topology, domain, indexed }))
    }
    fn expanded_mass_points(&mut self, radius: f32) -> Result<Vec<[f32; 4]>, String> {
        // 140b8d4c0 selects the published mass centre only when the hull's
        // mass-ready bit is set. ce1360's default BuildInfo leaves it clear.
        let centre = properties::mean(&self.topology, None)?;
        let mut points = Vec::new();
        points.try_reserve(self.indexed.vertices.len().saturating_mul(7))
            .map_err(|error| format!("mass hull expansion: {error}"))?;
        let outside = |point| contains_candidate(&self.topology, &self.domain,
            Some(&self.indexed.planes), point, false, 0.);
        let source = |index: usize| self.topology.vertices.get(index).and_then(Option::as_ref)
            .map(|vertex| vertex.point.source).ok_or("mass hull vertex is absent");
        let unit_offset = |vector: [f32; 3], vertex: bool| -> Option<[f32; 3]> {
            let squared = (vector[1] * vector[1] + vector[0] * vector[0]) + vector[2] * vector[2];
            if !(f32::EPSILON < squared) { return None; }
            let seed = crate::gameplay::convex_support::faces::reciprocal_sqrt_estimate(squared);
            // The edge and vertex loops use different multiplication order.
            let product = if vertex { squared * seed * seed } else { seed * squared * seed };
            let inverse = ((3. - product) * seed) * 0.5;
            Some(vector.map(|value| (inverse * value) * radius))
        };
        for &index in &self.indexed.faces {
            let face = self.topology.faces[index].as_ref().ok_or("mass hull face is absent")?;
            let group = face.group.ok_or("mass hull face has no plane")?;
            let plane = *self.indexed.planes.get(group).ok_or("mass hull plane is absent")?;
            let vertices = [source(face.vertices[0])?, source(face.vertices[1])?, source(face.vertices[2])?];
            let a = vertices[1];
            let b = vertices[0];
            let c = vertices[2];
            let edge1: [f32; 3] = std::array::from_fn(|axis| a[axis] - b[axis]);
            let edge2: [f32; 3] = std::array::from_fn(|axis| c[axis] - b[axis]);
            let normal = [edge2[2] * edge1[1] - edge1[2] * edge2[1],
                edge2[0] * edge1[2] - edge1[0] * edge2[2],
                edge2[1] * edge1[0] - edge1[1] * edge2[0]];
            let maximum = normal.into_iter().map(f32::abs).fold(0., f32::max);
            if f32::EPSILON < maximum {
                for vertex in vertices {
                    points.push([vertex[0] + plane[0] * radius, vertex[1] + plane[1] * radius,
                        vertex[2] + plane[2] * radius, 0.]);
                }
            }
            for edge in 0..3 {
                let adjacent = face.adjacent[edge].ok_or("mass hull edge is open")?;
                let neighbor = self.topology.faces[adjacent.face].as_ref().ok_or("mass hull neighbor is absent")?;
                let other = neighbor.group.ok_or("mass hull neighbor has no plane")?;
                if group >= other { continue; }
                let other_plane = self.indexed.planes.get(other).ok_or("mass hull neighbor plane is absent")?;
                let dot = (plane[1] * other_plane[1] + plane[0] * other_plane[0]) + plane[2] * other_plane[2];
                // a9f7e0(pi/4), with the source's sign XOR.
                if !(dot < f32::from_bits(0x3f35_04f3)) { continue; }
                let first = vertices[edge];
                let second = vertices[(edge + 1) % 3];
                let midpoint = std::array::from_fn(|axis| (first[axis] + second[axis]) * 0.5 - centre[axis]);
                if let Some(offset) = unit_offset(midpoint, false) {
                    for vertex in [first, second] {
                        let candidate = [vertex[0] + offset[0], vertex[1] + offset[1], vertex[2] + offset[2], 0.];
                        if !outside(candidate)? { points.push(candidate); }
                    }
                }
            }
        }
        for &index in &self.indexed.vertices {
            let vertex = source(index)?;
            let direction = std::array::from_fn(|axis| vertex[axis] - centre[axis]);
            if let Some(offset) = unit_offset(direction, true) {
                let candidate = [vertex[0] + offset[0], vertex[1] + offset[1], vertex[2] + offset[2], 0.];
                if !outside(candidate)? { points.push(candidate); }
            }
        }
        Ok(points)
    }
}

pub(super) fn contains_candidate(
    hull: &Topology,
    domain: &Domain,
    planes: Option<&[[f32; 4]]>,
    candidate: [f32; 4],
    use_float_planes: bool,
    tolerance: f32,
) -> Result<bool, String> {
    if candidate[..3].iter().any(|value| !value.is_finite()) {
        return Err("hull simplification produced a non-finite candidate".into());
    }
    if use_float_planes {
        let planes = planes.ok_or("hull float planes are not indexed")?;
        return Ok(planes.iter().all(|plane| {
            let distance = ((plane[1] * candidate[1] + plane[0] * candidate[0])
                + plane[2] * candidate[2]) + plane[3];
            !(tolerance < distance)
        }));
    }
    let Some(point) = domain.quantize_inside(candidate) else { return Ok(false) };
    let mut next = hull.first_face;
    while let Some(index) = next {
        let face = hull.faces.get(index).and_then(Option::as_ref)
            .ok_or("hull simplification face is absent")?;
        let side = i64::from(face.normal[2]) * i64::from(point[2])
            + i64::from(face.normal[1]) * i64::from(point[1])
            + i64::from(face.normal[0]) * i64::from(point[0]) + face.distance;
        if side >= 1 { return Ok(false); }
        next = face.next;
    }
    Ok(true)
}

pub(crate) fn calculate(vertices: &[[f32; 4]], radius: f32, config: [u32; 3], bounds: [[f32; 4]; 2])
    -> Result<super::PrincipalMassProperties, String> {
    let Some(mut source) = Spatial::acquire(vertices)? else {
        return Err("convex mass requires its lower-dimensional primitive owner".into());
    };
    let expanded;
    let spatial = if 0. < radius {
        let points = source.expanded_mass_points(radius)?;
        expanded = Spatial::acquire(&points)?.ok_or("rounded convex mass hull is not spatial")?;
        &expanded
    } else { &source };
    // ca38b0 invokes c96ab0 only when ce1360 reports failed mass properties.
    // Do not confuse an invalid source/owner error with that native fallback.
    let (center, volume, tensor) = match properties::inertia_tensor(&spatial.topology, &spatial.domain) {
        Ok(properties) => properties,
        Err(properties::MassError::Degenerate) => return Ok(super::bounds_mass_properties(config, bounds)),
        Err(properties::MassError::Invalid(error)) => return Err(error),
    };
    let (basis, inertia) = properties::principal_frame(tensor)?;
    let requested_mass = f32::from_bits(config[2]);
    let mass = if requested_mass < 0. { 0. - requested_mass * volume } else { requested_mass };
    let scale = mass * f32::from_bits(config[1]);
    Ok(super::PrincipalMassProperties { volume, mass, center,
        inertia: inertia.map(|value| value * scale),
        orientation: crate::assets::physworld::rotation_quaternion(basis) })
}
