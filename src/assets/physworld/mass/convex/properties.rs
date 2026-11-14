//! Vertex means, spatial volume, area and centroid used by hull construction.
//! These fixed-size construction values are not an SDK mass-properties asset.
use super::{domain::Domain, topology::Topology};

fn matrix_product(a: [[f32; 4]; 3], b: [[f32; 4]; 3]) -> [[f32; 4]; 3] {
    std::array::from_fn(|column| std::array::from_fn(|row| {
        (b[column][0] * a[0][row] + b[column][1] * a[1][row]) + b[column][2] * a[2][row]
    }))
}

/// 140ae04a0: diagonalize the hull's symmetric inertia tensor. Matrix
/// columns retain their fourth lane because the original 3x3 multiply does.
pub(super) fn diagonalize(mut tensor: [[f32; 4]; 3]) -> Result<([[f32; 4]; 3], [f32; 4]), String> {
    const IDENTITY: [[f32; 4]; 3] = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]];
    let mut axes = IDENTITY;
    let threshold = (((tensor[1][1] * tensor[1][1] + tensor[1][0] * tensor[1][0])
        + tensor[1][2] * tensor[1][2] + tensor[0][1] * tensor[0][1]
        + tensor[0][0] * tensor[0][0] + tensor[0][2] * tensor[0][2]
        + tensor[2][1] * tensor[2][1] + tensor[2][0] * tensor[2][0]
        + tensor[2][2] * tensor[2][2]) * f32::EPSILON) * f32::EPSILON;
    let off_diagonal = |m: &[[f32; 4]; 3]| {
        ((m[2][0] * m[2][0] + m[1][0] * m[1][0]) + m[2][1] * m[2][1]) * 2.
    };
    let mut remainder = off_diagonal(&tensor);
    let mut iteration = 0;
    while threshold < remainder && iteration < 128 {
        let magnitude = |value: f32| f32::from_bits(value.to_bits() & 0x7fff_ffff);
        let mut column = 1usize;
        let mut maximum = magnitude(tensor[1][0]);
        if maximum < magnitude(tensor[2][0]) { column = 2; maximum = magnitude(tensor[2][0]); }
        let row = usize::from(maximum < magnitude(tensor[2][1]));
        if row == 1 { column = 2; }
        let entry = tensor[column][row];
        let (cosine, sine) = if entry == 0. { (1., 0.) } else {
            let ratio = (tensor[column][column] - tensor[row][row]) / (entry * 2.);
            let root = (ratio * ratio + 1.).sqrt();
            let denominator = if ratio < 0. { ratio - root } else { root + ratio };
            let tangent = 1. / denominator;
            let cosine = 1. / (tangent * tangent + 1.).sqrt();
            (cosine, cosine * tangent)
        };
        let mut rotation = IDENTITY;
        let mut inverse = IDENTITY;
        rotation[row][row] = cosine;
        rotation[column][column] = cosine;
        rotation[column][row] = sine;
        rotation[row][column] = f32::from_bits(sine.to_bits() ^ 0x8000_0000);
        inverse[row][row] = cosine;
        inverse[column][column] = cosine;
        inverse[column][row] = f32::from_bits(sine.to_bits() ^ 0x8000_0000);
        inverse[row][column] = sine;
        tensor = matrix_product(inverse, matrix_product(tensor, rotation));
        axes = matrix_product(axes, rotation);
        remainder = off_diagonal(&tensor);
        iteration += 1;
    }
    if threshold < remainder { return Err("hull inertia did not diagonalize".into()); }
    Ok((axes, [tensor[0][0], tensor[1][1], tensor[2][2], tensor[2][2]]))
}

/// b959f0 normalizes the eigenbasis through a quaternion, recomputes the
/// diagonal in that basis, then exports the transposed basis to ce1360.
pub(super) fn principal_frame(tensor: [[f32; 4]; 3]) -> Result<([[f32; 4]; 3], [f32; 4]), String> {
    let (axes, _) = diagonalize(tensor)?;
    let transpose = |basis: [[f32; 4]; 3]| -> [[f32; 4]; 3] {
        std::array::from_fn(|column| [basis[0][column], basis[1][column], basis[2][column], 0.])
    };
    let basis = transpose(axes);
    let basis = crate::assets::physworld::quaternion_basis(crate::assets::physworld::basis_quaternion(basis));
    let intermediate = std::array::from_fn(|column| std::array::from_fn(|row| {
        (basis[1][column] * tensor[1][row] + basis[0][column] * tensor[0][row])
            + basis[2][column] * tensor[2][row]
    }));
    let transformed = matrix_product(basis, intermediate);
    Ok((transpose(basis), [transformed[0][0], transformed[1][1], transformed[2][2], transformed[2][2]]))
}

/// 140b97ad0 (retained source coordinates) / 140b97b50 (quantized export).
/// Linked vertex order and DIVPS's exact division precede multiplication.
pub(super) fn mean(hull: &Topology, domain: Option<&Domain>) -> Result<[f32; 4], String> {
    let mut sum = [0.; 4];
    let mut count = 0usize;
    let mut next = hull.first_vertex;
    while let Some(index) = next {
        let vertex = hull.vertices[index]
            .as_ref()
            .ok_or("hull mean vertex is retired")?;
        let point = domain.map_or(vertex.point.source, |domain| {
            domain.export(vertex.point.integer)
        });
        for axis in 0..3 {
            sum[axis] = point[axis] + sum[axis];
        }
        count += 1;
        next = vertex.next;
    }
    if count != 0 {
        let inverse = crate::float::divide(1., count as f32);
        for value in &mut sum[..3] {
            *value *= inverse;
        }
    }
    Ok(sum)
}

pub(super) enum MassError { Degenerate, Invalid(String) }
impl From<&str> for MassError {
    fn from(error: &str) -> Self { Self::Invalid(error.to_owned()) }
}
impl From<String> for MassError {
    fn from(error: String) -> Self { Self::Invalid(error) }
}

/// Spatial mass accumulators from 140b959f0, in linked face order. The
/// caller must first build the radius-expanded hull required by 140ce1360.
pub(super) fn inertia_tensor(hull: &Topology, domain: &Domain) -> Result<([f32; 4], f32, [[f32; 4]; 3]), MassError> {
    let origin = mean(hull, Some(domain))?;
    let mut volume = 0.;
    let mut first = [0.; 3];
    let mut diagonal = [0.; 3];
    // yz, zx, xy, matching the source's packed lanes.
    let mut products = [0.; 3];
    let limit = f32::from_bits(0x7f7f_ffee);
    let mut minimum = [limit; 3];
    let mut maximum = [-limit; 3];
    let mut next = hull.first_face;
    while let Some(index) = next {
        let face = hull.faces[index].as_ref().ok_or("mass hull face is retired")?;
        let mut points = [[0.; 3]; 3];
        for (corner, vertex) in face.vertices.into_iter().enumerate() {
            let vertex = hull.vertices[vertex].as_ref().ok_or("mass hull vertex is retired")?;
            let point = domain.export(vertex.point.integer);
            for axis in 0..3 {
                points[corner][axis] = point[axis] - origin[axis];
                minimum[axis] = if minimum[axis] < points[corner][axis] { minimum[axis] } else { points[corner][axis] };
                maximum[axis] = if maximum[axis] > points[corner][axis] { maximum[axis] } else { points[corner][axis] };
            }
        }
        let [a, b, c] = points;
        let cross = [c[2] * b[1] - b[2] * c[1], c[0] * b[2] - b[0] * c[2], c[1] * b[0] - b[1] * c[0]];
        let weight = (a[1] * cross[1] + a[0] * cross[0]) + a[2] * cross[2];
        volume += weight;
        for axis in 0..3 {
            first[axis] += ((b[axis] + a[axis]) + c[axis]) * weight;
            diagonal[axis] += (((c[axis] + b[axis]) * b[axis] + (b[axis] + a[axis]) * a[axis])
                + (a[axis] + c[axis]) * c[axis]) * weight;
        }
        for (lane, (x, y)) in [(2, 1), (2, 0), (1, 0)].into_iter().enumerate() {
            products[lane] += (((b[x] * b[y] + a[x] * a[y]) + c[x] * c[y])
                + ((b[x] + a[x]) + c[x]) * ((b[y] + a[y]) + c[y])) * weight;
        }
        next = face.next;
    }
    let inverse = crate::float::divide(1., volume);
    let relative = first.map(|value| (value * inverse) * 0.25);
    let centre = [relative[0] + origin[0], relative[1] + origin[1], relative[2] + origin[2], 0.];
    let extent: [f32; 3] = std::array::from_fn(|axis| maximum[axis] - minimum[axis]);
    let extent = if extent[1] > extent[0] { extent[1] } else { extent[0] };
    let extent = if maximum[2] - minimum[2] > extent { maximum[2] - minimum[2] } else { extent };
    let published_volume = volume * f32::from_bits(0x3e2a_aaab);
    if published_volume <= ((extent * extent) * extent) * f32::EPSILON {
        return Err(MassError::Degenerate);
    }
    let second = diagonal.map(|value| (value * 0.1) * inverse);
    let cross = products.map(|value| (value * -0.05) * inverse);
    let mut tensor = [
        [second[2] + second[1], cross[2], cross[1], 0.],
        [cross[2], second[2] + second[0], cross[0], 0.],
        [cross[1], cross[0], second[1] + second[0], 0.],
    ];
    let c = relative;
    let skew = [[0., c[2], 0. - c[1], 0.], [0. - c[2], 0., c[0], 0.], [c[1], 0. - c[0], 0., 0.]];
    for column in 0..3 {
        for row in 0..4 {
            let offset = (skew[column][0] * skew[0][row] + skew[column][1] * skew[1][row])
                + skew[column][2] * skew[2][row];
            tensor[column][row] += offset;
        }
    }
    Ok((centre, published_volume, tensor))
}

