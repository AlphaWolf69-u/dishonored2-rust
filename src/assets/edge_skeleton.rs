//! Original Edge skeleton rest data and skin-palette operations.
//! 24f2a0 validates the 0x40-byte header; 24f1d0/24f240 address
//! rest Q/T/S and hierarchy through relative offsets. Custom skeleton data
//! is parsed only through the inverse-bind table understood in 2564e0.

#[derive(Clone, Debug, PartialEq)]
pub struct JointRestPose {
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct EdgeSkeleton {
    pub rest: Vec<JointRestPose>,
    pub parents: Vec<Option<usize>>,
    pub parent_skeleton: Option<String>,
    pub parent_reference: Option<u16>,
    pub inverse_bind: Vec<[[f32; 4]; 3]>,
    pub joint_names: Vec<String>,
    pub hierarchy: Vec<[u16; 2]>,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn bytes(&mut self, count: usize) -> Result<&[u8], String> {
        let end = self
            .at
            .checked_add(count)
            .ok_or("Edge skeleton extent overflow")?;
        let data = self
            .data
            .get(self.at..end)
            .ok_or("truncated Edge skeleton")?;
        self.at = end;
        Ok(data)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn text(&mut self) -> Result<String, String> {
        let length = self.u32()? as usize;
        let text = std::str::from_utf8(self.bytes(length)?).map_err(|e| e.to_string())?;
        Ok(text.trim_end_matches('\0').to_owned())
    }
}

pub fn parse(bytes: &[u8]) -> Result<EdgeSkeleton, String> {
    let mut header = Reader { data: bytes, at: 0 };
    if header.u32()? != 0x4553_3036 {
        return Err("not an original Edge skeleton".into());
    }
    if header.u32()? as usize != bytes.len() {
        return Err("Edge skeleton header/file extent differs".into());
    }
    let word = |offset| -> Result<u32, String> {
        Reader {
            data: bytes,
            at: offset,
        }
        .u32()
    };
    let count = Reader {
        data: bytes,
        at: 0x10,
    }
    .u16()? as usize;
    let hierarchy_count = (word(0x3c)? as usize)
        .checked_add(3)
        .ok_or("Edge hierarchy count overflow")?
        & !3;
    let mut hierarchy = Vec::with_capacity(hierarchy_count);
    let mut hierarchy_data = Reader {
        data: bytes,
        at: 0x40,
    };
    for _ in 0..hierarchy_count {
        hierarchy.push([hierarchy_data.u16()?, hierarchy_data.u16()?]);
    }
    let relative = |offset: usize| -> Result<usize, String> {
        let value = word(offset)? as usize;
        if value == 0 {
            return Err(format!("null Edge skeleton relative table at {offset:#x}"));
        }
        offset
            .checked_add(value)
            .ok_or("Edge skeleton relative offset overflow".into())
    };
    let mut rest = Vec::with_capacity(count);
    let mut parents = Vec::with_capacity(count);
    if count != 0 {
        let rest_start = relative(0x18)?;
        let parent_start = relative(0x1c)?;
        for joint in 0..count {
            let mut data = Reader {
                data: bytes,
                at: rest_start + joint * 48,
            };
            let raw = data.bytes(48)?;
            let value = |i: usize| {
                f32::from_bits(u32::from_le_bytes(
                    raw[i * 4..i * 4 + 4].try_into().unwrap(),
                ))
            };
            rest.push(JointRestPose {
                rotation: std::array::from_fn(value),
                translation: std::array::from_fn(|i| value(i + 4)),
                scale: std::array::from_fn(|i| value(i + 8)),
            });
            let index = Reader {
                data: bytes,
                at: parent_start + joint * 2,
            }
            .u16()? as usize;
            let parent = Reader {
                data: bytes,
                at: 0x42 + index * 4,
            }
            .u16()?
                & 0x7fff;
            if parent > 0x3fff {
                parents.push(None);
            } else if usize::from(parent) < count {
                parents.push(Some(usize::from(parent)));
            } else {
                return Err("Edge skeleton parent exceeds its joint table".into());
            }
        }
    }
    // 24f2a0 reads through header.local_40+0x30 before calling 2564e0.
    let custom_start = (word(0x30)? as usize)
        .checked_add(0x30)
        .ok_or("Edge custom extent overflow")?;
    let mut data = Reader {
        data: bytes,
        at: custom_start,
    };
    data.u32()?; // Custom-data identifier: preserved by the native base loader.
    let parent = data.text()?;
    let parent_skeleton = (!parent.is_empty()).then_some(parent);
    let parent_reference = if parent_skeleton.is_some() {
        Some(data.u16()?)
    } else {
        None
    };
    data.u16()?;
    data.u16()?; // Native fields +150/+152.
    let joint_count = data.u16()? as usize;
    let inherited_count = data.u16()? as usize;
    if inherited_count > 4 {
        return Err("Edge inherited-joint prefix exceeds the native four slots".into());
    }
    if inherited_count != 0 {
        data.bytes(inherited_count * 2)?;
        data.bytes(joint_count * 2)?;
    }
    let mut joint_names = Vec::with_capacity(joint_count);
    for _ in 0..joint_count {
        joint_names.push(data.text()?);
    }
    data.bytes(
        joint_count
            .checked_mul(10)
            .ok_or("Edge joint metadata extent overflow")?,
    )?;
    let channel_count = data.u16()? as usize;
    for _ in 0..channel_count {
        data.text()?;
    }
    data.bytes(
        channel_count
            .checked_mul(10)
            .ok_or("Edge channel metadata extent overflow")?,
    )?;
    let padded = (joint_count + 7) & !7;
    let mut inverse_bind = Vec::with_capacity(padded);
    for _ in 0..padded {
        let raw = data.bytes(48)?;
        inverse_bind.push(std::array::from_fn(|row| {
            std::array::from_fn(|col| {
                let i = (row * 4 + col) * 4;
                f32::from_bits(u32::from_le_bytes(raw[i..i + 4].try_into().unwrap()))
            })
        }));
    }
    if joint_count != count {
        return Err("Edge rest and custom joint counts differ".into());
    }
    Ok(EdgeSkeleton {
        rest,
        parents,
        parent_skeleton,
        parent_reference,
        inverse_bind,
        joint_names,
        hierarchy,
    })
}

fn joint_local_matrix(pose: &JointRestPose) -> [[f32; 4]; 3] {
    let [x, y, z, w] = pose.rotation;
    let [sx, sy, sz] = pose.scale.map(|v| v + v);
    [
        [
            ((0.5 - y * y) - z * z) * sx,
            (y * x - w * z) * sy,
            (z * x + w * y) * sz,
            pose.translation[0],
        ],
        [
            (y * x + w * z) * sx,
            ((0.5 - x * x) - z * z) * sy,
            (z * y - w * x) * sz,
            pose.translation[1],
        ],
        [
            (z * x - w * y) * sx,
            (z * y + w * x) * sy,
            ((0.5 - x * x) - y * y) * sz,
            pose.translation[2],
        ],
    ]
}

#[cfg(target_arch = "x86_64")]
fn scale_reciprocal(value: f32) -> f32 {
    use std::arch::x86_64::*;
    unsafe {
        let input = _mm_set_ss(value);
        let reciprocal = _mm_rcp_ss(input);
        _mm_cvtss_f32(_mm_mul_ss(
            _mm_sub_ss(_mm_set_ss(2.0), _mm_mul_ss(reciprocal, input)),
            reciprocal,
        ))
    }
}

/// 12e1130: Q/T/S -> global affine matrices, four independent hierarchy
/// records at a time. Parent bit14 selects the supplied root transform;
/// bit15 compensates its LOCAL scale in the rotational columns only.
#[cfg(target_arch = "x86_64")]
pub fn pose_matrices(
    local: &[JointRestPose],
    hierarchy: &[[u16; 2]],
    root: &JointRestPose,
) -> Result<Vec<[[f32; 4]; 3]>, String> {
    if hierarchy.len() % 4 != 0 {
        return Err("Edge hierarchy lacks four-lane padding".into());
    }
    let mut result = vec![[[0.0; 4]; 3]; local.len()];
    let mut ready = vec![false; local.len()];
    let root_matrix = joint_local_matrix(root);
    for group in hierarchy.chunks_exact(4) {
        let mut prepared = Vec::with_capacity(4);
        for &[joint, parent] in group {
            let joint = usize::from(joint);
            let pose = local
                .get(joint)
                .ok_or("Edge hierarchy child is out of bounds")?;
            let (parent_matrix, parent_scale) = if parent & 0x4000 != 0 {
                (&root_matrix, root.scale)
            } else {
                let parent = usize::from(parent & 0x3fff);
                if !ready.get(parent).copied().unwrap_or(false) {
                    return Err(
                        "Edge hierarchy parent was not produced in a preceding group".into(),
                    );
                }
                (&result[parent], local[parent].scale)
            };
            let reciprocal = if parent & 0x8000 != 0 {
                parent_scale.map(scale_reciprocal)
            } else {
                [1.0; 3]
            };
            let matrix = joint_local_matrix(pose);
            let output = std::array::from_fn(|row| {
                std::array::from_fn(|column| {
                    if column == 3 {
                        ((parent_matrix[row][3] + parent_matrix[row][0] * pose.translation[0])
                            + parent_matrix[row][1] * pose.translation[1])
                            + parent_matrix[row][2] * pose.translation[2]
                    } else {
                        (matrix[1][column] * (parent_matrix[row][1] * reciprocal[1])
                            + matrix[0][column] * (parent_matrix[row][0] * reciprocal[0]))
                            + matrix[2][column] * (parent_matrix[row][2] * reciprocal[2])
                    }
                })
            });
            prepared.push((joint, output));
        }
        for (joint, output) in prepared {
            result[joint] = output;
            ready[joint] = true;
        }
    }
    if ready.iter().any(|ready| !*ready) {
        return Err("Edge hierarchy did not produce every joint".into());
    }
    Ok(result)
}

/// 09b980 copies a native column-major 4x4 into the row-major 3x4 skin ABI.
pub fn joint_matrix(columns: [[f32; 4]; 4]) -> [[f32; 4]; 3] {
    std::array::from_fn(|row| std::array::from_fn(|col| columns[col][row]))
}

/// 12e1d00, called by 24c340: pairwise current-joint * inverse-bind.
/// Preserve Y*row1 + X*row0 + Z*row2, followed by the implicit affine row.
pub fn skin_palette(
    current: &[[[f32; 4]; 3]],
    inverse: &[[[f32; 4]; 3]],
) -> Result<Vec<[[f32; 4]; 3]>, String> {
    if current.len() != inverse.len() {
        return Err("skin palette banks have different lengths".into());
    }
    Ok(current
        .iter()
        .zip(inverse)
        .map(|(a, b)| {
            std::array::from_fn(|row| {
                std::array::from_fn(|col| {
                    let implicit = if col == 3 { 1.0 } else { 0.0 };
                    ((a[row][1] * b[1][col] + a[row][0] * b[0][col]) + a[row][2] * b[2][col])
                        + a[row][col] * implicit
                })
            })
        })
        .collect())
}

/// Installed skin VS order: start with weightY, MAD weightX, then Z/W and
/// the optional second quartet. Offset is in joints, before multiplication3.
pub fn vertex_skin_matrix(
    palette: &[[[f32; 4]; 3]],
    offset: u32,
    indices: [u8; 8],
    weights: [u8; 8],
    influences: u8,
) -> Result<[[f32; 4]; 3], String> {
    if !matches!(influences, 4 | 8) {
        return Err("skin input must have four or eight weights".into());
    }
    let mut matrices = [[[0.0; 4]; 3]; 8];
    for i in 0..usize::from(influences) {
        let joint = offset
            .checked_add(u32::from(indices[i]))
            .ok_or("skin joint index overflow")? as usize;
        if let Some(matrix) = palette.get(joint) {
            matrices[i] = *matrix;
        } else if weights[i] != 0 {
            return Err(format!(
                "skin joint {joint} exceeds {} actual palette entries",
                palette.len()
            ));
        }
    }
    let weights = weights.map(|weight| f32::from(weight) / 255.0);
    Ok(std::array::from_fn(|row| {
        std::array::from_fn(|col| {
            let mut value =
                matrices[0][row][col].mul_add(weights[0], matrices[1][row][col] * weights[1]);
            for i in 2..usize::from(influences) {
                value = matrices[i][row][col].mul_add(weights[i], value);
            }
            value
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouped_pose_matches_scale_compensation_and_root() {
        for (case, line) in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/edge_pose_cases.txt"
        ))
        .lines()
        .enumerate()
        {
            let words = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect::<Vec<_>>();
            let pose = |at: usize| JointRestPose {
                rotation: std::array::from_fn(|i| f32::from_bits(words[at + i])),
                translation: std::array::from_fn(|i| f32::from_bits(words[at + 4 + i])),
                scale: std::array::from_fn(|i| f32::from_bits(words[at + 8 + i])),
            };
            let local = (0..8).map(|i| pose(i * 12)).collect::<Vec<_>>();
            let root = pose(96);
            let hierarchy = (0..8)
                .map(|i| [words[108 + i] as u16, (words[108 + i] >> 16) as u16])
                .collect::<Vec<_>>();
            let actual = pose_matrices(&local, &hierarchy, &root).unwrap();
            for joint in 0..8 {
                for i in 0..12 {
                    assert_eq!(
                        actual[joint][i / 4][i % 4].to_bits(),
                        words[116 + joint * 12 + i],
                        "case{case} joint{joint} lane{i}"
                    );
                }
            }
        }
    }
    #[test]
    fn joint_and_palette_match_executable() {
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/skin_palette_cases.txt"
        ))
        .lines()
        {
            let words = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect::<Vec<_>>();
            let a =
                std::array::from_fn(|r| std::array::from_fn(|c| f32::from_bits(words[r * 4 + c])));
            let b = std::array::from_fn(|r| {
                std::array::from_fn(|c| f32::from_bits(words[12 + r * 4 + c]))
            });
            let actual = skin_palette(&[a], &[b]).unwrap()[0];
            for i in 0..12 {
                assert_eq!(actual[i / 4][i % 4].to_bits(), words[24 + i]);
            }
            let columns = std::array::from_fn(|r| {
                std::array::from_fn(|c| f32::from_bits(words[36 + r * 4 + c]))
            });
            let actual = joint_matrix(columns);
            for i in 0..12 {
                assert_eq!(actual[i / 4][i % 4].to_bits(), words[52 + i]);
            }
        }
    }
}
