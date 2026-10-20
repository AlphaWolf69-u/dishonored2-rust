//! Query geometry.
use super::recorded_bounds;
use super::{float, packed_link_geometry, word};

/// FUN_14031c710 uses FUN_14010aca0 to invert the rigid camera pose and
/// FUN_14010ad80 to multiply the projection by that view transform before
/// passing the row-major result to the Umbra camera constructor.
pub fn render_query_base_rows(projection: [[f32; 4]; 4], pose: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut view = [[0.0; 4]; 4];
    for row in 0..3 {
        for column in 0..3 {
            view[row][column] = pose[column][row];
        }
        view[row][3] = -(0..3)
            .map(|axis| pose[axis][row] * pose[axis][3])
            .sum::<f32>();
    }
    view[3][3] = 1.0;
    std::array::from_fn(|row| {
        std::array::from_fn(|column| {
            (0..4)
                .map(|axis| projection[row][axis] * view[axis][column])
                .sum()
        })
    })
}

/// Compose the ordinary renderer projection and rigid camera pose used by
/// FUN_14031c710 before it builds the Umbra query descriptor. Inputs are the
/// renderer's horizontal reference FOV, active viewport dimensions and pixel
/// aspect, the camera pose in Tome/game coordinates, and the renderer near/far
/// values. The caller remains responsible for selecting custom or subrect
/// projection branches when those are active.
pub fn render_query_matrix(
    horizontal_reference_degrees: f32,
    render_width: u32,
    render_height: u32,
    pixel_aspect: f32,
    camera_pose: [[f32; 4]; 4],
    near: f32,
    far: f32,
) -> Result<[[f32; 4]; 4], String> {
    if camera_pose.iter().flatten().any(|value| !value.is_finite())
        || !near.is_finite()
        || near <= 0.0
        || !far.is_finite()
        || far < 0.0
    {
        return Err("invalid original render-query camera input".into());
    }
    let (horizontal_fov, vertical_fov) = render_fov_radians(
        horizontal_reference_degrees,
        render_width,
        render_height,
        pixel_aspect,
    )?;
    let right = near * (horizontal_fov * 0.5).tan();
    let top = near * (vertical_fov * 0.5).tan();
    let projection = render_perspective_rows([-right, right], [-top, top], near, far)?;
    Ok(render_query_base_rows(projection, camera_pose))
}

/// FUN_14031c710 copies the renderer projection, then replaces its depth
/// coefficients before submitting the flags-zero world visibility query.
/// The render projection is reverse depth, while this query uses near=0 and
/// far=1 in clip space. The replacement also reverses the 4x4 determinant
/// sign used by FUN_140a77560 to choose portal winding.
pub fn world_visibility_query_matrix(
    horizontal_reference_degrees: f32,
    render_width: u32,
    render_height: u32,
    pixel_aspect: f32,
    camera_pose: [[f32; 4]; 4],
    near: f32,
    far: f32,
) -> Result<[[f32; 4]; 4], String> {
    if camera_pose.iter().flatten().any(|value| !value.is_finite())
        || !near.is_finite()
        || near <= 0.0
        || !far.is_finite()
        || far < 0.0
    {
        return Err("invalid world visibility depth limits".into());
    }
    let (horizontal_fov, vertical_fov) = render_fov_radians(
        horizontal_reference_degrees,
        render_width,
        render_height,
        pixel_aspect,
    )?;
    let right = near * (horizontal_fov * 0.5).tan();
    let top = near * (vertical_fov * 0.5).tan();
    let mut projection = render_perspective_rows([-right, right], [-top, top], near, far)?;
    // 0x141b26854 is 7000.0f32, the original far fallback for this path.
    let query_far = if far <= near { 7000.0 } else { far };
    let denominator = query_far - near;
    projection[2][2] = -query_far / denominator;
    projection[2][3] = -(near * query_far) / denominator;
    Ok(render_query_base_rows(projection, camera_pose))
}

/// FUN_140a77560 stores (determinant(query matrix) < 0) at context +0x4b4.
pub fn query_matrix_winding_flip(rows: [[f32; 4]; 4]) -> bool {
    let minor = |omit: usize| {
        let columns = (0..4).filter(|&column| column != omit).collect::<Vec<_>>();
        let a = rows[1][columns[0]];
        let b = rows[1][columns[1]];
        let c = rows[1][columns[2]];
        let d = rows[2][columns[0]];
        let e = rows[2][columns[1]];
        let f = rows[2][columns[2]];
        let g = rows[3][columns[0]];
        let h = rows[3][columns[1]];
        let i = rows[3][columns[2]];
        a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
    };
    rows[0][0] * minor(0) - rows[0][1] * minor(1) + rows[0][2] * minor(2) - rows[0][3] * minor(3)
        < 0.0
}

/// FUN_140a77560 transposes the renderer's projection-view rows at
/// 0x140a77721 before placing them in the Tome query context. Object and
/// portal projection multiply world XYZ against these stored float4 vectors.
pub fn stored_query_matrix_from_render_rows(rows: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|row| std::array::from_fn(|column| rows[column][row]))
}

/// FUN_1401098e0 builds this row-major perspective matrix from the left/right
/// and bottom/top frustum bounds supplied by FUN_1401099e0. The engine uses
/// reverse depth: the near plane maps to 1 and the finite far plane maps to 0.
/// A far value no greater than near selects the executable's infinite-far
/// branch.
pub fn render_perspective_rows(
    horizontal_bounds: [f32; 2],
    vertical_bounds: [f32; 2],
    near: f32,
    far: f32,
) -> Result<[[f32; 4]; 4], String> {
    if horizontal_bounds
        .iter()
        .chain(vertical_bounds.iter())
        .chain([near, far].iter())
        .any(|value| !value.is_finite())
        || horizontal_bounds[0] == horizontal_bounds[1]
        || vertical_bounds[0] == vertical_bounds[1]
    {
        return Err("invalid original render-view projection bounds".into());
    }

    let [left, right] = horizontal_bounds;
    let [bottom, top] = vertical_bounds;
    let (depth_scale, depth_offset) = if near < far {
        let denominator = far - near;
        (near / denominator, near * far / denominator)
    } else {
        (-1.0, -far)
    };
    Ok([
        [
            2.0 * near / (right - left),
            0.0,
            (right + left) / (right - left),
            0.0,
        ],
        [
            0.0,
            2.0 * near / (top - bottom),
            (top + bottom) / (top - bottom),
            0.0,
        ],
        [0.0, 0.0, depth_scale, depth_offset],
        [0.0, 0.0, -1.0, 0.0],
    ])
}

/// Derive the original render-view horizontal and vertical FOV values.
/// FUN_140890b50 starts from ark_fieldOfView in degrees, uses the fixed
/// 9:16 reference-height factor (DAT_141aca860 = 0.5625) to derive vertical
/// FOV, then derives horizontal FOV from the active viewport width, height,
/// and pixel aspect. The width/height/pixel-aspect inputs come from the
/// renderer state fields read by FUN_140890ac0/FUN_1408908b0.
pub fn render_fov_radians(
    horizontal_reference_degrees: f32,
    render_width: u32,
    render_height: u32,
    pixel_aspect: f32,
) -> Result<(f32, f32), String> {
    if !horizontal_reference_degrees.is_finite()
        || !(0.0..180.0).contains(&horizontal_reference_degrees)
        || render_width == 0
        || render_height == 0
        || !pixel_aspect.is_finite()
        || pixel_aspect <= 0.0
    {
        return Err("invalid original render-view FOV inputs".into());
    }

    let vertical = render_vertical_fov_radians(horizontal_reference_degrees)?;
    let aspect = (render_width as f32 * pixel_aspect) / render_height as f32;
    let horizontal = 2.0 * ((vertical * 0.5).tan() * aspect).atan();
    Ok((horizontal, vertical))
}

/// FUN_140890b50's vertical-FOV conversion from the horizontal reference CVar.
pub fn render_vertical_fov_radians(
    horizontal_reference_degrees: f32,
) -> Result<f32, String> {
    if !horizontal_reference_degrees.is_finite()
        || !(0.0..180.0).contains(&horizontal_reference_degrees)
    {
        return Err("invalid original render-view reference FOV".into());
    }
    let half_reference = horizontal_reference_degrees * (std::f32::consts::PI / 180.0) * 0.5;
    Ok(2.0 * (half_reference.tan() * 0.5625).atan())
}

/// FUN_140a77560's automatic partition for a flags-0 render query. The
/// caller supplies a job index in [0, count), with count in [1, 16]. The
/// original code selects up to a 4x4 grid, then extends a job into an
/// unassigned neighbor when the grid has more slots than jobs.
pub fn render_query_job_tile(job: u32, count: u32) -> Result<[i32; 4], String> {
    if count == 0 || count > 16 || job >= count {
        return Err("invalid Tome render-query job index or count".into());
    }
    let mut columns = 1;
    let mut rows = 1;
    let mut waste = count * count;
    'search: for candidate_columns in 1..=4 {
        for candidate_rows in (candidate_columns as i32 - 2)..=candidate_columns as i32 {
            if candidate_rows < 1 {
                continue;
            }
            let slots = candidate_columns * candidate_rows as u32;
            if slots >= count && slots - count < waste {
                columns = candidate_columns;
                rows = candidate_rows as u32;
                waste = slots - count;
                if waste == 0 {
                    break 'search;
                }
            }
        }
    }
    let x_index = job / rows;
    let y_index = job % rows;
    let mut x_end = x_index + 1;
    if job + rows < rows * columns && count <= job + rows {
        x_end += 1;
    }
    let boundary = |index: u32, divisions: u32| -> i32 {
        ((index.min(4 % divisions) + index * (4 / divisions)) * 16) as i32
    };
    Ok([
        boundary(x_index, columns),
        boundary(y_index, rows),
        boundary(x_end, columns),
        boundary(y_index + 1, rows),
    ])
}

/// CVar registrations FUN_141379390/FUN_1413665e0: defaults 12 and 2.
/// DAT_141af3a2c is the ASCII string "12", not a numeric/one-digit field.
pub const DEFAULT_QUERY_JOB_COUNT: u32 = 12;
pub const DEFAULT_SCHEDULER_THREADING_MODE: u32 = 2;
/// FUN_14037e1d0's world-object buffer, separate from its visible-cell buffer.
pub const WORLD_QUERY_OUTPUT_CAPACITY: usize = 0x10000;
/// FUN_14037e1d0 supplies this second list to the later light query.
pub const VISIBLE_CELL_OUTPUT_CAPACITY: usize = 0x8000;

/// FUN_14037ef90's ordinary world-query branch (caller param_12 = 1).
/// The CVar permits 1..32; dispatch caps it at 16 and mode 0 forces one job.
pub fn world_query_job_count(configured: u32, threading_mode: u32) -> Result<u32, String> {
    if !(1..=32).contains(&configured) || threading_mode > 2 {
        return Err("invalid original Umbra job count or scheduler threading mode".into());
    }
    Ok(if threading_mode == 0 {
        1
    } else {
        configured.min(16)
    })
}

// FUN_1411f8980: segment/AABB overlap by the three box axes and three
// cross-product axes. Called for each edge by FUN_1411f8be0.
pub(super) fn segment_intersects_aabb(bounds: [[f32; 3]; 2], segment: [[f32; 3]; 2]) -> bool {
    let box_half = std::array::from_fn::<_, 3, _>(|axis| (bounds[1][axis] - bounds[0][axis]) * 0.5);
    let segment_half =
        std::array::from_fn::<_, 3, _>(|axis| (segment[1][axis] - segment[0][axis]) * 0.5);
    let delta = std::array::from_fn::<_, 3, _>(|axis| {
        (segment[0][axis] + segment_half[axis]) - (bounds[0][axis] + bounds[1][axis]) * 0.5
    });
    if (0..3).any(|axis| segment_half[axis].abs() + box_half[axis] < delta[axis].abs()) {
        return false;
    }
    if segment_half[1].abs() * box_half[2] + segment_half[2].abs() * box_half[1]
        < (delta[2] * segment_half[1] - delta[1] * segment_half[2]).abs()
        || segment_half[0].abs() * box_half[2] + segment_half[2].abs() * box_half[0]
            < (delta[0] * segment_half[2] - delta[2] * segment_half[0]).abs()
        || segment_half[0].abs() * box_half[1] + segment_half[1].abs() * box_half[0]
            < (delta[1] * segment_half[0] - delta[0] * segment_half[1]).abs()
    {
        return false;
    }
    true
}

// FUN_1411f8370 tests a segment against a convex four-corner polygon by
// signed four-point determinants. Its callers supply a quad and a box diagonal.
pub(super) fn segment_intersects_quad(segment: [[f32; 3]; 2], quad: [[f32; 3]; 4]) -> bool {
    pub(super) fn side(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) -> bool {
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let w = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
        let determinant = (u[1] * v[2] - u[2] * v[1]) * w[0]
            + (u[2] * v[0] - u[0] * v[2]) * w[1]
            + (u[0] * v[1] - u[1] * v[0]) * w[2];
        determinant.to_bits() & 0x8000_0000 == 0
    }
    if side(segment[0], quad[0], quad[1], quad[2]) == side(segment[1], quad[0], quad[1], quad[2]) {
        return false;
    }
    let first = side(quad[0], quad[1], segment[0], segment[1]);
    (1..4).all(|edge| first == side(quad[edge], quad[(edge + 1) & 3], segment[0], segment[1]))
}

/// FUN_1411f8be0's box/four-corner query overlap, used by FUN_140a80a50
/// in the world renderer's flags-0 global spatial-tree pass.
pub fn quad_intersects_aabb(bounds: [[f32; 3]; 2], quad: [[f32; 3]; 4]) -> bool {
    if (0..3).any(|axis| {
        !bounds[0][axis].is_finite()
            || !bounds[1][axis].is_finite()
            || bounds[0][axis] > bounds[1][axis]
    }) || quad.iter().flatten().any(|value| !value.is_finite())
    {
        return false;
    }
    let outcode = |point: [f32; 3]| -> u8 {
        let mut result = 0;
        for axis in 0..3 {
            if point[axis] < bounds[0][axis] {
                result |= 1 << axis;
            }
            if point[axis] > bounds[1][axis] {
                result |= 1 << (axis + 3);
            }
        }
        result
    };
    let codes = quad.map(outcode);
    if codes.contains(&0) {
        return true;
    }
    if codes
        .into_iter()
        .reduce(|left, right| left & right)
        .unwrap()
        != 0
    {
        return false;
    }
    for edge in 0..4 {
        if segment_intersects_aabb(bounds, [quad[edge], quad[(edge + 1) & 3]]) {
            return true;
        }
    }
    let a = quad[0];
    let b = quad[1];
    let c = quad[2];
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let normal = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let diagonal = [
        std::array::from_fn(|axis| bounds[usize::from(normal[axis].to_bits() >> 31 != 0)][axis]),
        std::array::from_fn(|axis| bounds[usize::from(normal[axis].to_bits() >> 31 == 0)][axis]),
    ];
    segment_intersects_quad(diagonal, quad)
}

/// FUN_140a7fd30: sphere passage through one global-cell portal, with the
/// optional positive/negative slack outputs used by the traversal. Geometry
/// belongs to the Tome root, not a render query's local spatial record.
pub fn sphere_intersects_portal(
    bytes: &[u8],
    link: [u32; 4],
    sphere: [f32; 4],
    inside_slack: Option<&mut f32>,
    outside_gap: Option<&mut f32>,
) -> Result<bool, String> {
    let point = [sphere[0], sphere[1], sphere[2]];
    let radius = sphere[3];
    let radius_squared = radius * radius;
    let (intersects, distance_squared) = if link[0] & (1 << 27) == 0 {
        let geometry = packed_link_geometry(link).ok_or("missing sphere portal geometry")?;
        let axis = geometry.axis as usize;
        if axis >= 3 {
            return Err("unsupported fourth-axis Tome sphere portal".into());
        }
        let root = recorded_bounds(bytes, 0x14)?;
        let mut lower = [0.0f32; 3];
        let mut upper = [0.0f32; 3];
        let interpolate = |dimension: usize, quantized: u16| {
            let q = quantized as f32 * (1.0 / 65535.0);
            (1.0 - q) * root[0][dimension] + q * root[1][dimension]
        };
        lower[axis] = interpolate(axis, geometry.plane);
        upper[axis] = lower[axis];
        for span in 0..2 {
            let other = (axis + span + 1) % 3;
            lower[other] = interpolate(other, geometry.spans[span][0]) - 0.0;
            upper[other] = interpolate(other, geometry.spans[span][1]) + 0.0;
        }
        let distances: [f32; 3] = std::array::from_fn(|i| {
            // MINPS/MAXPS choose the right operand on an unordered comparison.
            let clamped_low = if lower[i] > point[i] {
                lower[i]
            } else {
                point[i]
            };
            let clamped = if upper[i] < clamped_low {
                upper[i]
            } else {
                clamped_low
            };
            (clamped - point[i]).abs()
        });
        let squared = (distances[1] * distances[1] + distances[0] * distances[0])
            + distances[2] * distances[2];
        (squared <= radius_squared, squared)
    } else {
        let start = (link[3] >> 12) as usize;
        let count = (link[3] & 0xfff) as usize;
        let available = word(bytes, 0x74)? as usize;
        if count < 3 || start.checked_add(count).is_none_or(|end| end > available) {
            return Err("Tome sphere portal geometry exceeds table".into());
        }
        let table = word(bytes, 0x70)? as usize;
        let at = table
            .checked_add(
                start
                    .checked_mul(12)
                    .ok_or("sphere geometry index overflow")?,
            )
            .ok_or("sphere geometry offset overflow")?;
        let read_vertex = |index: usize| -> Result<[f32; 3], String> {
            let offset = at
                .checked_add(
                    index
                        .checked_mul(12)
                        .ok_or("sphere vertex index overflow")?,
                )
                .ok_or("sphere vertex offset overflow")?;
            Ok([
                float(bytes, offset)?,
                float(bytes, offset + 4)?,
                float(bytes, offset + 8)?,
            ])
        };
        let center = read_vertex(0)?;
        let normal = read_vertex(1)?;
        let plane_offset = float(bytes, at + 24)?;
        let inner_radius = float(bytes, at + 28)?;
        let outer_radius = float(bytes, at + 32)?;
        let signed_plane =
            ((normal[1] * point[1] + normal[0] * point[0]) + normal[2] * point[2]) + plane_offset;
        let mut projected: [f32; 3] =
            std::array::from_fn(|i| (point[i] - normal[i] * signed_plane) - center[i]);
        let length = ((projected[1] * projected[1] + projected[0] * projected[0])
            + projected[2] * projected[2])
            .sqrt();
        if length != 0.0 {
            let reciprocal = 1.0 / length;
            for value in &mut projected {
                *value *= reciprocal;
            }
        }
        let distance_to_disk = |disk_radius: f32| {
            let extent = if length < disk_radius {
                length
            } else {
                disk_radius
            };
            let distance: [f32; 3] =
                std::array::from_fn(|i| (center[i] - point[i]) + projected[i] * extent);
            (distance[1] * distance[1] + distance[0] * distance[0]) + distance[2] * distance[2]
        };
        let outer_distance = distance_to_disk(outer_radius);
        if !(outer_distance <= radius_squared) {
            (false, outer_distance)
        } else {
            let inner_distance = distance_to_disk(inner_radius);
            if !(radius_squared <= inner_distance) {
                (true, inner_distance)
            } else {
                let mut nearest = f32::MAX;
                for triangle in 0..count.saturating_sub(5) {
                    let squared = point_triangle_distance_squared(
                        point,
                        [
                            read_vertex(3)?,
                            read_vertex(4 + triangle)?,
                            read_vertex(5 + triangle)?,
                        ],
                    );
                    if squared < nearest {
                        nearest = squared;
                    }
                    if inside_slack.is_none() && nearest <= radius_squared {
                        break;
                    }
                }
                (nearest <= radius_squared, nearest)
            }
        }
    };
    if intersects {
        if let Some(slack) = inside_slack {
            let candidate = radius - distance_squared.sqrt();
            if !(*slack <= candidate) {
                *slack = candidate;
            }
        }
    } else if let Some(gap) = outside_gap {
        let candidate = distance_squared.sqrt() - radius;
        if !(*gap <= candidate) {
            *gap = candidate;
        }
    }
    Ok(intersects)
}

/// FUN_140a804e0: squared point-to-triangle distance used by sphere/gated
/// portal queries. Preserve the original region branches, y/x/z dot order,
/// absolute determinant and 0.0001 degeneracy sentinel (FLT_MAX).
pub fn point_triangle_distance_squared(point: [f32; 3], triangle: [[f32; 3]; 3]) -> f32 {
    let offset: [f32; 3] = std::array::from_fn(|i| triangle[0][i] - point[i]);
    let u: [f32; 3] = std::array::from_fn(|i| triangle[1][i] - triangle[0][i]);
    let v: [f32; 3] = std::array::from_fn(|i| triangle[2][i] - triangle[0][i]);
    let dot = |x: [f32; 3], y: [f32; 3]| (x[1] * y[1] + x[0] * y[0]) + x[2] * y[2];
    let a = dot(u, u);
    let b = dot(v, u);
    let c = dot(v, v);
    let d = dot(u, offset);
    let e = dot(v, offset);
    let f = dot(offset, offset);
    let determinant = (c * a - b * b).abs();
    if determinant < 0.0001 {
        return f32::MAX;
    }
    let s = e * b - d * c;
    let t = d * b - e * a;
    let opposite_edge =
        |s: f32, t: f32| ((t * c + s * b) + e * 2.0) * t + ((t * b + s * a) + d * 2.0) * s;
    let contribution = if determinant < t + s {
        if s >= 0.0 {
            if t >= 0.0 {
                let numerator = ((e + c) - b) - d;
                if numerator <= 0.0 {
                    e * 2.0 + c
                } else {
                    let denominator = (a - b * 2.0) + c;
                    if numerator < denominator {
                        let s = numerator / denominator;
                        opposite_edge(s, 1.0 - s)
                    } else {
                        d * 2.0 + a
                    }
                }
            } else {
                let endpoint = d + a;
                if endpoint <= e + b {
                    if endpoint <= 0.0 {
                        d * 2.0 + a
                    } else if d >= 0.0 {
                        return f.abs();
                    } else {
                        (-d / a) * d
                    }
                } else {
                    let numerator = endpoint - (e + b);
                    let denominator = (a - b * 2.0) + c;
                    if numerator < denominator {
                        let t = numerator / denominator;
                        // This branch adds the c/t term before the a/s term.
                        opposite_edge(1.0 - t, t)
                    } else {
                        e * 2.0 + c
                    }
                }
            }
        } else {
            let endpoint = e + c;
            if endpoint <= d + b {
                if endpoint <= 0.0 {
                    e * 2.0 + c
                } else if e >= 0.0 {
                    return f.abs();
                } else {
                    (-e / c) * e
                }
            } else {
                let numerator = endpoint - (d + b);
                let denominator = (a - b * 2.0) + c;
                if numerator < denominator {
                    let s = numerator / denominator;
                    opposite_edge(s, 1.0 - s)
                } else {
                    d * 2.0 + a
                }
            }
        }
    } else if s < 0.0 {
        if t >= 0.0 || d >= 0.0 {
            if e >= 0.0 {
                return f.abs();
            }
            if c <= -e { e * 2.0 + c } else { (-e / c) * e }
        } else if a <= -d {
            d * 2.0 + a
        } else {
            (-d / a) * d
        }
    } else if t >= 0.0 {
        let reciprocal = 1.0 / determinant;
        opposite_edge(s * reciprocal, t * reciprocal)
    } else if d >= 0.0 {
        return f.abs();
    } else if a <= -d {
        d * 2.0 + a
    } else {
        (-d / a) * d
    };
    (contribution + f).abs()
}
