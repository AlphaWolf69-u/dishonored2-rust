//! Penetration hull face geometry and tetrahedron topology (c228c0/c23070).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceEdge {
    pub face: usize,
    pub edge: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct PenetrationFace {
    pub vertices: [usize; 3],
    pub opposite: [FaceEdge; 3],
    pub normal: [f32; 4],
    pub distance: f32,
    pub mark: u32,
}

pub(super) fn reciprocal_sqrt_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        unsafe { _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(value))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        unimplemented!("penetration face normalization requires x86-64 RSQRTPS")
    }
}

pub fn face_plane(vertices: [[f32; 4]; 3]) -> ([f32; 4], f32) {
    let (normal, distance, _) = face_geometry(vertices);
    (normal, distance)
}

pub(super) fn face_geometry(vertices: [[f32; 4]; 3]) -> ([f32; 4], f32, f32) {
    let a: [f32; 4] = std::array::from_fn(|i| vertices[0][i] - vertices[1][i]);
    let b: [f32; 4] = std::array::from_fn(|i| vertices[1][i] - vertices[2][i]);
    let cross = [
        b[2] * a[1] - a[2] * b[1],
        b[0] * a[2] - a[0] * b[2],
        b[1] * a[0] - a[1] * b[0],
        b[3] * a[3] - a[3] * b[3],
    ];
    let squared = (cross[1] * cross[1] + cross[0] * cross[0]) + cross[2] * cross[2];
    let seed = reciprocal_sqrt_estimate(squared);
    let correction = 3. - seed * squared * seed;
    let factor = if squared <= 0. {
        0.
    } else {
        (correction * seed) * 0.5
    };
    let w_factor = if squared <= 0. {
        0.
    } else {
        correction * (seed * 0.5)
    };
    let normal = [
        cross[0] * factor,
        cross[1] * factor,
        cross[2] * factor,
        cross[3] * w_factor,
    ];
    let distance =
        (vertices[0][1] * normal[1] + vertices[0][0] * normal[0]) + vertices[0][2] * normal[2];
    (normal, distance, squared)
}

/// A tetrahedron's four faces share fixed cyclic edge links. The source flips
/// its orientation by swapping vertex identities0/1, preserving those links.
pub fn tetrahedron_faces(points: [[f32; 4]; 4]) -> [PenetrationFace; 4] {
    let mut vertices = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
    let (normal, _) = face_plane(vertices[3].map(|i| points[i]));
    let delta: [f32; 3] = std::array::from_fn(|i| points[0][i] - points[1][i]);
    let orientation = (delta[1] * normal[1] + delta[0] * normal[0]) + delta[2] * normal[2];
    if orientation >= 0. {
        for face in &mut vertices {
            for vertex in face {
                if *vertex < 2 {
                    *vertex ^= 1;
                }
            }
        }
    }
    let links = [
        [(2, 2), (3, 0), (1, 0)],
        [(0, 2), (3, 2), (2, 0)],
        [(1, 2), (3, 1), (0, 0)],
        [(0, 1), (2, 1), (1, 1)],
    ];
    std::array::from_fn(|i| {
        let (normal, distance) = face_plane(vertices[i].map(|index| points[index]));
        PenetrationFace {
            vertices: vertices[i],
            opposite: links[i].map(|(face, edge)| FaceEdge { face, edge }),
            normal,
            distance,
            mark: 0,
        }
    })
}
