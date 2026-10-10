//! Original single-Tome sphere/cell query (FUN_140a73580 / FUN_140a7f6c0).
//! Collection remapping and the original scratch allocator ABI are unsupported.
use super::{sphere_intersects_portal, word};

#[derive(Debug)]
pub struct SphereCellGraph {
    cells: Vec<[u32; 5]>,
    portals: Vec<[u32; 4]>,
}

#[derive(Debug, PartialEq)]
pub struct SphereCellResult {
    pub visible: bool,
    pub error_code: u32,
    pub indices: Vec<u32>,
    pub count: usize,
}

#[derive(Debug, Default, PartialEq)]
pub struct SphereBatchResult {
    pub error_code: u32,
    pub indices: Vec<u32>,
    pub count: usize,
}

/// Reusable FUN_140a6f530 worker state. The original clears its visited
/// mask per sphere and has exactly 2048 pending (Tome, cell) stack slots.
#[derive(Debug, Default)]
pub struct SphereCellWorker {
    visited: Vec<u32>,
    stack: Vec<u32>,
}

impl SphereCellGraph {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let cell_count = word(bytes, 0x7c)? as usize;
        let cell_offset = word(bytes, 0x80)? as usize;
        let portal_offset = word(bytes, 0x84)? as usize;
        fn rows<const N: usize>(
            bytes: &[u8],
            offset: usize,
            count: usize,
        ) -> Result<Vec<[u32; N]>, String> {
            let end = offset
                .checked_add(
                    count
                        .checked_mul(N * 4)
                        .ok_or("sphere graph size overflow")?,
                )
                .ok_or("sphere graph offset overflow")?;
            if end > bytes.len() || (count != 0 && offset == 0) {
                return Err("sphere graph table exceeds Tome".into());
            }
            (0..count)
                .map(|index| {
                    let mut row = [0; N];
                    for (column, value) in row.iter_mut().enumerate() {
                        *value = word(bytes, offset + (index * N + column) * 4)?;
                    }
                    Ok(row)
                })
                .collect()
        }
        let cells: Vec<[u32; 5]> = rows(bytes, cell_offset, cell_count)?;
        // FUN_140a7f6c0 uses each cell's range; +0x88 is not its portal count.
        let portal_count = cells.iter().try_fold(0usize, |end, cell| {
            (cell[0] as usize)
                .checked_add(cell[1] as usize)
                .map(|next| end.max(next))
                .ok_or("sphere portal range overflow")
        })?;
        let graph = Self {
            cells,
            portals: rows(bytes, portal_offset, portal_count)?,
        };
        for row in &graph.cells {
            let start = row[0] as usize;
            if start
                .checked_add(row[1] as usize)
                .is_none_or(|end| end > graph.portals.len())
            {
                return Err("sphere cell portal range exceeds table".into());
            }
        }
        for portal in &graph.portals {
            if portal[0] & (1 << 28) == 0 && (portal[0] & 0x03ff_ffff) as usize >= cell_count {
                return Err("sphere portal destination exceeds cell table".into());
            }
        }
        Ok(graph)
    }

    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// FUN_140a73580's candidate loop. The caller supplies the original
    /// point-to-global-cell results (None is the native -1 outside case).
    /// Subset order/duplicates and logical output overflow are preserved.
    /// This typed path does not reproduce the engine scratch allocator ABI.
    pub fn query_spheres(
        &self,
        bytes: &[u8],
        worker: &mut SphereCellWorker,
        spheres: &[[f32; 4]],
        point_cells: &[Option<u32>],
        visibility: &[u32],
        gates: Option<&[u32]>,
        subset: Option<&[i32]>,
        capacity: usize,
    ) -> Result<SphereBatchResult, String> {
        if point_cells.len() != spheres.len() || spheres.len() > i32::MAX as usize {
            return Err("sphere batch point-cell count mismatch".into());
        }
        if visibility.len() < self.cells.len().div_ceil(32) {
            return Err("sphere visibility mask is too short".into());
        }
        let mut result = SphereBatchResult::default();
        let candidate_count = subset.map_or(spheres.len(), <[i32]>::len);
        for position in 0..candidate_count {
            let candidate = subset.map_or(position as i32, |indices| indices[position]);
            if candidate < 0 || candidate as usize >= spheres.len() {
                result.error_code = 3;
                break;
            }
            let candidate = candidate as usize;
            let accepted = if let Some(start) = point_cells[candidate] {
                let query = self.query(
                    bytes,
                    worker,
                    start,
                    spheres[candidate],
                    Some(visibility),
                    gates,
                    None,
                    None,
                    false,
                )?;
                if query.error_code != 0 {
                    result.error_code = query.error_code;
                }
                query.visible
            } else {
                true
            };
            if accepted {
                if result.count < capacity {
                    result.indices.push(candidate as u32);
                }
                result.count += 1;
            }
        }
        if result.count > capacity {
            result.error_code = 2;
        }
        Ok(result)
    }

    /// FUN_140a7f6c0's direct, single-Tome traversal. `output_capacity`
    /// requests its optional bounded reachable-cell output. `margin` is
    /// the optional distance result; flag bit 2 selects inside slack only.
    pub fn query(
        &self,
        bytes: &[u8],
        worker: &mut SphereCellWorker,
        start: u32,
        sphere: [f32; 4],
        visibility: Option<&[u32]>,
        gates: Option<&[u32]>,
        output_capacity: Option<usize>,
        margin: Option<&mut f32>,
        inside_only: bool,
    ) -> Result<SphereCellResult, String> {
        if start as usize >= self.cells.len() {
            return Err("sphere start cell exceeds graph".into());
        }
        let words = self.cells.len().div_ceil(32);
        if visibility.is_some_and(|mask| mask.len() < words) {
            return Err("sphere visibility mask is too short".into());
        }
        let visible = |cell: u32| {
            visibility.is_some_and(|mask| mask[cell as usize / 32] & (1 << (cell % 32)) != 0)
        };
        let mut result = SphereCellResult {
            visible: false,
            error_code: 0,
            indices: Vec::new(),
            count: 0,
        };
        if visible(start) {
            result.visible = true;
            return Ok(result);
        }
        let emit = |result: &mut SphereCellResult, index: u32| {
            let Some(capacity) = output_capacity else {
                return true;
            };
            let fits = result.count < capacity;
            if fits {
                result.indices.push(index);
            }
            result.count += 1;
            fits
        };
        if !emit(&mut result, start) {
            result.visible = true;
            result.error_code = 2;
            return Ok(result);
        }
        worker.visited.resize(words, 0);
        worker.visited.fill(0);
        worker.visited[start as usize / 32] |= 1 << (start % 32);
        worker.stack.clear();
        worker.stack.push(start);
        let mut inside = f32::MAX;
        let mut outside = f32::MAX;
        while let Some(cell) = worker.stack.pop() {
            let row = self.cells[cell as usize];
            for &portal in &self.portals[row[0] as usize..(row[0] + row[1]) as usize] {
                if portal[0] & (1 << 28) != 0 {
                    continue;
                }
                if portal[0] & (1 << 27) != 0 && !sphere_portal_gates_allow(bytes, portal, gates)? {
                    continue;
                }
                let destination = portal[0] & 0x03ff_ffff;
                let word_index = destination as usize / 32;
                let bit = 1 << (destination % 32);
                if worker.visited[word_index] & bit != 0 {
                    continue;
                }
                if !(sphere[3] <= 0.0)
                    && !sphere_intersects_portal(
                        bytes,
                        portal,
                        sphere,
                        margin.as_ref().map(|_| &mut inside),
                        (margin.is_some() && !inside_only).then_some(&mut outside),
                    )?
                {
                    continue;
                }
                if visible(destination) {
                    result.visible = true;
                    return Ok(result);
                }
                if !emit(&mut result, destination) || worker.stack.len() >= 2048 {
                    result.visible = true;
                    result.error_code = 2;
                    return Ok(result);
                }
                worker.visited[word_index] |= bit;
                worker.stack.push(destination);
            }
        }
        if let Some(margin) = margin {
            *margin = f32::MAX;
            // Original replaces sub-FLT_MAX values, including negative slack.
            if !(f32::MAX <= inside) {
                *margin = inside;
            }
            if !inside_only && !(*margin <= outside) {
                *margin = outside;
            }
        }
        Ok(result)
    }
}

/// FUN_140a7bab0: only nonnegative gate indices consult the state mask;
/// a missing mask accepts every gate. Collection index remapping is not
/// claimed by this single-Tome path.
pub fn sphere_portal_gates_allow(
    bytes: &[u8],
    portal: [u32; 4],
    states: Option<&[u32]>,
) -> Result<bool, String> {
    let Some(states) = states else {
        return Ok(true);
    };
    let table = word(bytes, 0x78)? as usize;
    let start = (portal[2] >> 12) as usize;
    let count = (portal[2] & 0xfff) as usize;
    for offset in start..start + count {
        let at = table
            .checked_add(offset.checked_mul(4).ok_or("sphere gate index overflow")?)
            .ok_or("sphere gate offset overflow")?;
        let gate = word(bytes, at)?;
        if (gate as i32) < 0 {
            continue;
        }
        let state = states
            .get(gate as usize / 32)
            .ok_or("sphere gate-state mask is too short")?;
        if state & (1 << (gate % 32)) == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_table_extent_comes_from_cell_ranges_not_header_88() {
        let mut bytes = vec![0; 0x100 + 20 + 16];
        bytes[0x7c..0x80].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(&0x100u32.to_le_bytes());
        bytes[0x84..0x88].copy_from_slice(&0x114u32.to_le_bytes());
        bytes[0x88..0x8c].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[0x104..0x108].copy_from_slice(&1u32.to_le_bytes());
        let graph = SphereCellGraph::parse(&bytes).unwrap();
        assert_eq!(graph.portals.len(), 1);
        assert_eq!(graph.cell_count(), 1);
    }

    #[test]
    fn batch_keeps_outside_candidates_and_preserves_subset_order_and_overflow() {
        let graph = SphereCellGraph {
            cells: vec![[0; 5]],
            portals: Vec::new(),
        };
        let spheres = [[0.0; 4]; 3];
        let points = [None, Some(0), None];
        let mut worker = SphereCellWorker::default();
        let result = graph
            .query_spheres(&[], &mut worker, &spheres, &points, &[0], None, None, 3)
            .unwrap();
        assert_eq!(
            result,
            SphereBatchResult {
                error_code: 0,
                indices: vec![0, 2],
                count: 2
            }
        );
        let result = graph
            .query_spheres(
                &[],
                &mut worker,
                &spheres,
                &points,
                &[0],
                None,
                Some(&[2, 0, 2]),
                1,
            )
            .unwrap();
        assert_eq!(
            result,
            SphereBatchResult {
                error_code: 2,
                indices: vec![2],
                count: 3
            }
        );
        let result = graph
            .query_spheres(
                &[],
                &mut worker,
                &spheres,
                &points,
                &[1],
                None,
                Some(&[1, -1, 0]),
                3,
            )
            .unwrap();
        assert_eq!(
            result,
            SphereBatchResult {
                error_code: 3,
                indices: vec![1],
                count: 1
            }
        );
        let result = graph
            .query_spheres(
                &[],
                &mut worker,
                &spheres,
                &points,
                &[1],
                None,
                Some(&[1, 3]),
                0,
            )
            .unwrap();
        assert_eq!(
            result,
            SphereBatchResult {
                error_code: 2,
                indices: Vec::new(),
                count: 1
            }
        );
    }

    #[test]
    fn sphere_cell_traversal_matches_installed_instructions() {
        let mut checked = 0;
        for (case, line) in include_str!("../analysis/original_light_sphere_query_cases.txt")
            .lines()
            .enumerate()
        {
            let (encoded, fields) = line.split_once(' ').unwrap();
            let bytes: Vec<u8> = encoded
                .as_bytes()
                .chunks_exact(2)
                .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
                .collect();
            let values: Vec<u32> = fields
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            let graph = SphereCellGraph::parse(&bytes).unwrap();
            let mut visible = vec![0; graph.cell_count().div_ceil(32)];
            visible[0] = values[1];
            let capacity = (values[3] != u32::MAX).then_some(values[3] as usize);
            let mut margin = 123.0f32;
            let result = graph
                .query(
                    &bytes,
                    &mut SphereCellWorker::default(),
                    values[0],
                    [1.0, 2.0, 1.0, f32::from_bits(values[2])],
                    Some(&visible),
                    None,
                    capacity,
                    (values[4] != 0).then_some(&mut margin),
                    values[4] == 2,
                )
                .unwrap();
            assert_eq!(
                (
                    u32::from(result.visible),
                    result.error_code,
                    margin.to_bits(),
                    result.count
                ),
                (values[5], values[6], values[7], values[8] as usize),
                "sphere query case {case}"
            );
            assert_eq!(
                result.indices,
                values[10..],
                "sphere query stored indices case {case}"
            );
            assert_eq!(result.indices.len(), values[9] as usize);
            checked += 1;
        }
        assert_eq!(checked, 4501);
    }
}
