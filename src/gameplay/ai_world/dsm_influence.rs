//! Search-spot byte influence5bc5f0/5b9890 on the canonical DSM owner.
//! The direct key lookup and FIRST-five KD traversal follow5b9af0/5bc210;
//! replacing the latter with a closest-distance scan changes the chosen cell.
use super::startup_managers::Dsm;
use crate::assets::physworld::navmesh::Graph;

#[derive(Clone, Copy)]
struct SortCell {
    coordinate: f32,
    index: u32,
}
#[derive(Clone, Copy)]
struct Node {
    branches: u32,
    index: u32,
}
pub(super) struct State {
    nodes: Vec<Node>,
    values: Vec<u8>,
}

///18b660: XY cell centres and the actual authored Z displacement. No
///navigation-triangle positions or invented cell bounds participate.
fn position(graph: &Graph, index: usize) -> [f32; 3] {
    let cell = &graph.cells[index];
    [
        cell.key[0] as f32 + graph.grid_origin[0] + 0.5,
        cell.key[1] as f32 + graph.grid_origin[1] + 0.5,
        (cell.key[2] as f32 * 2. + graph.grid_origin[2]) + cell.offset,
    ]
}
impl State {
    fn new(graph: &Graph) -> Result<Self, String> {
        let count = graph.cells.len();
        if count > usize::from(u16::MAX) {
            return Err("DSM cell roster exceeds its source16-bit index domain".into());
        }
        let mut sorted: [Vec<SortCell>; 3] = std::array::from_fn(|axis| {
            (0..count)
                .map(|index| SortCell {
                    coordinate: position(graph, index)[axis],
                    index: index as u32,
                })
                .collect()
        });
        for list in &mut sorted {
            //385a10's comparer deliberately returns-1 on equal coordinates.
            //Reuse the existing source quicksort to retain its tie ordering.
            crate::sorting::quick_sort(
                list,
                |a, b| if b.coordinate < a.coordinate { 1 } else { -1 },
            );
        }
        let mut scratch = vec![
            SortCell {
                coordinate: 0.,
                index: 0
            };
            count
        ];
        let mut state = Self {
            nodes: Vec::with_capacity(count),
            values: vec![127; count],
        };
        if count != 0 {
            state.build(graph, &mut sorted, &mut scratch, 0, 0, count)?;
        }
        Ok(state)
    }
    ///5b94c0→5b90e0 emits root/left/right packed8-byte nodes. Move the
    ///median left across equal coordinates before stable-partitioning the
    ///other two sorted axes, exactly as the source builder does.
    fn build(
        &mut self,
        graph: &Graph,
        sorted: &mut [Vec<SortCell>; 3],
        scratch: &mut [SortCell],
        axis: usize,
        start: usize,
        count: usize,
    ) -> Result<(), String> {
        let mut median = start + count / 2;
        let split = sorted[axis][median].coordinate;
        while median > start && sorted[axis][median - 1].coordinate == split {
            median -= 1;
        }
        let pivot = sorted[axis][median].index;
        let left = median - start;
        let right = count - left - 1;
        sorted[axis][median].index = u32::MAX;
        let branches = ((axis as u32) << 30) | ((u32::from(right != 0)) << 29) | (left as u32);
        self.nodes.push(Node {
            branches,
            index: pivot,
        });
        if left == 0 && right == 0 {
            return Ok(());
        }
        for next in [(axis + 1) % 3, (axis + 2) % 3] {
            scratch[start..start + count].copy_from_slice(&sorted[next][start..start + count]);
            sorted[next][median].index = u32::MAX;
            let (mut low, mut high) = (start, median + 1);
            for entry in &scratch[start..start + count] {
                if entry.index == pivot {
                    continue;
                }
                if entry.index == u32::MAX {
                    return Err("DSM KD partition contains a retired pivot".into());
                }
                if split <= position(graph, entry.index as usize)[axis] {
                    if high >= start + count {
                        return Err("DSM KD upper partition exceeds its source segment".into());
                    }
                    sorted[next][high] = *entry;
                    high += 1;
                } else {
                    if low >= median {
                        return Err("DSM KD lower partition exceeds its source segment".into());
                    }
                    sorted[next][low] = *entry;
                    low += 1;
                }
            }
            if low != median || high != start + count {
                return Err("DSM KD partition count differs from the source split".into());
            }
        }
        if left != 0 {
            self.build(graph, sorted, scratch, (axis + 1) % 3, start, left)?;
        }
        if right != 0 {
            self.build(graph, sorted, scratch, (axis + 1) % 3, median + 1, right)?;
        }
        Ok(())
    }
    fn candidates(
        &self,
        graph: &Graph,
        at: usize,
        point: [f32; 3],
        radius: f32,
        found: &mut [u16; 5],
        count: &mut usize,
    ) -> Result<bool, String> {
        let node = *self
            .nodes
            .get(at)
            .ok_or("DSM KD child offset exceeds its node roster")?;
        let center = position(graph, node.index as usize);
        let delta = [
            center[0] - point[0],
            center[1] - point[1],
            center[2] - point[2],
        ];
        //The scalar addition order is5bc210's Y,X,Z, with strict radius.
        if delta[1] * delta[1] + delta[0] * delta[0] + delta[2] * delta[2] < radius * radius {
            found[*count] = node.index as u16;
            *count += 1;
            if *count == found.len() {
                return Ok(false);
            }
        }
        let axis = (node.branches >> 30) as usize;
        let left = (node.branches & 0x1fff_ffff) as usize;
        if left != 0
            && point[axis] - radius < center[axis]
            && !self.candidates(graph, at + 1, point, radius, found, count)?
        {
            return Ok(false);
        }
        if node.branches & 0x2000_0000 != 0
            && center[axis] <= point[axis] + radius
            && !self.candidates(graph, at + left + 1, point, radius, found, count)?
        {
            return Ok(false);
        }
        Ok(true)
    }
}
impl Dsm {
    fn spot_ready(&mut self) -> Result<bool, String> {
        if !self.initialized {
            return Err("DSM search influence requested before Initialize".into());
        }
        let Some(graph) = self.graph.as_deref() else {
            return Ok(false);
        };
        if graph.cells.is_empty() {
            return Ok(false);
        }
        if self.valid.len() < graph.cells.len().div_ceil(32) {
            return Err("DSM valid-cell bit owner is not initialized".into());
        }
        if self.spot_influence.is_none() {
            //Host: retain the source runtime KD representation only when an
            //actual search-spot consumer requests it; unused levels allocate
            //neither this tree nor the distinct native+5d0 byte field.
            self.spot_influence = Some(State::new(graph)?);
        }
        Ok(true)
    }
    fn cell_valid(&self, index: u16) -> bool {
        self.valid
            .get(usize::from(index) >> 5)
            .is_some_and(|word| word & (1 << (index & 31)) != 0)
    }
    ///5b9af0 first selects an exact quantized valid cell. Only a miss runs
    ///the source preorder radius query, capped before validity filtering.
    fn search_spot_cell(&mut self, point: [f32; 3]) -> Result<Option<u16>, String> {
        if !self.spot_ready()? {
            return Ok(None);
        }
        let graph = self.graph.as_deref().ok_or("DSM graph lease disappeared")?;
        let key = [
            crate::float::truncate_i32(point[0] - graph.grid_origin[0]),
            crate::float::truncate_i32(point[1] - graph.grid_origin[1]),
            crate::float::truncate_i32((point[2] - graph.grid_origin[2]) * 0.5),
        ];
        if key.iter().all(|&value| (0..65535).contains(&value)) {
            if let Some(&index) = self.cell_access.get(&key.map(|value| value as u16)) {
                if index != u16::MAX && self.cell_valid(index) {
                    return Ok(Some(index));
                }
            }
        }
        let state = self
            .spot_influence
            .as_ref()
            .ok_or("DSM spot query state absent")?;
        let mut found = [0u16; 5];
        let mut count = 0;
        state.candidates(graph, 0, point, 5., &mut found, &mut count)?;
        Ok(found[..count]
            .iter()
            .copied()
            .find(|&index| self.cell_valid(index)))
    }
    ///5bc5f0→5beb90 changes only the selected native+5d0 byte. It does
    ///not clamp the source7-bit conversion or silently alter other fields.
    pub fn add_search_influence(
        &mut self,
        point: [f32; 3],
        influence: f32,
    ) -> Result<Option<u16>, String> {
        let Some(index) = self.search_spot_cell(point)? else {
            return Ok(None);
        };
        if self.cell_valid(index) {
            let value = &mut self
                .spot_influence
                .as_mut()
                .ok_or("DSM spot values absent")?
                .values[usize::from(index)];
            let low = if 0. < influence {
                crate::float::truncate_i64(influence * 126.) as u8 & 127
            } else {
                0
            };
            *value = (*value & 128) | low;
        }
        Ok(Some(index))
    }
    ///5b9890→5bea90 restores the constructor byte127 unconditionally,
    ///then5bec00(false) clears protection only for a currently valid cell.
    pub fn remove_search_influence(&mut self, index: u16) -> Result<(), String> {
        if !self.spot_ready()? {
            return Err("DSM removed search influence has no admitted graph".into());
        }
        let state = self
            .spot_influence
            .as_mut()
            .ok_or("DSM spot values absent")?;
        *state
            .values
            .get_mut(usize::from(index))
            .ok_or("DSM removed search cell index exceeds graph")? = 127;
        self.protect_search_influence(index, false)
    }
    ///5bec00, the source high-bit guard shares the same valid-cell owner.
    pub fn protect_search_influence(&mut self, index: u16, protected: bool) -> Result<(), String> {
        if !self.spot_ready()? {
            return Err("DSM protected search influence has no admitted graph".into());
        }
        if self.cell_valid(index) {
            let value = self
                .spot_influence
                .as_mut()
                .ok_or("DSM spot values absent")?
                .values
                .get_mut(usize::from(index))
                .ok_or("DSM protected search cell index exceeds graph")?;
            *value = (*value & 127) | (u8::from(protected) << 7);
        }
        Ok(())
    }
    pub(crate) fn clear_spot_influence(&mut self) {
        self.spot_influence = None;
    }
}
