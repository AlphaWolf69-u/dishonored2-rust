//! 381cd0/381730 build the Voronoi accessor's packed, preorder k-d tree;
//! 3856c0 visits the near branch first and retains four nearest cell IDs.
//! Backing belongs to the map's ProbeLightingSet; construction scratch dies
//! before loading completes, and queries allocate nothing.
use super::query::CellGeometry;

#[derive(Clone, Copy, Debug)]
struct Node { branches: u32, cell: u32 }

#[derive(Clone, Debug, Default)]
pub(super) struct CellTree { nodes: Vec<Node> }

#[derive(Clone, Copy, Default)]
struct Row { coordinate: f32, cell: u32 }

// 385a10's comparer returns -1 for equal coordinates. Its small-range
// selection sort and quicksort swaps therefore matter to native tie order.
fn sort(rows: &mut [Row]) {
    if rows.len() < 2 { return; }
    let mut pending = Vec::with_capacity(128);
    let (mut lo, mut hi) = (0isize, rows.len() as isize - 1);
    loop {
        while hi - lo > 3 && pending.len() < 127 {
            rows.swap(hi as usize, ((hi + lo) / 2) as usize);
            let pivot = rows[hi as usize].coordinate;
            let (mut i, mut j) = (lo, hi - 1);
            loop {
                while !(pivot < rows[i as usize].coordinate) && i < j { i += 1; }
                while pivot < rows[j as usize].coordinate {
                    j -= 1;
                    if j <= i { break; }
                }
                if j <= i { break; }
                rows.swap(i as usize, j as usize);
                j -= 1; i += 1;
                if i >= j { break; }
            }
            while pivot < rows[j as usize].coordinate && lo < j { j -= 1; }
            while !(pivot < rows[i as usize].coordinate) && i < hi { i += 1; }
            rows.swap(i as usize, hi as usize);
            if j - lo < hi - i {
                pending.push((i, hi)); hi = j;
            } else {
                pending.push((lo, j)); lo = i;
            }
        }
        while lo < hi {
            let mut largest = lo;
            for i in lo + 1..=hi {
                if rows[largest as usize].coordinate < rows[i as usize].coordinate { largest = i; }
            }
            rows.swap(largest as usize, hi as usize); hi -= 1;
        }
        let Some(range) = pending.pop() else { break; };
        (lo, hi) = range;
    }
}

impl CellTree {
    /// A local-only executable-oracle packet, requested explicitly by diagnostics.
    /// No fixture or copied game data is retained during normal gameplay.
    pub(super) fn write_fixture(&self, path: &std::path::Path, cells: &[CellGeometry],
                               tetrahedra: &[crate::pnetwork::RuntimeTetrahedron]) -> Result<(), String> {
        use std::io::Write;
        let mut output = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| e.to_string())?);
        output.write_all(b"VOROTR01").map_err(|e| e.to_string())?;
        for count in [self.nodes.len(), tetrahedra.len()] {
            output.write_all(&(count as u32).to_le_bytes()).map_err(|e| e.to_string())?;
        }
        for node in &self.nodes { for word in [node.branches, node.cell] {
            output.write_all(&word.to_le_bytes()).map_err(|e| e.to_string())?;
        } }
        for cell in cells { for word in cell.center {
            output.write_all(&word.to_le_bytes()).map_err(|e| e.to_string())?;
        } }
        for tetrahedron in tetrahedra {
            for row in tetrahedron.positions.iter().chain(&tetrahedron.face_normals) { for word in row {
                output.write_all(&word.to_le_bytes()).map_err(|e| e.to_string())?;
            } }
        }
        output.flush().map_err(|e| e.to_string())
    }
    pub(super) fn build(cells: &[CellGeometry]) -> Result<Self, String> {
        if cells.len() >= 1 << 29 { return Err("Voronoi tree exceeds packed child index".into()); }
        let mut axes: [Vec<Row>; 3] = std::array::from_fn(|axis| {
            cells.iter().enumerate().map(|(cell, geometry)| Row {
                coordinate: geometry.center[axis], cell: cell as u32,
            }).collect()
        });
        for axis in &mut axes { sort(axis); }
        let mut tree = Self { nodes: Vec::with_capacity(cells.len()) };
        let mut scratch = vec![Row::default(); cells.len()];
        if !cells.is_empty() { tree.branch(cells, &mut axes, &mut scratch, 0, 0, cells.len()); }
        Ok(tree)
    }

    fn branch(&mut self, cells: &[CellGeometry], axes: &mut [Vec<Row>; 3], scratch: &mut [Row],
              mut axis: usize, mut start: usize, mut count: usize) {
        loop {
            let mut middle = start + count / 2;
            let coordinate = axes[axis][middle].coordinate;
            while middle > start && axes[axis][middle - 1].coordinate == coordinate { middle -= 1; }
            let cell = axes[axis][middle].cell;
            let left = middle - start;
            let right = count - left - 1;
            self.nodes.push(Node { cell, branches: ((axis as u32) << 30)
                | (u32::from(right != 0) << 29) | left as u32 });
            if left == 0 && right == 0 { return; }
            for other in [(axis + 1) % 3, (axis + 2) % 3] {
                scratch[start..start + count].copy_from_slice(&axes[other][start..start + count]);
                let (mut l, mut r) = (start, middle + 1);
                for row in &scratch[start..start + count] {
                    if row.cell == cell { continue; }
                    if cells[row.cell as usize].center[axis] < coordinate {
                        axes[other][l] = *row; l += 1;
                    } else {
                        axes[other][r] = *row; r += 1;
                    }
                }
            }
            let next = (axis + 1) % 3;
            if left != 0 { self.branch(cells, axes, scratch, next, start, left); }
            if right == 0 { return; }
            axis = next; start = middle + 1; count = right;
        }
    }

    pub(super) fn nearest(&self, cells: &[CellGeometry], point: [f32; 3])
        -> ([(f32, usize); 4], usize) {
        let mut nearest = [(f32::MAX, usize::MAX); 4];
        let mut visits = 0;
        if !self.nodes.is_empty() { self.visit(0, cells, point, &mut nearest, &mut visits); }
        (nearest, visits)
    }

    fn visit(&self, mut index: usize, cells: &[CellGeometry], point: [f32; 3],
             nearest: &mut [(f32, usize); 4], visits: &mut usize) {
        loop {
            let node = self.nodes[index];
            *visits += 1;
            let center = cells[node.cell as usize].center;
            let split = point[(node.branches >> 30) as usize] - center[(node.branches >> 30) as usize];
            let left = (node.branches & 0x1fff_ffff) as usize;
            let right = node.branches & 0x2000_0000 != 0;
            if split >= 0. {
                if right { self.visit(index + left + 1, cells, point, nearest, visits); }
            } else if left != 0 { self.visit(index + 1, cells, point, nearest, visits); }
            let delta: [f32; 3] = std::array::from_fn(|i| center[i] - point[i]);
            let distance = delta[1] * delta[1] + delta[0] * delta[0] + delta[2] * delta[2];
            if distance < nearest[3].0 {
                nearest[3] = (distance, node.cell as usize);
                for k in (1..4).rev() {
                    if nearest[k].0 < nearest[k - 1].0 { nearest.swap(k, k - 1); }
                }
            }
            if (left == 0 && !right) || nearest[3].0 <= split * split { return; }
            if split >= 0. {
                if left == 0 { return; }
                index += 1;
            } else {
                if !right { return; }
                index += left + 1;
            }
        }
    }
}
