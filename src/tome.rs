//! Authored global spatial tree from the game's Umbra Tome v18.
//!
//! FUN_140a7b6c0 and FUN_140a89500 use the header's packed two-bit tree;
//! FUN_140a89660 chooses a branch using the recorded split plane, and
//! FUN_140a7a710 derives child indices from the rank of internal nodes.
//! The ordinary camera query combines global spatial selection, local cell
//! traversal, portal coverage, and per-cell object acceptance.

use std::collections::{HashMap, VecDeque};

#[path = "tome_sphere.rs"]
mod sphere;
pub use sphere::{
    sphere_portal_gates_allow, SphereBatchResult, SphereCellGraph, SphereCellResult,
    SphereCellWorker,
};

#[derive(Debug)]
pub struct SpatialTree {
    bounds: Vec<[[f32; 3]; 2]>,
    children: Vec<Option<[usize; 2]>>,
    axes: Vec<u8>,
    planes: Vec<f32>,
    offsets: Vec<u32>,
    global_distance_scales: Option<Vec<f32>>,
}

/// Raw 0x28-byte split-query entry used by FUN_140a81340. The executable
/// overlays the eighth copied dword with three byte-sized path fields, so the
/// entry stays byte-addressed rather than assigning unsupported meanings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpatialQueryBranchEntry {
    bytes: [u8; 0x28],
}

impl SpatialQueryBranchEntry {
    pub fn from_bytes(bytes: [u8; 0x28]) -> Self {
        Self { bytes }
    }

    pub fn bytes(&self) -> &[u8; 0x28] {
        &self.bytes
    }

    pub fn spatial_node_index(&self) -> u32 {
        u32::from_le_bytes(self.bytes[0x0c..0x10].try_into().unwrap())
    }

    pub fn branch_mode(&self) -> u32 {
        u32::from_le_bytes(self.bytes[0x20..0x24].try_into().unwrap())
    }

    pub fn query_state(&self) -> u32 {
        u32::from_le_bytes(self.bytes[0x24..0x28].try_into().unwrap())
    }
}

fn global_node_overlaps_query_planes(
    bounds: [[f32; 3]; 2],
    planes: &[[f32; 4]],
    camera_imprecision: [f32; 3],
) -> bool {
    let expanded = [
        [
            bounds[0][0] - camera_imprecision[0],
            bounds[0][1] - camera_imprecision[1],
            bounds[0][2] - camera_imprecision[2],
            1.0,
        ],
        [
            bounds[1][0] + camera_imprecision[0],
            bounds[1][1] + camera_imprecision[1],
            bounds[1][2] + camera_imprecision[2],
            1.0,
        ],
    ];
    !planes.iter().any(|plane| {
        let support: [f32; 4] = std::array::from_fn(|axis| {
            if plane[axis] > 0.0 {
                expanded[1][axis]
            } else {
                expanded[0][axis]
            }
        });
        let terms = std::array::from_fn::<_, 4, _>(|axis| support[axis] * plane[axis]);
        ((terms[0] + terms[1]) + terms[2]) + terms[3] < 0.0
    })
}

impl SpatialTree {
    /// Construct the root 0x28-byte branch record initialized by
    /// FUN_140a80a50/FUN_140a7b910. The surrounding query structure has a
    /// 0x34-byte prefix; this value is only the record beginning at +0x34.
    pub fn initial_query_branch_entry(&self) -> Result<SpatialQueryBranchEntry, String> {
        let bounds = *self
            .bounds
            .first()
            .ok_or("Tome spatial tree has no root bounds")?;
        let root_axis = *self
            .axes
            .first()
            .ok_or("Tome spatial tree has no root split code")?;
        let mut bytes = [0; 0x28];
        for axis in 0..3 {
            write_u32(&mut bytes, axis * 4, bounds[0][axis].to_bits());
            write_u32(&mut bytes, 0x10 + axis * 4, bounds[1][axis].to_bits());
        }
        // FUN_140a7b910 writes the zero fourth lane and the root path header:
        // path length 0, low two root-code bits, and the initial 0x003f mask.
        bytes[0x0c..0x10].fill(0);
        bytes[0x1c] = 0;
        bytes[0x1d] = root_axis;
        bytes[0x1e..0x20].copy_from_slice(&0x003fu16.to_le_bytes());
        // The query allocator zero-initializes branch mode and the caller
        // installs root cell-state index zero immediately after this helper.
        Ok(SpatialQueryBranchEntry::from_bytes(bytes))
    }
}

/// Mutable stack for the original 0x28-byte spatial split entries. This ports
/// FUN_140a81340's insertion operation; the caller still supplies its branch
/// decision and the two state values as the game does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialQueryBranchStack {
    entries: Vec<SpatialQueryBranchEntry>,
    top: usize,
    active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpatialBranchDirection {
    AxisSides([bool; 3]),
    AxisValues([f32; 3]),
}

/// Initial global-tree volume selected by FUN_140a85090 for the ordinary
/// flags-zero renderer query. An inverted six-float input selects the camera
/// point; an ordered input remains an authored AABB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrdinarySpatialQueryVolume {
    CameraPoint([f32; 3]),
    Bounds([[f32; 3]; 2]),
}

/// The three inputs accepted by FUN_140a82220 when FUN_140a85090 validates
/// a query against the global Tome bounds. The four-corner form is produced
/// by FUN_140a774a0 for the original corner-volume mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OriginalSpatialQueryVolume {
    Bounds([[f32; 3]; 2]),
    FourCorners([[f32; 3]; 4]),
    Point([f32; 3]),
}

/// Match FUN_140a82220's query-volume test against the Tome root bounds.
/// AABB mode requires the query bounds to contain the root; four-corner mode
/// accepts when all four input corners lie in the root; point mode tests the
/// camera point inclusively. This is only the initializer's root gate, not
/// the later stateful global-node traversal.
pub fn original_query_accepts_tome_root(
    tree: &SpatialTree,
    volume: OriginalSpatialQueryVolume,
) -> Result<bool, String> {
    let root = tree
        .bounds(0)
        .ok_or("Tome spatial tree has no root bounds")?;
    match volume {
        OriginalSpatialQueryVolume::Bounds(query) => {
            if query.iter().flatten().any(|value| !value.is_finite())
                || (0..3).any(|axis| query[0][axis] > query[1][axis])
            {
                return Err("invalid original Tome query AABB".into());
            }
            Ok((0..3)
                .all(|axis| query[0][axis] <= root[0][axis] && root[1][axis] <= query[1][axis]))
        }
        OriginalSpatialQueryVolume::FourCorners(corners) => {
            if corners.iter().flatten().any(|value| !value.is_finite()) {
                return Err("invalid original Tome query corners".into());
            }
            Ok(corners.iter().all(|corner| {
                (0..3).all(|axis| root[0][axis] <= corner[axis] && corner[axis] <= root[1][axis])
            }))
        }
        OriginalSpatialQueryVolume::Point(point) => {
            if point.iter().any(|value| !value.is_finite()) {
                return Err("invalid original Tome query point".into());
            }
            Ok((0..3).all(|axis| root[0][axis] <= point[axis] && point[axis] <= root[1][axis]))
        }
    }
}

/// Resolve the standard renderer's six-float volume input. FUN_14037e1d0
/// supplies an inverted sentinel, and FUN_140a85090 maps it to the camera
/// point at query-context +0x4a0 when its mode argument is zero.
pub fn ordinary_spatial_query_volume(
    bounds: [f32; 6],
    camera_position: [f32; 3],
) -> Result<OrdinarySpatialQueryVolume, String> {
    if bounds
        .iter()
        .chain(camera_position.iter())
        .any(|value| !value.is_finite())
    {
        return Err("nonfinite ordinary Tome query volume".into());
    }
    if (0..3).any(|axis| bounds[axis + 3] < bounds[axis]) {
        return Ok(OrdinarySpatialQueryVolume::CameraPoint(camera_position));
    }
    Ok(OrdinarySpatialQueryVolume::Bounds([
        [bounds[0], bounds[1], bounds[2]],
        [bounds[3], bounds[4], bounds[5]],
    ]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpatialQueryBranchCandidate {
    pub spatial_node_index: usize,
    pub cell_state_index: u16,
}

/// A selected outer spatial node paired with the pending-list slot returned
/// by FUN_140a85510 and consumed by FUN_140a82340.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpatialRecordActivation {
    pub spatial_node_index: usize,
    pub pending_slot: u16,
    pub activated_cells: usize,
}

/// Result of FUN_140a85af0's ordinary point-mode local-cell initialization,
/// when the camera resolves to a cell and the initial work node is queued.
#[derive(Debug, PartialEq)]
pub struct InitialCameraCellSeed {
    pub spatial_node_index: usize,
    pub pending_slot: u16,
    pub selection: CellSelection,
    pub work_item: CellWorkItem,
}

/// Global state left by the ordinary camera-point branch of FUN_140a80a50.
/// The local-cell coordinator consumes the branch states and pending lists
/// after this first pass; seeded records alone are not final visible objects.
#[derive(Debug)]
pub struct OrdinaryGlobalCameraQuery {
    pub seeded_records: Vec<InitialCameraCellSeed>,
    pub branch_states: Vec<u16>,
    pub branch_stack: SpatialQueryBranchStack,
    pub nodes_visited: usize,
    /// FUN_140a80a50 marks terminal spatial records that direct links may
    /// activate later; FUN_140a85bd0 checks this bit before scheduling one.
    pub link_target_mask: Vec<u32>,
}

impl OrdinaryGlobalCameraQuery {
    /// FUN_140a85510 selects a pending spatial record, then FUN_140a82340
    /// drains its list into the active cell list and clears the global head.
    /// Record-local projection/link setup and cell traversal follow this step.
    pub fn activate_next_spatial_record(
        &mut self,
        tree: &SpatialTree,
        camera_point: [f32; 3],
        active_tree_nodes: &mut [u32],
        worklists: &mut SpatialCellWorklists,
    ) -> Result<Option<SpatialRecordActivation>, String> {
        let Some(candidate) = self.branch_stack.next_leaf(
            tree,
            &self.branch_states,
            active_tree_nodes,
            &tree.planes,
            0,
            tree.planes.len(),
            SpatialBranchDirection::AxisValues(camera_point),
        )?
        else {
            return Ok(None);
        };
        let activation = worklists.activate_spatial_candidate(candidate)?;
        self.branch_states[candidate.cell_state_index as usize] = 0;
        Ok(Some(activation))
    }
}

impl SpatialQueryBranchStack {
    pub fn new(root: SpatialQueryBranchEntry) -> Self {
        Self {
            entries: vec![root],
            top: 0,
            active: true,
        }
    }

    /// Seed the original global split walk from the parsed Tome root record.
    /// Query-wide state arrays and the outer point/frustum tests are still
    /// initialized by their own FUN_140a80a50 stages.
    pub fn from_original_root(tree: &SpatialTree) -> Result<Self, String> {
        Ok(Self::new(tree.initial_query_branch_entry()?))
    }

    pub fn top(&self) -> usize {
        self.top
    }

    pub fn current(&self) -> &SpatialQueryBranchEntry {
        &self.entries[self.top]
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn entry(&self, index: usize) -> Option<&SpatialQueryBranchEntry> {
        self.entries.get(index)
    }

    /// Port FUN_140a81340 using the already-decoded global spatial tree.
    /// `branch_flags` bit 1 pushes the high child; bit 0 pushes the low child.
    /// With both flags set, `prefer_low` controls which child occupies the new
    /// stack top, matching the original write order.
    pub fn push_split(
        &mut self,
        tree: &SpatialTree,
        branch_flags: u8,
        plane_value: u32,
        prefer_low: bool,
        low_state: u32,
        high_state: u32,
    ) -> Result<(), String> {
        if !self.active {
            return Err("cannot split an exhausted Tome query stack".into());
        }
        if branch_flags & !0x03 != 0 || branch_flags == 0 {
            return Err("invalid original Tome split-branch flags".into());
        }
        let current = *self.current();
        let parent = current.spatial_node_index() as usize;
        let axis = current.bytes[0x1d] as usize;
        if axis > 2 {
            return Err("Tome split query parent axis exceeds three spatial axes".into());
        }
        let [low_child, high_child] = tree
            .children
            .get(parent)
            .and_then(|children| *children)
            .ok_or("Tome split query node has no two children")?;

        let both = branch_flags == 0x03;
        let low_depth = self.top + usize::from(both && prefer_low);
        let high_depth = self.top + usize::from(both && !prefer_low);
        let required = self.top + usize::from(both) + 1;
        if self.entries.len() < required {
            self.entries.resize(required, current);
        }
        if both {
            self.entries[self.top + 1] = current;
        }

        if branch_flags & 0x02 != 0 {
            let entry = &mut self.entries[high_depth].bytes;
            write_u32(entry, 0x0c, high_child as u32);
            write_u32(entry, 0x20, 2);
            write_u32(entry, 0x24, high_state);
            entry[0x1d] = tree.axes[high_child];
            write_u32(entry, axis * 4, plane_value);
            entry[0x1c] = entry[0x1c].wrapping_add(1);
            entry[0x1e] &= !(1 << (axis * 2));
        }
        if branch_flags & 0x01 != 0 {
            let entry = &mut self.entries[low_depth].bytes;
            write_u32(entry, 0x0c, low_child as u32);
            write_u32(entry, 0x20, 1);
            write_u32(entry, 0x24, low_state);
            entry[0x1d] = tree.axes[low_child];
            write_u32(entry, 0x10 + axis * 4, plane_value);
            entry[0x1c] = entry[0x1c].wrapping_add(1);
            entry[0x1e] &= !(2 << (axis * 2));
        }
        if both {
            self.top += 1;
        }
        Ok(())
    }

    /// Select the next spatial leaf using FUN_140a85510's cell-state and
    /// branch-order rules. The query supplies the same per-axis input used by
    /// the original point/volume mode; `active_tree_nodes` is its mutable
    /// global-node bitset, and `plane_offset/plane_count` describe the caller's
    /// selected split-plane range.
    pub fn next_leaf(
        &mut self,
        tree: &SpatialTree,
        cell_states: &[u16],
        active_tree_nodes: &mut [u32],
        planes: &[f32],
        plane_offset: usize,
        plane_count: usize,
        direction: SpatialBranchDirection,
    ) -> Result<Option<SpatialQueryBranchCandidate>, String> {
        while self.active {
            let entry = *self.current();
            let state_index = entry.query_state() as usize;
            let state = *cell_states
                .get(state_index)
                .ok_or("Tome split query state index exceeds state table")?;
            let node_index = entry.spatial_node_index() as usize;

            if state & 0x7fff != 0 && state & 0x8000 == 0 {
                let cell_state_index = u16::try_from(state_index)
                    .map_err(|_| "Tome split query state index exceeds 16 bits")?;
                return Ok(Some(SpatialQueryBranchCandidate {
                    spatial_node_index: node_index,
                    cell_state_index,
                }));
            }

            if state & 0x8000 == 0 {
                let mask = active_tree_nodes
                    .get_mut(node_index / 32)
                    .ok_or("Tome active-node bitset is too short")?;
                *mask &= !(1 << (node_index % 32));
                if self.top == 0 {
                    self.active = false;
                } else {
                    self.top -= 1;
                }
                continue;
            }

            let axis = entry.bytes[0x1d] as usize;
            if axis > 2 {
                return Err("Tome split query entry axis exceeds three spatial axes".into());
            }
            let plane = if node_index < plane_count {
                *planes
                    .get(
                        plane_offset
                            .checked_add(node_index)
                            .ok_or("Tome split plane index overflow")?,
                    )
                    .ok_or("Tome split-plane range exceeds source table")?
            } else {
                let low = f32::from_bits(read_u32(&entry.bytes, axis * 4));
                let high = f32::from_bits(read_u32(&entry.bytes, 0x10 + axis * 4));
                (low + high) * 0.5
            };
            let next_state_index = (state & 0x3fff) as usize + 1;
            let next_state = *cell_states
                .get(next_state_index)
                .ok_or("Tome split query next state exceeds state table")?;
            let current_cell_state = *cell_states
                .get((state & 0x3fff) as usize)
                .ok_or("Tome split query current state exceeds state table")?;
            let prefer_low = if next_state & 0x7fff == 0 {
                true
            } else if current_cell_state & 0x7fff != 0 {
                match direction {
                    SpatialBranchDirection::AxisSides(sides) => sides[axis],
                    SpatialBranchDirection::AxisValues(values) => values[axis] < plane,
                }
            } else {
                false
            };
            self.push_split(
                tree,
                3,
                plane.to_bits(),
                prefer_low,
                u32::from(state & 0x3fff),
                next_state_index as u32,
            )?;
        }
        Ok(None)
    }
}

fn read_u32(bytes: &[u8; 0x28], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn write_u32(bytes: &mut [u8; 0x28], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[derive(Debug, PartialEq)]
pub struct CellSelection {
    pub tree_node: usize,
    pub leaf_rank: usize,
    pub cell_index: usize,
    pub cell_id: u32,
    pub link_count: u32,
    pub bounds: [[f32; 3]; 2],
}

/// The second packed tree inside one global spatial record. The game uses
/// FUN_140a7b540 to select a leaf, FUN_140a89440 to decode its cell code,
/// FUN_140a89350 for special portal codes, and FUN_140a7b890 for the cell ID.
#[derive(Debug)]
pub struct CellTree {
    bounds: Vec<[[f32; 3]; 2]>,
    children: Vec<Option<[usize; 2]>>,
    axes: Vec<u8>,
    planes: Vec<f32>,
    leaf_ranks: Vec<Option<usize>>,
    cell_codes: Vec<u32>,
    cell_ids: Vec<u32>,
    cell_visibility_counts: Vec<u32>,
    direct_cell_ids_available: bool,
    cell_bounds: Vec<[[f32; 3]; 2]>,
    cell_link_ranges: Vec<(u32, u32)>,
    cell_candidate_ranges: Vec<(u32, u32)>,
    links_at: usize,
    candidates_at: usize,
    candidate_index_bits: usize,
    candidate_run_bits: usize,
    candidate_record_count: usize,
    object_count: usize,
    code_width: usize,
    portal_nodes_at: Option<usize>,
    portal_planes_at: Option<usize>,
}

/// Inputs installed for one FUN_140a82710 cell call by the outer query
/// scheduler. The caller supplies the cell's accumulated portal mask and the
/// record-local query state created by FUN_140a82340.
pub struct CellObjectVisibilityQuery<'a> {
    pub record_query_bounds: [[f32; 4]; 2],
    pub query_planes: &'a [[f32; 4]],
    pub active_planes: &'a [usize],
    pub projection_rows: [[f32; 4]; 4],
    pub camera_translation_imprecision: f32,
    pub object_distance_point: [f32; 3],
    pub object_distance_scale_squared: [f32; 4],
    pub minimum_area_metric: f32,
    pub viewport_tile_bounds: [u8; 4],
    pub portal: &'a PortalTileMask,
    pub depth_grid: Option<(&'a [f32], f32)>,
}

/// Immutable direct-Tome resource views shared by camera queries. The original
/// FUN_140a7c020 binds header-relative tables; it does not rebuild decoded
/// spatial/cell trees every frame. Keep decoding out of the render query too.
/// This data must be used with the same immutable bytes and SpatialTree.
pub struct DirectTomeQueryData {
    pub slot_rows: SpatialCellSlotRows,
    record_trees: Vec<Option<Result<CellTree, String>>>,
    object_count: usize,
    cell_count: usize,
}

impl DirectTomeQueryData {
    pub fn parse(bytes: &[u8], tree: &SpatialTree) -> Result<Self, String> {
        let slot_rows = SpatialCellSlotRows::parse(bytes)?;
        let object_count = word(bytes, 0x40)? as usize;
        // Errors in an unvisited record must not disable a formerly valid
        // query. Preserve them and report them only when that record is used.
        let record_trees = tree
            .offsets
            .iter()
            .map(|&offset| (offset != 0).then(|| CellTree::parse(bytes, offset)))
            .collect();
        Ok(Self {
            slot_rows,
            record_trees,
            object_count,
            cell_count: word(bytes, 0x7c)? as usize,
        })
    }

    pub fn cell_count(&self) -> usize {
        self.cell_count
    }

    /// FUN_140a7b410/FUN_140a7b4d0 compose the spatial and local point
    /// lookups, then read the direct global cell ID. Native UINT_MAX means
    /// outside/unlocated and must retain a light in the sphere batch query.
    pub fn point_cell_id(
        &self,
        bytes: &[u8],
        tree: &SpatialTree,
        point: [f32; 3],
    ) -> Result<Option<u32>, String> {
        let Some(spatial_index) = tree.leaf_for_point(point) else {
            return Ok(None);
        };
        if self
            .record_trees
            .get(spatial_index)
            .is_none_or(Option::is_none)
        {
            return Ok(None);
        }
        Ok(self
            .cells(spatial_index)?
            .cell_for_point(bytes, point)?
            .and_then(|cell| (cell.cell_id != u32::MAX).then_some(cell.cell_id)))
    }

    pub fn decoded_record_count(&self) -> usize {
        self.record_trees
            .iter()
            .filter(|record| record.is_some())
            .count()
    }

    fn cells(&self, spatial_index: usize) -> Result<&CellTree, String> {
        self.record_trees
            .get(spatial_index)
            .and_then(Option::as_ref)
            .ok_or_else(|| "active Tome spatial node has no record".to_owned())?
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// FUN_140a82710 limits each candidate-processing batch to 0x20 entries.
pub const ORIGINAL_OBJECT_QUERY_BATCH_SIZE: usize = 32;

/// Query-context capacities passed by FUN_140a73820 to FUN_140a81500.
/// FUN_140a81e20 allocates one 16-bit state per cell-state slot; FUN_140a85090
/// uses the node count for its 0x18-byte cell work-node pool.
pub const ORIGINAL_QUERY_WORK_NODE_COUNT: usize = 0x400;
pub const ORIGINAL_QUERY_CELL_STATE_COUNT: usize = 0x1000;
/// FUN_140a85af0 clears 0xffe bytes at query +0xc80 before its per-record
/// point-cell lookup, which is 0x7ff local u16 cell-state entries.
pub const ORIGINAL_RECORD_CELL_STATE_COUNT: usize = 0x7ff;

fn word(bytes: &[u8], at: usize) -> Result<u32, String> {
    let data = bytes
        .get(at..at.checked_add(4).ok_or("Tome offset overflow")?)
        .ok_or("truncated Tome")?;
    Ok(u32::from_le_bytes(data.try_into().unwrap()))
}

/// FUN_140a767e0 reads a direct header-relative object-ID table when the
/// alternative object mapping at +0x4c is absent.
pub fn object_ids(bytes: &[u8]) -> Result<Vec<u32>, String> {
    if word(bytes, 0x4c)? != 0 {
        return Err("unsupported Tome object mapping".into());
    }
    let count = word(bytes, 0x40)? as usize;
    let table = word(bytes, 0x50)? as usize;
    let end = table
        .checked_add(count.checked_mul(4).ok_or("Tome object count overflow")?)
        .ok_or("Tome object table overflow")?;
    if bytes.get(table..end).is_none() {
        return Err("truncated Tome object table".into());
    }
    (0..count)
        .map(|index| word(bytes, table + index * 4))
        .collect()
}

/// FUN_140a767e0/FUN_140a768d0 map one Tome object index to the authored
/// object IDs consumed by the world renderer. With header+0x4c present, it
/// indexes two offsets into the ID table at header+0x50; otherwise there is
/// one ID per Tome index. Empty groups are valid in the packed format.
pub fn object_ids_for_index(bytes: &[u8], index: u32) -> Result<Vec<u32>, String> {
    let count = word(bytes, 0x40)?;
    if index >= count {
        return Err("Tome object index out of range".into());
    }
    let ids_at = word(bytes, 0x50)? as usize;
    if ids_at == 0 {
        return Err("Tome object ID table is absent".into());
    }
    let groups_at = word(bytes, 0x4c)? as usize;
    let (start, end) = if groups_at == 0 {
        (index as usize, index as usize + 1)
    } else {
        let at = groups_at
            .checked_add(
                (index as usize)
                    .checked_mul(4)
                    .ok_or("Tome object-group index overflow")?,
            )
            .ok_or("Tome object-group offset overflow")?;
        (word(bytes, at)? as usize, word(bytes, at + 4)? as usize)
    };
    if end < start {
        return Err("Tome object-group offsets decrease".into());
    }
    let first = ids_at
        .checked_add(
            start
                .checked_mul(4)
                .ok_or("Tome object ID index overflow")?,
        )
        .ok_or("Tome object ID offset overflow")?;
    let last = ids_at
        .checked_add(end.checked_mul(4).ok_or("Tome object ID index overflow")?)
        .ok_or("Tome object ID offset overflow")?;
    if bytes.get(first..last).is_none() {
        return Err("truncated Tome object ID group".into());
    }
    (start..end).map(|i| word(bytes, ids_at + i * 4)).collect()
}

/// FUN_14031cf30 searches unsigned IDs and returns the last matching
/// index, including duplicates. The renderer tests only whether it exists.
pub fn sorted_authored_id_index(ids: &[u32], target: u32) -> Option<usize> {
    ids.partition_point(|&id| id <= target)
        .checked_sub(1)
        .filter(|&index| ids[index] == target)
}

/// FUN_140a73bf0 binds the header +0x44 table to the render query;
/// FUN_140a82710 reads six float bounds at Tome object index * 0x18.
pub fn object_bounds(bytes: &[u8], index: u32) -> Result<[[f32; 3]; 2], String> {
    let count = word(bytes, 0x40)?;
    if index >= count {
        return Err("Tome object index out of range".into());
    }
    let table = word(bytes, 0x44)? as usize;
    let at = table
        .checked_add(
            (index as usize)
                .checked_mul(0x18)
                .ok_or("Tome object bounds index overflow")?,
        )
        .ok_or("Tome object bounds offset overflow")?;
    let bounds = [
        [
            float(bytes, at)?,
            float(bytes, at + 4)?,
            float(bytes, at + 8)?,
        ],
        [
            float(bytes, at + 12)?,
            float(bytes, at + 16)?,
            float(bytes, at + 20)?,
        ],
    ];
    if (0..3).any(|axis| {
        !bounds[0][axis].is_finite()
            || !bounds[1][axis].is_finite()
            || bounds[0][axis] > bounds[1][axis]
    }) {
        return Err("invalid Tome object bounds".into());
    }
    Ok(bounds)
}

/// FUN_140a81500's null distance descriptor selects 1.0, squared and splatted
/// at query context +0x160. The Rust ordinary null-input path uses this value;
/// renderer-supplied distance descriptors/overrides remain unsupported.
pub const ORIGINAL_DEFAULT_OBJECT_DISTANCE_SCALE_SQUARED: f32 = 1.0;

/// FUN_140a81500 binds the optional header +0x48 distance table. Each object
/// has a 32-byte record: lower XYZ / squared near, upper XYZ / squared far.
/// Preserve raw floats: the executable's ordered SIMD comparisons, not format
/// validation or camera imprecision, determine acceptance of these records.
pub fn object_distance_bounds(bytes: &[u8], index: u32) -> Result<Option<[[f32; 4]; 2]>, String> {
    let table = word(bytes, 0x48)? as usize;
    if table == 0 {
        return Ok(None);
    }
    if index >= word(bytes, 0x40)? {
        return Err("Tome object distance index out of range".into());
    }
    let at = table
        .checked_add(
            (index as usize)
                .checked_mul(32)
                .ok_or("Tome object distance index overflow")?,
        )
        .ok_or("Tome object distance offset overflow")?;
    let end = at
        .checked_add(32)
        .ok_or("Tome object distance offset overflow")?;
    if bytes.get(at..end).is_none() {
        return Err("truncated Tome object distance record".into());
    }
    let mut record = [[0.0; 4]; 2];
    for (row, values) in record.iter_mut().enumerate() {
        for (axis, value) in values.iter_mut().enumerate() {
            *value = float(bytes, at + row * 16 + axis * 4)?;
        }
    }
    Ok(Some(record))
}

/// FUN_140a82710, 0x140a82de7..0x140a82e58. Clamp the query point to the
/// distance-record box, mask W, sum squared XYZ in the original SSE order,
/// then reject lanes where scaled distance < near or scaled distance >= far.
/// MINPS/MAXPS select their second operand on NaNs/equality; f32::min/max do
/// not reproduce that behavior. Far is exclusive and near is inclusive.
pub fn object_distance_rejection_mask(
    record: [[f32; 4]; 2],
    point: [f32; 3],
    scale_squared: [f32; 4],
) -> u32 {
    let squared: [f32; 3] = std::array::from_fn(|axis| {
        let nearest = if record[0][axis] > point[axis] {
            record[0][axis]
        } else {
            point[axis]
        };
        let nearest = if record[1][axis] < nearest {
            record[1][axis]
        } else {
            nearest
        };
        let delta = nearest - point[axis];
        delta * delta
    });
    let distance_squared = ((squared[1] + squared[0]) + squared[2]) + 0.0;
    let mut mask = 0;
    for (lane, scale) in scale_squared.into_iter().enumerate() {
        let distance = distance_squared * scale;
        if record[1][3] <= distance || distance < record[0][3] {
            mask |= 1 << lane;
        }
    }
    mask
}

/// FUN_140a82710 expands each Tome object's authored bounds by the query
/// offset installed at context +0x4c0. The world renderer supplies the
/// `umbra_distance` cvar, described by the executable as camera translation
/// imprecision; FUN_140a77560 stores its scalar value in all three axes.
pub fn expand_object_bounds_for_camera_imprecision(
    bounds: [[f32; 3]; 2],
    camera_translation_imprecision: f32,
) -> [[f32; 3]; 2] {
    [
        bounds[0].map(|value| value - camera_translation_imprecision),
        bounds[1].map(|value| value + camera_translation_imprecision),
    ]
}

/// Project an expanded Tome-object AABB through the query matrix, clipping
/// the box to the positive-W half-space as FUN_140a82710 does. The original
/// SIMD crossing branch evaluates the box corners and W=0 intersections on
/// its twelve edges, then forms the projected rectangle. A rectangle can be
/// unbounded when a nonzero X or Y coordinate reaches W=0; the later screen
/// clamp handles those infinities.
pub fn project_object_bounds_clipped_w(
    bounds: [[f32; 3]; 2],
    matrix: [[f32; 4]; 4],
) -> Result<Option<[f32; 4]>, String> {
    if bounds.iter().flatten().any(|value| !value.is_finite())
        || matrix.iter().flatten().any(|value| !value.is_finite())
        || (0..3).any(|axis| bounds[0][axis] > bounds[1][axis])
    {
        return Err("invalid Tome object bounds or query matrix".into());
    }
    let mut clips = [[0.0f32; 4]; 8];
    for corner in 0..8 {
        let point = [
            bounds[(corner >> 0) & 1][0],
            bounds[(corner >> 1) & 1][1],
            bounds[(corner >> 2) & 1][2],
            1.0,
        ];
        // The original SSE path scales the four stored matrix rows by x, y,
        // z, and 1, then sums those vectors. Keep the stored row order here;
        // transposing this multiply changes clip W and the camera-plane branch.
        clips[corner] = std::array::from_fn(|component| {
            (0..4)
                .map(|axis| matrix[axis][component] * point[axis])
                .sum::<f32>()
        });
        if clips[corner].iter().any(|value| !value.is_finite()) {
            return Err("nonfinite Tome object clip coordinate".into());
        }
    }

    let mut rectangle = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    let mut has_positive_w = false;
    for clip in clips {
        if clip[3] > 0.0 {
            has_positive_w = true;
            include_projected_point(&mut rectangle, clip[0] / clip[3], clip[1] / clip[3]);
        }
    }
    if !has_positive_w {
        return Ok(None);
    }

    // The original SIMD branch computes the same edge crossings in parallel.
    // A linear-fractional coordinate along an edge is monotone while W > 0;
    // its limit at W=0 is unbounded unless that coordinate is also zero.
    for corner in 0..8 {
        for axis in 0..3 {
            if corner & (1 << axis) != 0 {
                continue;
            }
            let other = corner | (1 << axis);
            let a = clips[corner];
            let b = clips[other];
            let wa = a[3];
            let wb = b[3];
            let crosses_into_positive = (wa < 0.0 && wb > 0.0)
                || (wb < 0.0 && wa > 0.0)
                || (wa == 0.0 && wb > 0.0)
                || (wb == 0.0 && wa > 0.0);
            if !crosses_into_positive {
                continue;
            }
            let delta_w = wb - wa;
            if delta_w == 0.0 {
                continue;
            }
            let t = -wa / delta_w;
            let x = a[0] + (b[0] - a[0]) * t;
            let y = a[1] + (b[1] - a[1]) * t;
            include_w_zero_limit(&mut rectangle, x, (b[0] - a[0]) / delta_w, 0);
            include_w_zero_limit(&mut rectangle, y, (b[1] - a[1]) / delta_w, 1);
        }
    }

    Ok(Some(rectangle))
}

fn include_projected_point(rectangle: &mut [f32; 4], x: f32, y: f32) {
    rectangle[0] = rectangle[0].min(x);
    rectangle[1] = rectangle[1].min(y);
    rectangle[2] = rectangle[2].max(x);
    rectangle[3] = rectangle[3].max(y);
}

fn include_w_zero_limit(rectangle: &mut [f32; 4], coordinate: f32, edge_limit: f32, axis: usize) {
    let (min_index, max_index) = if axis == 0 { (0, 2) } else { (1, 3) };
    if coordinate > 0.0 {
        rectangle[max_index] = f32::INFINITY;
    } else if coordinate < 0.0 {
        rectangle[min_index] = f32::NEG_INFINITY;
    } else {
        rectangle[min_index] = rectangle[min_index].min(edge_limit);
        rectangle[max_index] = rectangle[max_index].max(edge_limit);
    }
}

/// Convert an NDC rectangle into FUN_140a82710's query-pixel rectangle and
/// intersect it with the query viewport rectangle. The executable reads
/// `[left, top, right, bottom]` as bytes from the current 0x18-byte cell
/// work record at `query_context + 0x9a8 + cell_index*0x18 + 8`;
/// FUN_140a82160 copies those bytes from query-context `+0x180`, which
/// FUN_140a81500 packs from viewport bounds `+0x570..+0x57c`. Use
/// `query_viewport_tile_bounds` to produce those four bytes. This maps NDC
/// with `32*x + 32`, then scales horizontal tiles by 8 pixels and vertical
/// tiles by 4 pixels before intersection.
pub fn query_viewport_tile_bounds(viewport: [i32; 4]) -> [u8; 4] {
    let [left, top, right, bottom] = viewport;
    [
        (left >> 3) as u8,
        (top >> 2) as u8,
        (right.wrapping_add(7) >> 3) as u8,
        (bottom.wrapping_add(3) >> 2) as u8,
    ]
}

pub fn projected_object_pixel_rect(
    projected: [f32; 4],
    query_tile_bounds: [u8; 4],
) -> Result<[i32; 4], String> {
    if projected.iter().any(|value| value.is_nan()) {
        return Err("NaN projected Tome object rectangle".into());
    }
    let [left, top, right, bottom] = query_tile_bounds;
    if left > right || top > bottom {
        return Err("invalid Tome object query-tile rectangle".into());
    }
    let clipped = projected.map(|value| value.clamp(-1.0, 1.0));
    // Native right/bottom bias rounds coverage outward; truncating both
    // edges identically makes thin distant objects lose all query pixels.
    let pixel = [
        (clipped[0] * 32.0 + 32.0) as i32,
        (clipped[1] * 32.0 + 32.0) as i32,
        (clipped[2] * 32.0 + f32::from_bits(0x4203_ffff)) as i32,
        (clipped[3] * 32.0 + f32::from_bits(0x4203_ffff)) as i32,
    ];
    Ok([
        pixel[0].max(i32::from(left) * 8),
        pixel[1].max(i32::from(top) * 4),
        pixel[2].min(i32::from(right) * 8),
        pixel[3].min(i32::from(bottom) * 4),
    ])
}

/// FUN_140a82710's per-accepted-object metric for the world visibility
/// output at +0x58. It clips the projected screen rectangle to [-1, 1]
/// and multiplies its area by DAT_141abf150 (0.25). The world-material
/// collector submits the maximum metric for a material to FUN_14031a510.
/// `projected` is [minimum x, minimum y, maximum x, maximum y] after the
/// original query's object projection and portal tests.
pub fn projected_object_area_metric(projected: [f32; 4]) -> f32 {
    let clipped = projected.map(|value| value.min(1.0).max(-1.0));
    (clipped[3] - clipped[1]) * (clipped[2] - clipped[0]) * 0.25
}

/// FUN_14031c710 sets the ordinary render query's area cutoff to
/// `25.0 / (viewport_width * viewport_height)`. Combined with
/// `projected_object_area_metric`, this rejects projected rectangles smaller
/// than 25 viewport pixels in FUN_140a82710.
pub fn minimum_projected_area_metric(
    viewport_width: i32,
    viewport_height: i32,
) -> Result<f32, String> {
    if viewport_width <= 0 || viewport_height <= 0 {
        return Err("invalid Tome query viewport dimensions".into());
    }
    let viewport_pixels = viewport_width
        .checked_mul(viewport_height)
        .ok_or("Tome query viewport pixel count overflow")?;
    Ok(25.0 / viewport_pixels as f32)
}

/// FUN_140a82710's optional, deduplicated visible-cell output. Its bitset
/// domain is the Tome header's +0x7c count, not the object count at +0x40.
/// The logical count keeps advancing after the caller's buffer fills;
/// FUN_140a71f20 exposes only min(count, capacity) stored entries.
#[derive(Debug)]
pub struct VisibleCellIndexOutput {
    seen: Vec<u32>,
    indices: Vec<u32>,
    cell_count: usize,
    capacity: usize,
    count: usize,
}

impl VisibleCellIndexOutput {
    pub fn new(cell_count: usize, capacity: usize) -> Self {
        Self {
            seen: vec![0; cell_count.saturating_add(31) / 32],
            indices: Vec::with_capacity(capacity.min(cell_count)),
            cell_count,
            capacity,
            count: 0,
        }
    }

    pub fn push_unique(&mut self, index: u32) -> Result<bool, String> {
        if index as usize >= self.cell_count {
            return Err("visible Tome cell index exceeds output bitset".into());
        }
        let slot = &mut self.seen[index as usize / 32];
        let bit = 1u32 << (index % 32);
        if *slot & bit != 0 {
            return Ok(false);
        }
        *slot |= bit;
        if self.count < self.capacity {
            self.indices.push(index);
        }
        self.count = self
            .count
            .checked_add(1)
            .ok_or("visible Tome cell count overflow")?;
        Ok(true)
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn count(&self) -> usize {
        self.count
    }
}

/// FUN_140a70570: allocate/zero ceil(cell_count/32) words and set one bit
/// for each stored visible-cell index. The Rust boundary rejects invalid
/// indices instead of reproducing an out-of-bounds native write.
pub fn cell_visibility_mask(cell_count: usize, indices: &[u32]) -> Result<Vec<u32>, String> {
    let word_count = cell_count
        .checked_add(31)
        .ok_or("Tome cell-mask size overflow")?
        / 32;
    let mut mask = vec![0; word_count];
    for &index in indices {
        if index as usize >= cell_count {
            return Err("visible Tome cell index exceeds mask domain".into());
        }
        mask[index as usize / 32] |= 1 << (index % 32);
    }
    Ok(mask)
}

/// The original render query's deduplicated Tome-object index output.
/// FUN_140a82710 checks the query bitset before adding an index to the
/// caller's bounded int buffer. Its output count advances for every unique
/// index, including when the caller's buffer is already full.
#[derive(Debug)]
pub struct VisibleObjectIndexOutput {
    seen: Vec<u32>,
    indices: Vec<u32>,
    area_metrics: Vec<Option<f32>>,
    object_count: usize,
    capacity: usize,
    count: usize,
    distance_rejected_objects: usize,
    cells: Option<VisibleCellIndexOutput>,
}

impl VisibleObjectIndexOutput {
    /// `object_count` sizes the original per-query deduplication bitset;
    /// `capacity` is the caller-owned index buffer capacity.
    pub fn new(object_count: usize, capacity: usize) -> Self {
        Self {
            seen: vec![0; object_count.saturating_add(31) / 32],
            indices: Vec::with_capacity(capacity.min(object_count)),
            area_metrics: Vec::with_capacity(capacity.min(object_count)),
            object_count,
            capacity,
            count: 0,
            distance_rejected_objects: 0,
            cells: None,
        }
    }

    pub fn cell_output(&self) -> Option<&VisibleCellIndexOutput> {
        self.cells.as_ref()
    }

    /// Record one index that has already passed the original visibility
    /// predicates. Returns true only on its first occurrence.
    pub fn push_unique(&mut self, index: u32) -> Result<bool, String> {
        self.push_unique_with_area_metric(index, None)
    }

    /// Whether the query has accepted or permanently distance-rejected this
    /// object. FUN_140a82710 checks this bit before loading distance/geometry
    /// bounds. Only distance rejection marks a failed visibility test: portal
    /// or projection rejection must remain eligible in another cell.
    pub fn contains(&self, index: u32) -> Result<bool, String> {
        if index as usize >= self.object_count {
            return Err("visible Tome object index exceeds output bitset".into());
        }
        let slot = self
            .seen
            .get(index as usize / 32)
            .ok_or("visible Tome object index exceeds output bitset")?;
        Ok(*slot & (1u32 << (index % 32)) != 0)
    }

    fn push_unique_with_area_metric(
        &mut self,
        index: u32,
        area_metric: Option<f32>,
    ) -> Result<bool, String> {
        if index as usize >= self.object_count {
            return Err("visible Tome object index exceeds output bitset".into());
        }
        let slot = self
            .seen
            .get_mut(index as usize / 32)
            .ok_or("visible Tome object index exceeds output bitset")?;
        let bit = 1u32 << (index % 32);
        if *slot & bit != 0 {
            return Ok(false);
        }
        *slot |= bit;
        if self.count < self.capacity {
            self.indices.push(index);
            self.area_metrics.push(area_metric);
        }
        self.count = self
            .count
            .checked_add(1)
            .ok_or("visible Tome object count overflow")?;
        Ok(true)
    }

    /// Apply FUN_140a82710's final projected-rectangle visibility branch,
    /// then append the Tome index only when the original portal predicate
    /// accepts it. The original selects the portal-only test when the query
    /// has no depth grid and the combined portal/depth test when it does.
    pub fn push_if_rectangle_visible(
        &mut self,
        index: u32,
        portal: &PortalTileMask,
        pixels: [i32; 4],
        depth_grid: Option<(&[f32], f32)>,
    ) -> Result<bool, String> {
        self.push_if_rectangle_visible_with_area_metric(index, portal, pixels, depth_grid, None)
    }

    fn push_if_rectangle_visible_with_area_metric(
        &mut self,
        index: u32,
        portal: &PortalTileMask,
        pixels: [i32; 4],
        depth_grid: Option<(&[f32], f32)>,
        area_metric: Option<f32>,
    ) -> Result<bool, String> {
        if self.contains(index)? {
            return Ok(false);
        }
        let accepted = match depth_grid {
            Some((depths, threshold)) => {
                portal.intersects_depth_pixels(depths, pixels, threshold)?
            }
            None => portal.intersects_pixels(pixels)?,
        };
        if !accepted {
            return Ok(false);
        }
        self.push_unique_with_area_metric(index, area_metric)
    }

    /// Apply FUN_140a82710's final projected-object stages when its earlier
    /// candidate, bounds, and projection checks have already produced NDC
    /// bounds. The original applies the area cutoff before pixel clipping and
    /// portal coverage, then emits a unique Tome index and its area metric.
    /// Returns the metric only when this call adds a new visible index.
    pub fn push_if_projected_object_visible(
        &mut self,
        index: u32,
        projected: [f32; 4],
        minimum_area_metric: f32,
        query_tile_bounds: [u8; 4],
        portal: &PortalTileMask,
        depth_grid: Option<(&[f32], f32)>,
    ) -> Result<Option<f32>, String> {
        if self.contains(index)? {
            return Ok(None);
        }
        let area_metric = projected_object_area_metric(projected);
        if minimum_area_metric > area_metric {
            return Ok(None);
        }
        let pixels = projected_object_pixel_rect(projected, query_tile_bounds)?;
        // FUN_140a82710 skips the candidate when either clipped pixel axis
        // has no positive extent, before calling its portal/depth test.
        if pixels[0] >= pixels[2] || pixels[1] >= pixels[3] {
            return Ok(None);
        }
        if self.push_if_rectangle_visible_with_area_metric(
            index,
            portal,
            pixels,
            depth_grid,
            Some(area_metric),
        )? {
            Ok(Some(area_metric))
        } else {
            Ok(None)
        }
    }

    /// Stored indices, limited by the caller's original output capacity.
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Per-index projected-area values for indices stored within the bounded
    /// output capacity. `None` means the caller used the rectangle-only path.
    pub fn area_metrics(&self) -> &[Option<f32>] {
        &self.area_metrics
    }

    /// Number of unique accepted indices, even if the output buffer filled.
    pub fn count(&self) -> usize {
        self.count
    }
}

fn float(bytes: &[u8], at: usize) -> Result<f32, String> {
    Ok(f32::from_bits(word(bytes, at)?))
}

/// Build FUN_140a82340's per-spatial-record query AABB. The query expands the
/// authored record bounds by the record's +0x30 padding and the query context's
/// camera-translation-imprecision vector at +0x4c0. This is the world-space
/// box passed to active query-plane selection; it is distinct from the global
/// camera matrix used by the viewer's diagnostic.
pub fn spatial_record_query_bounds(
    bytes: &[u8],
    record_offset: u32,
    camera_translation_imprecision: [f32; 3],
) -> Result<[[f32; 4]; 2], String> {
    let offset = record_offset as usize;
    let bounds = recorded_bounds(bytes, offset)?;
    let padding = float(bytes, offset + 0x30)?;
    if !padding.is_finite()
        || camera_translation_imprecision
            .iter()
            .any(|value| !value.is_finite())
    {
        return Err("invalid Tome spatial-record query expansion".into());
    }
    let expanded = [
        [
            bounds[0][0] - (padding + camera_translation_imprecision[0]),
            bounds[0][1] - (padding + camera_translation_imprecision[1]),
            bounds[0][2] - (padding + camera_translation_imprecision[2]),
            1.0,
        ],
        [
            bounds[1][0] + (padding + camera_translation_imprecision[0]),
            bounds[1][1] + (padding + camera_translation_imprecision[1]),
            bounds[1][2] + (padding + camera_translation_imprecision[2]),
            1.0,
        ],
    ];
    if expanded.iter().flatten().any(|value| !value.is_finite()) {
        return Err("nonfinite Tome spatial-record query bounds".into());
    }
    Ok(expanded)
}

/// Record-local state assembled by FUN_140a82340 before cell traversal.
/// `query_planes` are supplied by the caller after constructing the current
/// render-query matrix; this helper performs the record-specific setup.
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialRecordQuerySetup {
    pub bounds: [[f32; 4]; 2],
    pub link_center: [i32; 4],
    pub link_tolerance: [i32; 4],
    pub active_planes: Vec<usize>,
}

pub fn setup_spatial_record_query(
    bytes: &[u8],
    record_offset: u32,
    query_position: [f32; 3],
    camera_translation_imprecision: [f32; 3],
    query_planes: &[[f32; 4]],
) -> Result<SpatialRecordQuerySetup, String> {
    if query_planes
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
    {
        return Err("nonfinite Tome spatial-record query plane".into());
    }
    let bounds = spatial_record_query_bounds(bytes, record_offset, camera_translation_imprecision)?;
    let (link_center, link_tolerance) =
        direct_link_query_quantization(bytes, record_offset, query_position)?;
    let active_planes = active_query_planes(query_planes, bounds);
    Ok(SpatialRecordQuerySetup {
        bounds,
        link_center,
        link_tolerance,
        active_planes,
    })
}

/// FUN_140a82340's direct-Tome link-query coordinates. FUN_140a72df0
/// initializes FUN_140a77560 with a zero offset, so the record's +0x30
/// float supplies the per-axis padding. The original x86 code uses RCPPS
/// and one Newton step before CVTTPS2DQ; preserve those operations here.
pub fn direct_link_query_quantization(
    bytes: &[u8],
    record_offset: u32,
    query_position: [f32; 3],
) -> Result<([i32; 4], [i32; 4]), String> {
    let base = record_offset as usize;
    let bounds = recorded_bounds(bytes, base)?;
    let padding = float(bytes, base + 0x30)?;
    if !padding.is_finite() || query_position.iter().any(|value| !value.is_finite()) {
        return Err("invalid Tome link-query coordinates".into());
    }
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // x86_64 guarantees SSE. Each lane follows the original packed
        // instructions; the fourth homogeneous lane has zero width.
        unsafe {
            let low = _mm_set_ps(1.0, bounds[0][2], bounds[0][1], bounds[0][0]);
            let high = _mm_set_ps(1.0, bounds[1][2], bounds[1][1], bounds[1][0]);
            let width = _mm_mul_ps(_mm_sub_ps(high, low), _mm_set1_ps(1.0 / 65535.0));
            let zero = _mm_setzero_ps();
            let one = _mm_set1_ps(1.0);
            let empty = _mm_cmpeq_ps(width, zero);
            let safe_width = _mm_or_ps(_mm_andnot_ps(empty, width), _mm_and_ps(empty, one));
            let rough = _mm_rcp_ps(safe_width);
            let corrected = _mm_sub_ps(
                _mm_add_ps(rough, rough),
                _mm_mul_ps(_mm_mul_ps(width, rough), rough),
            );
            let position = _mm_set_ps(0.0, query_position[2], query_position[1], query_position[0]);
            let margin = _mm_set_ps(0.0, padding, padding, padding);
            let center = _mm_cvttps_epi32(_mm_mul_ps(_mm_sub_ps(position, low), corrected));
            let tolerance = _mm_cvttps_epi32(_mm_add_ps(_mm_mul_ps(corrected, margin), one));
            let mut center_values = [0i32; 4];
            let mut tolerance_values = [0i32; 4];
            _mm_storeu_si128(center_values.as_mut_ptr().cast(), center);
            _mm_storeu_si128(tolerance_values.as_mut_ptr().cast(), tolerance);
            Ok((center_values, tolerance_values))
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        Err("original Tome link quantization requires x86_64 SSE".into())
    }
}

/// FUN_140a77240: depth bit 2, horizontal bit 1, vertical bit 0.
/// Retain the scalar multiply/add and reciprocal order of the instructions.
pub fn query_frustum_corner(planes: &[[f32; 4]; 6], index: usize) -> Result<[f32; 3], String> {
    if index >= 8 || planes.iter().flatten().any(|v| !v.is_finite()) {
        return Err("invalid Tome query corner planes or index".into());
    }
    let [a, b, c] = [
        planes[index >> 2],
        planes[2 + ((index >> 1) & 1)],
        planes[4 + (index & 1)],
    ];
    let cross = |a: [f32; 4], b: [f32; 4]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let bc = cross(b, c);
    let ca = cross(c, a);
    let ab = cross(a, b);
    let determinant = bc[1] * a[1] + bc[0] * a[0] + bc[2] * a[2];
    if determinant == 0.0 || !determinant.is_finite() {
        return Err("Tome query corner planes do not intersect at a finite point".into());
    }
    let reciprocal = 1.0 / determinant;
    let point = std::array::from_fn(|axis| {
        reciprocal * -(bc[axis] * a[3] + ca[axis] * b[3] + ab[axis] * c[3])
    });
    if point.iter().any(|v: &f32| !v.is_finite()) {
        return Err("Tome query corner is not finite".into());
    }
    Ok(point)
}

/// FUN_140a774a0 orders the first-depth-plane corners as 0, 1, 3, 2.
pub fn first_depth_quad(planes: [[f32; 4]; 6]) -> Result<[[f32; 3]; 4], String> {
    Ok([
        query_frustum_corner(&planes, 0)?,
        query_frustum_corner(&planes, 1)?,
        query_frustum_corner(&planes, 3)?,
        query_frustum_corner(&planes, 2)?,
    ])
}

/// FUN_140a77c00's default six-plane bounds, after near-plane normalization.
pub fn query_bounds_from_planes(
    planes: &[[f32; 4]; 6],
    imprecision: [f32; 3],
) -> Result<[[f32; 4]; 2], String> {
    if planes
        .iter()
        .flatten()
        .chain(imprecision.iter())
        .any(|v| !v.is_finite())
    {
        return Err("nonfinite Tome query bounds descriptor".into());
    }
    let [near, far] = [planes[0], planes[1]];
    let far_length = (far[1] * far[1] + far[0] * far[0] + far[2] * far[2]).sqrt();
    let opposite_depth_planes = near[1] * far[1] + near[0] * far[0] + near[2] * far[2] < 0.0;
    let [x, y, z] = imprecision;
    if x * x + y * y + z * z > 0.0 || far_length <= 0.0 || !opposite_depth_planes {
        return Ok([[-f32::MAX; 4], [f32::MAX; 4]]);
    }
    let mut low = [f32::MAX; 4];
    let mut high = [-f32::MAX; 4];
    for index in 0..8 {
        let point = query_frustum_corner(planes, index)?;
        for axis in 0..3 {
            if point[axis] < low[axis] {
                low[axis] = point[axis];
            }
            if high[axis] < point[axis] {
                high[axis] = point[axis];
            }
        }
    }
    low[3] = 1.0;
    high[3] = 1.0;
    for axis in 0..4 {
        let margin = (high[axis] - low[axis]) * f32::from_bits(0x38d1_b717);
        let mask = if far[axis] > 0.0 { u32::MAX } else { 0 };
        low[axis] -= f32::from_bits(margin.to_bits() & mask);
        high[axis] += f32::from_bits(margin.to_bits() & !mask);
    }
    if low.iter().chain(high.iter()).any(|v| !v.is_finite()) {
        return Err("nonfinite Tome finite query bounds".into());
    }
    Ok([low, high])
}

/// FUN_140a77560 derives the two depth planes from matrix rows 2 and 3;
/// FUN_140a77c00 derives the four side planes from rows 0, 1 and 3 of
/// its crop-adjusted query matrix. Float4 rows match the original layout.
pub fn query_planes_from_rows(depth: [[f32; 4]; 4], sides: [[f32; 4]; 4]) -> [[f32; 4]; 6] {
    let combine = |a: [f32; 4], b: [f32; 4], sign: f32| -> [f32; 4] {
        std::array::from_fn(|axis| a[axis] + sign * b[axis])
    };
    [
        depth[2],
        combine(depth[3], depth[2], -1.0),
        combine(sides[3], sides[0], -1.0),
        combine(sides[3], sides[0], 1.0),
        combine(sides[3], sides[1], -1.0),
        combine(sides[3], sides[1], 1.0),
    ]
}

/// FUN_140a76ac0 selects query planes that can intersect a box. The sign
/// masks at query+0x340 are generated from positive plane components by
/// FUN_140a77b10; the original SIMD bit-select uses the lower bound for those
/// lanes and the upper bound otherwise, yielding the minimum plane distance.
/// A retained plane is passed to later clipping; this alone does not establish
/// visibility.
pub fn active_query_planes(planes: &[[f32; 4]], bounds: [[f32; 4]; 2]) -> Vec<usize> {
    planes
        .iter()
        .enumerate()
        .filter_map(|(index, plane)| {
            let corner: [f32; 4] = std::array::from_fn(|axis| {
                if plane[axis] > 0.0 {
                    bounds[0][axis]
                } else {
                    bounds[1][axis]
                }
            });
            // The original MULPS/SHUFPS/ADDPS sequence adds x+y, then z,
            // then w. Keep that order for the zero-boundary comparison.
            let products: [f32; 4] = std::array::from_fn(|axis| corner[axis] * plane[axis]);
            let distance = ((products[0] + products[1]) + products[2]) + products[3];
            (distance <= 0.0).then_some(index)
        })
        .collect()
}

/// FUN_140a71b90: reject a box outside the query bounds or the maximum-distance
/// corner of an active plane. No active planes accepts without the bounds test.
/// Keep the original four-plane MINPS reduction, including unordered operands.
pub fn query_bounds_accepts_aabb(
    query_bounds: [[f32; 4]; 2],
    input: [[f32; 4]; 2],
    planes: &[[f32; 4]],
    active: &[usize],
) -> Result<bool, String> {
    if active.is_empty() {
        return Ok(true);
    }
    if active.iter().any(|&index| index >= planes.len()) {
        return Err("invalid Tome query-bounds plane index".into());
    }
    let distance = |index: usize| {
        let plane = planes[index];
        let terms: [f32; 4] = std::array::from_fn(|axis| {
            let corner = if plane[axis] > 0.0 {
                input[1][axis]
            } else {
                input[0][axis]
            };
            corner * plane[axis]
        });
        ((terms[1] + terms[0]) + terms[2]) + terms[3]
    };
    let minps = |a: f32, b: f32| if a < b { a } else { b };
    let mut minimum = 0.0;
    let mut groups = active.chunks_exact(4);
    for group in &mut groups {
        let first = minps(distance(group[0]), distance(group[1]));
        let second = minps(distance(group[2]), distance(group[3]));
        minimum = minps(minimum, minps(first, second));
    }
    for &index in groups.remainder() {
        minimum = minps(minimum, distance(index));
    }
    Ok(!(minimum < 0.0
        || (0..4).any(|axis| {
            query_bounds[1][axis] < input[0][axis] || input[1][axis] < query_bounds[0][axis]
        })))
}

/// FUN_140a76bd0 clips a four-lane box against the selected query planes
/// after FUN_140a76ac0. Plane indices 0 and 1 are depth planes and are
/// skipped by this helper. The original visits eight xyz corners and the
/// three edges incident to each rejected corner. It falls back to the
/// query-bounds-clamped input when no ordered clipped bounds result.
#[cfg(target_arch = "x86_64")]
pub fn clip_box_to_active_query_planes(
    query_bounds: [[f32; 4]; 2],
    input: [[f32; 4]; 2],
    planes: &[[f32; 4]],
    active: &[usize],
) -> Result<[[f32; 4]; 2], String> {
    use std::arch::x86_64::*;
    if input
        .iter()
        .flatten()
        .chain(query_bounds.iter().flatten())
        .any(|value| !value.is_finite())
        || active.iter().any(|&index| index >= planes.len())
    {
        return Err("invalid Tome box clip input".into());
    }
    let low = std::array::from_fn(|axis| input[0][axis].max(query_bounds[0][axis]));
    let high = std::array::from_fn(|axis| input[1][axis].min(query_bounds[1][axis]));
    let selected = active
        .iter()
        .copied()
        .filter(|&index| index >= 2)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok([low, high]);
    }
    let corners: [[f32; 4]; 8] = std::array::from_fn(|index| {
        std::array::from_fn(|axis| {
            if axis < 3 && index & (1 << axis) != 0 {
                high[axis]
            } else {
                low[axis]
            }
        })
    });
    let distance = |corner: [f32; 4], plane: [f32; 4]| -> f32 {
        let terms: [f32; 4] = std::array::from_fn(|axis| corner[axis] * plane[axis]);
        ((terms[1] + terms[0]) + terms[2]) + terms[3]
    };
    let distances = corners.map(|corner| {
        selected
            .iter()
            .map(|&index| distance(corner, planes[index]))
            .collect::<Vec<_>>()
    });
    let mut clipped_low = [f32::MAX; 4];
    let mut clipped_high = [-f32::MAX; 4];
    let mut include = |corner: [f32; 4]| {
        for axis in 0..4 {
            clipped_low[axis] = clipped_low[axis].min(corner[axis]);
            clipped_high[axis] = clipped_high[axis].max(corner[axis]);
        }
    };
    for current in 0..8 {
        if distances[current].iter().all(|&value| value >= 0.0) {
            include(corners[current]);
            continue;
        }
        for axis in 0..3 {
            let neighbor = current ^ (1 << axis);
            let mut fraction = 1.0f32;
            for plane in 0..selected.len() {
                let from = distances[neighbor][plane];
                let to = distances[current][plane];
                if from > 0.0 && to <= 0.0 {
                    let denominator = from - to;
                    let reciprocal = unsafe { _mm_cvtss_f32(_mm_rcp_ps(_mm_set1_ps(denominator))) };
                    fraction = fraction.min(reciprocal * from);
                }
            }
            if fraction < 1.0 {
                include(std::array::from_fn(|lane| {
                    corners[neighbor][lane]
                        + (corners[current][lane] - corners[neighbor][lane]) * fraction
                }));
            }
        }
    }
    if (0..4).any(|axis| clipped_high[axis] < clipped_low[axis]) {
        Ok([low, high])
    } else {
        Ok([clipped_low, clipped_high])
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn clip_box_to_active_query_planes(
    _: [[f32; 4]; 2],
    _: [[f32; 4]; 2],
    _: &[[f32; 4]],
    _: &[usize],
) -> Result<[[f32; 4]; 2], String> {
    Err("original Tome box clipping requires x86_64 SSE".into())
}

/// FUN_140a77c00 constructs a crop matrix from integer tile bounds on the
/// original 64-unit viewport grid. FUN_1411f6e10/FUN_1411f6ea0 apply it
/// as crop * base to the row-major query matrix.
pub fn crop_query_rows(base: [[f32; 4]; 4], tile: [i32; 4]) -> Result<[[f32; 4]; 4], String> {
    let [left, top, right, bottom] = tile;
    if left < 0 || top < 0 || right > 64 || bottom > 64 || right <= left || bottom <= top {
        return Err("invalid Tome viewport tile".into());
    }
    let sx = 64.0 / (right - left) as f32;
    let sy = 64.0 / (bottom - top) as f32;
    let ox = sx - 1.0 - (left as f32 / 64.0) * 2.0 * sx;
    let oy = sy - 1.0 - (top as f32 / 64.0) * 2.0 * sy;
    let crop = [
        [sx, 0.0, 0.0, ox],
        [0.0, sy, 0.0, oy],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    Ok(std::array::from_fn(|row| {
        std::array::from_fn(|column| {
            (0..4)
                .map(|axis| crop[row][axis] * base[axis][column])
                .sum()
        })
    }))
}

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
pub fn original_render_query_matrix(
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
    let (horizontal_fov, vertical_fov) = original_render_fov_radians(
        horizontal_reference_degrees,
        render_width,
        render_height,
        pixel_aspect,
    )?;
    let right = near * (horizontal_fov * 0.5).tan();
    let top = near * (vertical_fov * 0.5).tan();
    let projection = original_render_perspective_rows([-right, right], [-top, top], near, far)?;
    Ok(render_query_base_rows(projection, camera_pose))
}

/// FUN_14031c710 copies the renderer projection, then replaces its depth
/// coefficients before submitting the flags-zero world visibility query.
/// The render projection is reverse depth, while this query uses near=0 and
/// far=1 in clip space. The replacement also reverses the 4x4 determinant
/// sign used by FUN_140a77560 to choose portal winding.
pub fn original_world_visibility_query_matrix(
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
    let (horizontal_fov, vertical_fov) = original_render_fov_radians(
        horizontal_reference_degrees,
        render_width,
        render_height,
        pixel_aspect,
    )?;
    let right = near * (horizontal_fov * 0.5).tan();
    let top = near * (vertical_fov * 0.5).tan();
    let mut projection = original_render_perspective_rows([-right, right], [-top, top], near, far)?;
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
pub fn original_render_perspective_rows(
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
pub fn original_render_fov_radians(
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

    let vertical = original_render_vertical_fov_radians(horizontal_reference_degrees)?;
    let aspect = (render_width as f32 * pixel_aspect) / render_height as f32;
    let horizontal = 2.0 * ((vertical * 0.5).tan() * aspect).atan();
    Ok((horizontal, vertical))
}

/// FUN_140890b50's vertical-FOV conversion from the horizontal reference CVar.
pub fn original_render_vertical_fov_radians(
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
pub const ORIGINAL_DEFAULT_QUERY_JOB_COUNT: u32 = 12;
pub const ORIGINAL_DEFAULT_SCHEDULER_THREADING_MODE: u32 = 2;
/// FUN_14037e1d0's world-object buffer, separate from its visible-cell buffer.
pub const ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY: usize = 0x10000;
/// FUN_14037e1d0 supplies this second list to the later light query.
pub const ORIGINAL_VISIBLE_CELL_OUTPUT_CAPACITY: usize = 0x8000;

/// FUN_14037ef90's ordinary world-query branch (caller param_12 = 1).
/// The CVar permits 1..32; dispatch caps it at 16 and mode 0 forces one job.
pub fn original_world_query_job_count(configured: u32, threading_mode: u32) -> Result<u32, String> {
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
fn segment_intersects_aabb(bounds: [[f32; 3]; 2], segment: [[f32; 3]; 2]) -> bool {
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
fn segment_intersects_quad(segment: [[f32; 3]; 2], quad: [[f32; 3]; 4]) -> bool {
    fn side(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) -> bool {
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
            if c <= -e {
                e * 2.0 + c
            } else {
                (-e / c) * e
            }
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

/// Gate indexes referenced by a traversal record with bit 27 set.
/// FUN_140a7b9f0 reads a 12-bit count and 20-bit start from word 2, then
/// checks the selected Tome's header-relative +0x78 gate-index table.
/// Collection remapping is not covered by this direct single-Tome decoder.
pub fn gate_indices_for_link(bytes: &[u8], link: [u32; 4]) -> Result<Vec<u32>, String> {
    if link[0] & (1 << 27) == 0 {
        return Ok(Vec::new());
    }
    let start = (link[2] >> 12) as usize;
    let count = (link[2] & 0xfff) as usize;
    let entries = word(bytes, 0x74)? as usize;
    let table = word(bytes, 0x78)? as usize;
    let gate_count = word(bytes, 0x68)?;
    if start.checked_add(count).is_none_or(|end| end > entries) {
        return Err("Tome link gate range exceeds index table".into());
    }
    let mut result = Vec::with_capacity(count);
    for index in start..start + count {
        let at = table
            .checked_add(
                index
                    .checked_mul(4)
                    .ok_or("Tome gate index offset overflow")?,
            )
            .ok_or("Tome gate index offset overflow")?;
        let gate = word(bytes, at)?;
        if gate >= gate_count {
            return Err("Tome link gate index exceeds gate table".into());
        }
        result.push(gate);
    }
    Ok(result)
}

/// FUN_140a7b9f0 permits a gated link only when every indexed gate bit is
/// set. A missing gate-state bitset is accepted by the original function.
pub fn gate_link_allows(
    bytes: &[u8],
    link: [u32; 4],
    states: Option<&[u32]>,
) -> Result<bool, String> {
    let Some(states) = states else {
        return Ok(true);
    };
    for gate in gate_indices_for_link(bytes, link)? {
        let word = states
            .get((gate / 32) as usize)
            .ok_or("Tome gate-state bitset is too short")?;
        if word & (1 << (gate % 32)) == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

/// FUN_140a85bd0 tests bit 28 before treating the low 26 bits as a
/// spatial-record target. Bit 27 controls its separate gate predicate.
/// A collection Tome may remap this index before use; this is the direct
/// single-Tome value stored in the link.
pub fn direct_spatial_link_target(link: [u32; 4]) -> Option<u32> {
    (link[0] & (1 << 28) == 0).then_some(link[0] & 0x03ff_ffff)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectLinkDestination {
    pub spatial_index: usize,
    pub record_offset: u32,
    pub cell_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkDestination {
    Spatial(u32),
    BoundaryFace(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedLinkGeometry {
    pub destination: LinkDestination,
    pub axis: u8,
    pub positive_side: bool,
    pub plane: u16,
    /// Packed lower and upper coordinates on the next two cyclic axes.
    pub spans: [[u16; 2]; 2],
}

/// FUN_140a85bd0 reconstructs ungated portal rectangles from word1 low16
/// and the high/low halves of words2 and 3. Bit 28 changes the destination
/// from a spatial record to one of six boundary faces; it does not change
/// this geometry layout. Bit-27 links use a separate float geometry table.
pub fn packed_link_geometry(link: [u32; 4]) -> Option<PackedLinkGeometry> {
    if link[0] & (1 << 27) != 0 {
        return None;
    }
    let code = (link[0] >> 29) as u8;
    Some(PackedLinkGeometry {
        destination: match direct_spatial_link_target(link) {
            Some(target) => LinkDestination::Spatial(target),
            None => LinkDestination::BoundaryFace(code),
        },
        axis: (link[0] >> 30) as u8,
        positive_side: link[0] & (1 << 29) != 0,
        plane: link[1] as u16,
        spans: [
            [(link[2] >> 16) as u16, link[2] as u16],
            [(link[3] >> 16) as u16, link[3] as u16],
        ],
    })
}

/// FUN_140a85bd0's ungated packed-link direction test for a volume query.
/// The original query state stores four quantized center coordinates at
/// +0xb10 and four tolerances at +0xb20. Gated links use a gate predicate
/// instead of this first direction check.
pub fn packed_link_faces_volume(
    geometry: PackedLinkGeometry,
    query_center: [i32; 4],
    query_tolerance: [i32; 4],
) -> bool {
    let axis = geometry.axis as usize;
    let delta = query_center[axis].wrapping_sub(i32::from(geometry.plane));
    let directed = if geometry.positive_side {
        delta
    } else {
        delta.wrapping_neg()
    };
    directed <= query_tolerance[axis]
}

/// FUN_140a85bd0 uses the query's stored axis-side bits at +0x14c when
/// its +0x561 point-mode flag is set, instead of the quantized volume test.
pub fn packed_link_faces_point(geometry: PackedLinkGeometry, axis_sides: [bool; 4]) -> bool {
    geometry.positive_side == axis_sides[geometry.axis as usize]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkQueryDirection {
    Volume {
        center: [i32; 4],
        tolerance: [i32; 4],
    },
    Point {
        axis_sides: [bool; 4],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalLinkPrecheck {
    PackedDirection { faces_query: bool },
    Gates { open: bool },
}

impl TraversalLinkPrecheck {
    pub fn allows_traversal(self) -> bool {
        match self {
            Self::PackedDirection { faces_query } => faces_query,
            Self::Gates { open } => open,
        }
    }
}

/// Apply FUN_140a85bd0's first traversal-link branch. Bit-27 links go
/// through FUN_140a7b9f0's gate check; other links use the packed face
/// predicate selected by the query's point/volume mode. This is only the
/// first precheck: accepted links still require the original portal clipping,
/// raster coverage and visited-cell tests before they can enqueue a cell.
pub fn traversal_link_precheck(
    bytes: &[u8],
    link: [u32; 4],
    direction: LinkQueryDirection,
    gate_states: Option<&[u32]>,
) -> Result<TraversalLinkPrecheck, String> {
    if link[0] & (1 << 27) != 0 {
        return Ok(TraversalLinkPrecheck::Gates {
            open: gate_link_allows(bytes, link, gate_states)?,
        });
    }
    let geometry = packed_link_geometry(link).ok_or("missing packed Tome link geometry")?;
    let faces_query = match direction {
        LinkQueryDirection::Volume { center, tolerance } => {
            packed_link_faces_volume(geometry, center, tolerance)
        }
        LinkQueryDirection::Point { axis_sides } => packed_link_faces_point(geometry, axis_sides),
    };
    Ok(TraversalLinkPrecheck::PackedDirection { faces_query })
}

/// FUN_140a85790's ready same-spatial-cell link condition. It scans direct
/// links targeting the current spatial record, accepts a gated link or an
/// ungated link whose face does not face the current query, and only selects
/// a destination cell whose state is queued (0x8000 without 0x4000). The
/// state's low 14 bits identify that cell's work-list entry; they are not the
/// destination cell index. Gate openness is decided earlier by
/// FUN_140a85bd0; this helper expects links already admitted by that stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadyCellTarget {
    pub cell_index: u16,
    pub worklist_entry: u16,
}

pub fn ready_same_spatial_cell_target(
    link: [u32; 4],
    current_spatial_index: usize,
    direction: LinkQueryDirection,
    destination_state: u16,
) -> Option<ReadyCellTarget> {
    if direct_spatial_link_target(link)? as usize != current_spatial_index {
        return None;
    }
    if destination_state & 0xc000 != 0x8000 {
        return None;
    }
    if link[0] & (1 << 27) == 0 {
        let geometry = packed_link_geometry(link)?;
        let faces_query = match direction {
            LinkQueryDirection::Volume { center, tolerance } => {
                packed_link_faces_volume(geometry, center, tolerance)
            }
            LinkQueryDirection::Point { axis_sides } => {
                packed_link_faces_point(geometry, axis_sides)
            }
        };
        if faces_query {
            return None;
        }
    }
    Some(ReadyCellTarget {
        cell_index: (link[1] >> 16) as u16,
        worklist_entry: destination_state & CellTraversalState::ENTRY_MASK,
    })
}

/// FUN_140a89a90/140a89d80 allocate and grow a portal's screen-tile mask;
/// FUN_140a89b20 merges incoming coverage and reports newly reached tiles.
/// Pool allocation and its failure path are separate from this mask data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalTileMask {
    pub rect: [u8; 4], // left, top, right, bottom; upper edges exclusive
    pub bits: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraversalCellId {
    pub spatial_index: usize,
    pub cell_index: usize,
}

/// The original query's 16-bit cell-state word. FUN_140a82340 writes
/// 0x8000 for queued work, FUN_140a85bd0 writes 0x4000 after processing,
/// and FUN_140a85790 selects entries whose 0xc000 flag bits equal 0x8000.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellTraversalState(u16);

impl CellTraversalState {
    const ENTRY_MASK: u16 = 0x3fff;
    const PROCESSED: u16 = 0x4000;
    const QUEUED: u16 = 0x8000;

    pub fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    pub fn queued(entry: u16) -> Result<Self, String> {
        if entry & !Self::ENTRY_MASK != 0 {
            return Err("Tome traversal entry exceeds 14-bit state field".into());
        }
        Ok(Self(entry | Self::QUEUED))
    }

    pub fn processed(entry: u16) -> Result<Self, String> {
        if entry & !Self::ENTRY_MASK != 0 {
            return Err("Tome traversal entry exceeds 14-bit state field".into());
        }
        Ok(Self(entry | Self::PROCESSED))
    }

    pub fn is_pending(self) -> bool {
        self.0 & (Self::PROCESSED | Self::QUEUED) == Self::QUEUED
    }

    pub fn entry(self) -> u16 {
        self.0 & Self::ENTRY_MASK
    }

    pub fn raw(self) -> u16 {
        self.0
    }
}

/// Result of FUN_140a84f10's scan for one spatial-record index. `slots` holds
/// the intermediate query cell slots followed by the final slot. The game
/// stores the final slot at cache[count] without increasing count, so a cache
/// hit returns the final element while the caller marks only the preceding
/// elements processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialCellSlotMapping {
    pub slots: Vec<u16>,
    pub next_slot: u16,
}

/// Packed row data exposed by the Tome resource view for FUN_140a84f10.
/// FUN_140a7c020 builds the descriptor from header +0x98, with row width
/// header +0x94 and total packed bits header +0x90 * header +0x94.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialCellSlotRows {
    spatial_record_count: usize,
    bit_count: usize,
    packed_words: Vec<u32>,
}

impl SpatialCellSlotRows {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let spatial_record_count = word(bytes, 0x90)? as usize;
        let bit_count = word(bytes, 0x94)? as usize;
        let data_offset = word(bytes, 0x98)? as usize;
        if spatial_record_count == 0 || bit_count == 0 {
            return Err("Tome spatial slot row dimensions are zero".into());
        }
        let total_bits = spatial_record_count
            .checked_mul(bit_count)
            .ok_or("Tome spatial slot row bit count overflow")?;
        let word_count = total_bits
            .checked_add(31)
            .ok_or("Tome spatial slot row word count overflow")?
            / 32;
        let data_bytes = word_count
            .checked_mul(4)
            .ok_or("Tome spatial slot row byte count overflow")?;
        let end = data_offset
            .checked_add(data_bytes)
            .ok_or("Tome spatial slot row extent overflow")?;
        if bytes.get(data_offset..end).is_none() {
            return Err("Tome spatial slot rows exceed file".into());
        }
        let packed_words = (0..word_count)
            .map(|index| word(bytes, data_offset + index * 4))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            spatial_record_count,
            bit_count,
            packed_words,
        })
    }

    pub fn spatial_record_count(&self) -> usize {
        self.spatial_record_count
    }

    pub fn bit_count(&self) -> usize {
        self.bit_count
    }

    pub fn resolve(
        &self,
        mapper: &mut SpatialCellSlotMapper,
        target_spatial_index: usize,
        query_cell_states: &[u16],
    ) -> Result<u16, String> {
        if target_spatial_index >= self.spatial_record_count {
            return Err("Tome target spatial index exceeds slot row table".into());
        }
        mapper.resolve(
            target_spatial_index,
            self.bit_count,
            1,
            &self.packed_words,
            query_cell_states,
        )
    }
}

/// Per-query cache used by FUN_140a84f10. The game caches the mapping for one
/// target spatial record and reuses its final slot until a different record is
/// requested.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpatialCellSlotMapper {
    target_spatial_index: Option<usize>,
    mapping: Option<SpatialCellSlotMapping>,
}

impl SpatialCellSlotMapper {
    pub fn resolve(
        &mut self,
        target_spatial_index: usize,
        bit_count: usize,
        packed_word_stride: usize,
        packed_words: &[u32],
        query_cell_states: &[u16],
    ) -> Result<u16, String> {
        if self.target_spatial_index == Some(target_spatial_index) {
            return self
                .mapping
                .as_ref()
                .map(|mapping| mapping.next_slot)
                .ok_or_else(|| "Tome spatial slot cache is missing its mapping".into());
        }
        let mapping = map_spatial_cell_slots(
            target_spatial_index,
            bit_count,
            packed_word_stride,
            packed_words,
            query_cell_states,
        )?;
        let next_slot = mapping.next_slot;
        self.target_spatial_index = Some(target_spatial_index);
        self.mapping = Some(mapping);
        Ok(next_slot)
    }

    /// Slots the original caller marks processed after using the final slot.
    pub fn intermediate_slots(&self) -> &[u16] {
        self.mapping
            .as_ref()
            .map(|mapping| &mapping.slots[..mapping.slots.len().saturating_sub(1)])
            .unwrap_or_default()
    }

    /// Match FUN_140a85bd0's post-portal loop, which ORs 0x4000 into every
    /// cached intermediate cell state and leaves the final slot unprocessed.
    pub fn mark_intermediate_processed(&self, cell_states: &mut [u16]) -> Result<(), String> {
        for &slot in self.intermediate_slots() {
            let state = cell_states
                .get_mut(slot as usize)
                .ok_or("Tome spatial map exceeds query cell-state table")?;
            *state |= CellTraversalState::PROCESSED;
        }
        Ok(())
    }
}

/// Decode the packed-bit scan performed by FUN_140a84f10.
///
/// FUN_140a85bd0 passes the direct link's target spatial index, so the
/// function scans `bit_count` bits starting at `target_spatial_index * bit_count`.
/// Each successive 32-bit packed word is `packed_word_stride` words after the
/// prior one, matching the executable's `descriptor[0x28] * word_index`
/// address calculation. The query-state array is indexed starting at slot 1.
/// This ports the scan itself; construction of the packed descriptor and the
/// executable's cache cursor remain caller responsibilities.
pub fn map_spatial_cell_slots(
    target_spatial_index: usize,
    bit_count: usize,
    packed_word_stride: usize,
    packed_words: &[u32],
    query_cell_states: &[u16],
) -> Result<SpatialCellSlotMapping, String> {
    if packed_word_stride == 0 {
        return Err("Tome spatial bit-row word stride is zero".into());
    }
    let bit_start = target_spatial_index
        .checked_mul(bit_count)
        .ok_or("Tome spatial bit offset overflow")?;
    let mut word_index = bit_start / 32;
    let mut bit_in_word = bit_start % 32;
    let mut bits_remaining = bit_count;
    let mut slot = 1usize;
    let mut any_prior_bit = false;
    let mut slots = Vec::new();

    while bits_remaining != 0 {
        let in_word = bits_remaining.min(32 - bit_in_word);
        let packed_at = word_index
            .checked_mul(packed_word_stride)
            .ok_or("Tome spatial packed-word offset overflow")?;
        let mut packed = *packed_words
            .get(packed_at)
            .ok_or("truncated Tome spatial bit rows")?
            >> bit_in_word;

        for _ in 0..in_word {
            let bit = (packed & 1) as usize;
            if any_prior_bit {
                let state = *query_cell_states
                    .get(slot)
                    .ok_or("Tome spatial map exceeds query cell-state table")?;
                if state & 0x7fff == 0 {
                    slots.push(
                        u16::try_from(slot)
                            .map_err(|_| "Tome spatial query cell slot exceeds u16")?,
                    );
                }
                slot = ((state & 0x3fff) as usize + 1)
                    .checked_sub(bit)
                    .ok_or("invalid Tome spatial query cell slot transition")?;
            }
            any_prior_bit |= bit != 0;
            packed >>= 1;
        }

        bits_remaining -= in_word;
        bit_in_word = 0;
        word_index = word_index
            .checked_add(1)
            .ok_or("Tome spatial packed-word offset overflow")?;
    }

    let next_slot = u16::try_from(slot).map_err(|_| "Tome spatial query cell slot exceeds u16")?;
    slots.push(next_slot);
    Ok(SpatialCellSlotMapping { slots, next_slot })
}

/// One runtime work-list node in FUN_140a85bd0. The entry ID is the node's
/// intrusive-list index; the cell index is the +4 value stored in that node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellWorkItem {
    pub entry_id: u16,
    pub cell_index: u16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CellWorkNode {
    bytes: [u8; 0x18],
    allocated: bool,
}

impl CellWorkNode {
    fn next(&self) -> u16 {
        u16::from_le_bytes(self.bytes[0..2].try_into().unwrap())
    }

    fn set_next(&mut self, next: u16) {
        self.bytes[0..2].copy_from_slice(&next.to_le_bytes());
    }

    fn cell_index(&self) -> u16 {
        u16::from_le_bytes(self.bytes[4..6].try_into().unwrap())
    }

    fn set_cell_index(&mut self, cell_index: u16) {
        self.bytes[4..6].copy_from_slice(&cell_index.to_le_bytes());
    }
}

/// Query-local 0x18-byte node pool used by FUN_140a85090/FUN_140a84e90.
/// Index zero is the sentinel. Fresh IDs are consumed in ascending order;
/// once exhausted, FUN_140a84dd0 flushes the recycled head through the cell
/// object query before reusing that node. The callback owns that delayed
/// candidate pass and release of the node's portal-mask allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellWorkNodePool {
    nodes: Vec<CellWorkNode>,
    owners: Vec<Option<TraversalCellId>>,
    free_head: u16,
    recycled_head: u16,
}

impl CellWorkNodePool {
    /// `node_count` includes the sentinel slot at index zero, matching the
    /// original 16-bit count and 14-bit cell-state entry field.
    pub fn new(node_count: usize) -> Result<Self, String> {
        if node_count == 0 || node_count > 0x4000 {
            return Err("Tome work-node count is outside the original 16-bit pool range".into());
        }
        let mut nodes = vec![CellWorkNode::default(); node_count];
        for (id, node) in nodes.iter_mut().enumerate().skip(1) {
            node.set_next(
                u16::try_from(id + 1)
                    .ok()
                    .filter(|&next| next < node_count as u16)
                    .unwrap_or(0),
            );
        }
        Ok(Self {
            owners: vec![None; node_count],
            nodes,
            free_head: if node_count > 1 { 1 } else { 0 },
            recycled_head: 0,
        })
    }

    /// Create FUN_140a85090's bounded node pool, including sentinel ID zero.
    pub fn new_for_original_query() -> Result<Self, String> {
        Self::new(ORIGINAL_QUERY_WORK_NODE_COUNT)
    }

    pub fn free_count(&self) -> usize {
        let mut count = 0;
        let mut current = self.free_head;
        while current != 0 {
            count += 1;
            current = self.nodes[current as usize].next();
        }
        count
    }

    pub fn recycled_count(&self) -> usize {
        let mut count = 0;
        let mut current = self.recycled_head;
        while current != 0 {
            count += 1;
            current = self.nodes[current as usize].next();
        }
        count
    }

    /// Allocate a node for a new cell. If the fresh list is empty, the
    /// callback performs the recycled head's delayed cell query and releases
    /// its portal-mask allocation; this then resets the old cell state and
    /// returns that same node ID as FUN_140a84dd0/FUN_140a84e90 do.
    pub fn allocate<F>(
        &mut self,
        destination: TraversalCellId,
        states: &mut [u16],
        coverage: &mut VisitedCellCoverage,
        mut process_recycled: F,
    ) -> Result<Option<CellWorkItem>, String>
    where
        F: FnMut(CellWorkItem, TraversalCellId, &VisitedCellCoverage) -> Result<(), String>,
    {
        let cell_index = u16::try_from(destination.cell_index)
            .map_err(|_| "Tome work-node cell index exceeds 16 bits")?;
        if states.get(destination.cell_index).is_none() {
            return Err("Tome cell-state table is too short for allocated node".into());
        }
        let id = if self.free_head != 0 {
            let id = self.free_head;
            self.free_head = self.nodes[id as usize].next();
            id
        } else {
            let id = self.recycled_head;
            if id == 0 {
                return Ok(None);
            }
            let previous = CellWorkItem {
                entry_id: id,
                cell_index: self.nodes[id as usize].cell_index(),
            };
            let previous_owner = self.owners[id as usize]
                .ok_or("recycled Tome work node has no owning spatial cell")?;
            let Some(state) = states.get(previous.cell_index as usize) else {
                return Err("Tome cell-state table is too short for recycled node".into());
            };
            if *state != (id | 0x4000) {
                return Err("recycled Tome node disagrees with its cell state".into());
            }
            if let Err(error) = process_recycled(previous, previous_owner, coverage) {
                return Err(error);
            }
            self.recycled_head = self.nodes[id as usize].next();
            self.nodes[id as usize].bytes[8..].fill(0);
            states[previous.cell_index as usize] = id;
            coverage.remove(previous_owner);
            id
        };
        let node = &mut self.nodes[id as usize];
        if node.allocated {
            return Err("Tome work-node pool selected an allocated node".into());
        }
        node.set_cell_index(cell_index);
        node.set_next(0);
        node.allocated = true;
        self.owners[id as usize] = Some(destination);
        Ok(Some(CellWorkItem {
            entry_id: id,
            cell_index,
        }))
    }

    /// Retire a processed cell onto the recycled head. Its candidates are
    /// flushed only if the fresh free list later runs out, matching the
    /// original bounded pool's deferred object-query path.
    pub fn release_processed(
        &mut self,
        item: CellWorkItem,
        states: &mut [u16],
    ) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(item.entry_id as usize)
            .filter(|_| item.entry_id != 0)
            .ok_or("Tome work-node ID exceeds pool")?;
        if !node.allocated || node.cell_index() != item.cell_index {
            return Err("Tome work-node release does not match its allocation".into());
        }
        let state = states
            .get_mut(item.cell_index as usize)
            .ok_or("Tome cell-state table is too short for processed node")?;
        *state = item.entry_id | 0x4000;
        node.set_next(self.recycled_head);
        node.allocated = false;
        self.recycled_head = item.entry_id;
        Ok(())
    }

    /// FUN_140a85bd0 removes a processed node from the recycled chain when
    /// another portal contributes new coverage to that same local cell.
    /// Object candidates remain deferred until the node is retired again.
    pub fn reactivate_processed(
        &mut self,
        item: CellWorkItem,
        owner: TraversalCellId,
        states: &[u16],
    ) -> Result<(), String> {
        let id = item.entry_id;
        let node = self
            .nodes
            .get(id as usize)
            .filter(|_| id != 0)
            .ok_or("processed Tome work-node ID exceeds pool")?;
        if node.allocated
            || node.cell_index() != item.cell_index
            || self.owners[id as usize] != Some(owner)
            || states.get(owner.cell_index).copied() != Some(id | 0x4000)
        {
            return Err("processed Tome work node disagrees with its cell state".into());
        }
        let mut previous = 0;
        let mut current = self.recycled_head;
        while current != 0 && current != id {
            previous = current;
            current = self.nodes[current as usize].next();
        }
        if current == 0 {
            return Err("processed Tome work node is absent from recycled list".into());
        }
        let next = self.nodes[id as usize].next();
        if previous == 0 {
            self.recycled_head = next;
        } else {
            self.nodes[previous as usize].set_next(next);
        }
        self.nodes[id as usize].set_next(0);
        self.nodes[id as usize].allocated = true;
        Ok(())
    }

    /// Drain processed nodes after the active cell list is exhausted. The
    /// original FUN_140a85bd0 runs their deferred candidate queries, clears
    /// each node's portal mask and returns every node to the fresh-list head.
    pub fn flush_recycled<F>(
        &mut self,
        states: &mut [u16],
        coverage: &mut VisitedCellCoverage,
        mut process_recycled: F,
    ) -> Result<usize, String>
    where
        F: FnMut(CellWorkItem, TraversalCellId, &VisitedCellCoverage) -> Result<(), String>,
    {
        let mut flushed = 0;
        while self.recycled_head != 0 {
            let id = self.recycled_head;
            let item = CellWorkItem {
                entry_id: id,
                cell_index: self.nodes[id as usize].cell_index(),
            };
            let owner = self.owners[id as usize]
                .ok_or("recycled Tome work node has no owning spatial cell")?;
            let Some(state) = states.get(item.cell_index as usize) else {
                return Err("Tome cell-state table is too short for recycled node".into());
            };
            if *state != (id | 0x4000) {
                return Err("recycled Tome node disagrees with its cell state".into());
            }
            if let Err(error) = process_recycled(item, owner, coverage) {
                return Err(error);
            }
            self.recycled_head = self.nodes[id as usize].next();
            let node = &mut self.nodes[id as usize];
            node.bytes[8..].fill(0);
            node.set_next(self.free_head);
            node.allocated = false;
            self.owners[id as usize] = None;
            self.free_head = id;
            states[item.cell_index as usize] = id;
            coverage.remove(owner);
            flushed += 1;
        }
        Ok(flushed)
    }

    /// Return a just-allocated node to the fresh-list head when portal
    /// rasterization produced no new coverage, matching FUN_140a85bd0's
    /// uVar50 == 0 / cVar18 == 0 rollback branch.
    pub fn release_unused(&mut self, item: CellWorkItem) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(item.entry_id as usize)
            .filter(|_| item.entry_id != 0)
            .ok_or("Tome work-node ID exceeds pool")?;
        if !node.allocated || node.cell_index() != item.cell_index {
            return Err("unused Tome work-node return does not match its allocation".into());
        }
        node.bytes[8..].fill(0);
        node.set_next(self.free_head);
        node.allocated = false;
        self.owners[item.entry_id as usize] = None;
        self.free_head = item.entry_id;
        Ok(())
    }
}

/// Ordered model of the runtime list passed to FUN_140a85790. Node index zero
/// is the list sentinel and is therefore not a work item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellTraversalWorklist {
    entries: VecDeque<CellWorkItem>,
}

impl CellTraversalWorklist {
    pub fn new(entries: impl IntoIterator<Item = CellWorkItem>) -> Result<Self, String> {
        let entries = entries.into_iter().collect::<VecDeque<_>>();
        let mut seen = std::collections::HashSet::with_capacity(entries.len());
        for item in &entries {
            if item.entry_id == 0 || item.entry_id & !CellTraversalState::ENTRY_MASK != 0 {
                return Err("Tome work-list entry is outside its 14-bit non-sentinel range".into());
            }
            if !seen.insert(item.entry_id) {
                return Err("duplicate Tome work-list entry ID".into());
            }
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> impl Iterator<Item = CellWorkItem> + '_ {
        self.entries.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Port FUN_140a85790's same-spatial fast path and its ordered-list/state
    /// transitions. The scan limit is derived from the active list after its
    /// head is removed, matching FUN_140a85790's decrement of the query-local
    /// active count. Gate openness is assumed to have been checked by
    /// FUN_140a85bd0 before this helper is reached.
    pub fn select_same_spatial_chain<F>(
        &mut self,
        spatial_index: usize,
        direction: LinkQueryDirection,
        cell_states: &mut [u16],
        mut links_for_cell: F,
    ) -> Result<Option<CellWorkItem>, String>
    where
        F: FnMut(usize, &mut dyn FnMut([u32; 4]) -> Result<bool, String>) -> Result<(), String>,
    {
        let Some(mut current) = self.entries.pop_front() else {
            return Ok(None);
        };

        let mut remaining = self.entries.len();
        while remaining != 0 {
            let mut next = None;
            links_for_cell(current.cell_index as usize, &mut |link| {
                // FUN_140a85790 jumps out of the fast scan at the first
                // boundary-face or different-spatial link.
                let Some(target_spatial) = direct_spatial_link_target(link) else {
                    return Ok(false);
                };
                if target_spatial as usize != spatial_index {
                    return Ok(false);
                }
                let target_cell_index = (link[1] >> 16) as usize;
                let state = *cell_states
                    .get(target_cell_index)
                    .ok_or("Tome traversal cell-state table is too short")?;
                if let Some(target) =
                    ready_same_spatial_cell_target(link, spatial_index, direction, state)
                {
                    next = Some(target);
                    return Ok(false);
                }
                Ok(true)
            })?;

            let Some(target) = next else {
                break;
            };

            // FUN_140a85790 puts the current entry at the list head and marks
            // its cell processed before removing the queued target entry.
            self.entries.push_front(current);
            let source_state = cell_states
                .get_mut(current.cell_index as usize)
                .ok_or("Tome traversal cell-state table is too short")?;
            *source_state = current.entry_id | 0xc000;

            let target_position = self
                .entries
                .iter()
                .position(|item| item.entry_id == target.worklist_entry)
                .ok_or("queued Tome work-list entry is absent from the active list")?;
            current = self.entries.remove(target_position).unwrap();
            remaining -= 1;
        }

        // Match FUN_140a85790's list walk: requeue entries until the first
        // entry whose cell already has the queued flag.
        for item in &self.entries {
            let state = cell_states
                .get_mut(item.cell_index as usize)
                .ok_or("Tome traversal cell-state table is too short")?;
            if *state & 0xc000 == 0x8000 {
                break;
            }
            *state = item.entry_id | 0x8000;
        }

        Ok(Some(current))
    }
}

/// Pending per-spatial cell lists and the active list initialized by
/// FUN_140a82340. The slot is the mapped list-head index produced for a
/// spatial record by the query; work-item entry IDs come from the game's
/// separate query-local node pool.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SpatialCellWorklists {
    pending_by_slot: HashMap<usize, VecDeque<CellWorkItem>>,
    active: CellTraversalWorklist,
    cell_states: Vec<u16>,
}

impl SpatialCellWorklists {
    pub fn new(cell_state_count: usize) -> Self {
        Self {
            pending_by_slot: HashMap::new(),
            active: CellTraversalWorklist::default(),
            cell_states: vec![0; cell_state_count],
        }
    }

    /// Create the per-record local cell-state buffer cleared by
    /// FUN_140a85af0 before its point-cell lookup.
    pub fn new_for_original_query() -> Self {
        Self::new(ORIGINAL_RECORD_CELL_STATE_COUNT)
    }

    /// Match FUN_140a85af0's per-record clearing of the local cell-state
    /// buffer before FUN_140a84370 seeds the camera cell.
    pub fn reset_for_record(&mut self, local_cell_count: usize) -> Result<(), String> {
        if local_cell_count > self.cell_states.len() {
            return Err("Tome record exceeds the original local cell-state buffer".into());
        }
        self.cell_states.fill(0);
        Ok(())
    }

    /// Return an existing cell in a pending spatial list. FUN_140a85bd0 does
    /// this search before allocating and prepending a new work-list node.
    pub fn pending_cell(&self, slot: usize, cell_index: u16) -> Option<CellWorkItem> {
        self.pending_by_slot
            .get(&slot)?
            .iter()
            .find(|item| item.cell_index == cell_index)
            .copied()
    }

    /// Seed the camera's local cell into its spatial-record pending list.
    /// FUN_140a84370 selects that cell and calls FUN_140a82160, which copies
    /// the query's initial viewport mask into a fresh work node, prepends the
    /// node at the supplied slot, and marks the cell queued.
    pub fn seed_initial_camera_cell(
        &mut self,
        pool: &mut CellWorkNodePool,
        coverage: &mut VisitedCellCoverage,
        destination: TraversalCellId,
        pending_slot: usize,
        initial_mask: &PortalTileMask,
    ) -> Result<Option<CellWorkItem>, String> {
        let state = *self
            .cell_states
            .get(destination.cell_index)
            .ok_or("Tome cell-state table is too short for initial camera cell")?;
        if state & 0xc000 != 0 {
            return Ok(None);
        }
        let Some(item) =
            pool.allocate(destination, &mut self.cell_states, coverage, |_, _, _| {
                Err("initial camera-cell seed cannot recycle a node".into())
            })?
        else {
            return Ok(None);
        };
        if let Err(error) = coverage.merge_incoming(destination, initial_mask) {
            pool.release_unused(item)?;
            coverage.remove(destination);
            return Err(error);
        }
        if let Err(error) = self.prepend_pending_cell(pending_slot, item) {
            pool.release_unused(item)?;
            coverage.remove(destination);
            return Err(error);
        }
        self.cell_states[destination.cell_index] = item.entry_id | CellTraversalState::QUEUED;
        Ok(Some(item))
    }

    /// Prepend a newly allocated destination cell node to the pending list
    /// for its mapped spatial slot, matching FUN_140a85bd0's +0x4e head update.
    pub fn prepend_pending_cell(&mut self, slot: usize, item: CellWorkItem) -> Result<(), String> {
        if item.entry_id == 0 || item.entry_id & !CellTraversalState::ENTRY_MASK != 0 {
            return Err("Tome work-list entry is outside its 14-bit non-sentinel range".into());
        }
        if self.pending_cell(slot, item.cell_index).is_some() {
            return Err("destination cell already exists in pending spatial list".into());
        }
        if self
            .active
            .entries
            .iter()
            .any(|active| active.entry_id == item.entry_id)
            || self
                .pending_by_slot
                .values()
                .flatten()
                .any(|queued| queued.entry_id == item.entry_id)
        {
            return Err("duplicate Tome work-list entry ID".into());
        }
        self.pending_by_slot
            .entry(slot)
            .or_default()
            .push_front(item);
        Ok(())
    }

    /// Prepend a newly reached cell to the active list for the current
    /// spatial record. FUN_140a85bd0 takes this branch after new portal
    /// coverage reaches an unvisited cell in that record; it marks the cell
    /// queued and increments the active-list count.
    pub fn prepend_active_cell(&mut self, item: CellWorkItem) -> Result<(), String> {
        if item.entry_id == 0 || item.entry_id & !CellTraversalState::ENTRY_MASK != 0 {
            return Err("Tome work-list entry is outside its 14-bit non-sentinel range".into());
        }
        let cell_index = item.cell_index as usize;
        let state = *self
            .cell_states
            .get(cell_index)
            .ok_or("Tome cell-state table is too short for active cell")?;
        if state & 0xc000 != 0 {
            return Err("new active Tome cell is not in the unvisited state".into());
        }
        if self
            .active
            .entries
            .iter()
            .any(|active| active.entry_id == item.entry_id || active.cell_index == item.cell_index)
            || self
                .pending_by_slot
                .values()
                .flatten()
                .any(|queued| queued.entry_id == item.entry_id)
        {
            return Err("duplicate Tome active work-list entry or cell".into());
        }
        self.active.entries.push_front(item);
        self.cell_states[cell_index] = item.entry_id | CellTraversalState::QUEUED;
        Ok(())
    }

    /// Requeue a previously processed same-record cell after newly merged
    /// portal coverage. FUN_140a85bd0 unlinks its node from the recycled
    /// chain before restoring the active-list flag.
    pub fn reactivate_processed_active_cell(
        &mut self,
        pool: &mut CellWorkNodePool,
        coverage: &mut VisitedCellCoverage,
        destination: TraversalCellId,
        item: CellWorkItem,
        pixels: [i32; 4],
        incoming: &PortalTileMask,
        path: PortalCoveragePath,
    ) -> Result<bool, String> {
        if self.cell_states.get(destination.cell_index).copied() != Some(item.entry_id | 0x4000)
            || item.cell_index as usize != destination.cell_index
            || self
                .active
                .entries
                .iter()
                .any(|active| active.entry_id == item.entry_id)
        {
            return Err("processed Tome cell cannot be reactivated from this worklist".into());
        }
        if !coverage.propagate_portal(destination, pixels, incoming, path)? {
            return Ok(false);
        }
        pool.reactivate_processed(item, destination, &self.cell_states)?;
        self.active.entries.push_front(item);
        self.cell_states[destination.cell_index] = item.entry_id | CellTraversalState::QUEUED;
        Ok(true)
    }

    /// Commit scheduling for a work item that the caller has already
    /// allocated. The source-ordered operation, which allocates before
    /// coverage merge and returns unused nodes to the pool, is
    /// `propagate_and_schedule_unvisited_cell_from_pool` below.
    pub fn propagate_and_schedule_unvisited_cell(
        &mut self,
        coverage: &mut VisitedCellCoverage,
        destination: TraversalCellId,
        pending_slot: usize,
        destination_is_current_spatial: bool,
        item: CellWorkItem,
        pixels: [i32; 4],
        incoming: &PortalTileMask,
        path: PortalCoveragePath,
    ) -> Result<bool, String> {
        if !coverage.propagate_portal(destination, pixels, incoming, path)? {
            return Ok(false);
        }
        if destination_is_current_spatial {
            self.prepend_active_cell(item)?;
        } else if self.pending_cell(pending_slot, item.cell_index).is_none() {
            self.prepend_pending_cell(pending_slot, item)?;
        }
        Ok(true)
    }

    /// FUN_140a85bd0 allocates a node for a new unvisited cell before merging
    /// portal coverage. If the merge adds no bits, that node is returned to
    /// the fresh pool. Cross-record cells already on a pending list reuse
    /// their node and are not inserted twice.
    pub fn propagate_and_schedule_unvisited_cell_from_pool<F>(
        &mut self,
        pool: &mut CellWorkNodePool,
        coverage: &mut VisitedCellCoverage,
        destination: TraversalCellId,
        pending_slot: usize,
        destination_is_current_spatial: bool,
        pixels: [i32; 4],
        incoming: &PortalTileMask,
        path: PortalCoveragePath,
        process_recycled: F,
    ) -> Result<bool, String>
    where
        F: FnMut(CellWorkItem, TraversalCellId, &VisitedCellCoverage) -> Result<(), String>,
    {
        let destination_cell = u16::try_from(destination.cell_index)
            .map_err(|_| "Tome destination cell index exceeds 16 bits")?;
        let existing_pending = if destination_is_current_spatial {
            None
        } else {
            self.pending_cell(pending_slot, destination_cell)
        };
        let newly_allocated = existing_pending.is_none();
        let item = match existing_pending {
            Some(item) => item,
            None => {
                let state = *self
                    .cell_states
                    .get(destination.cell_index)
                    .ok_or("Tome cell-state table is too short for destination")?;
                // Cross-record destinations are searched in their pending
                // spatial list. The current record's local state at the same
                // numeric cell index belongs to a different cell.
                if destination_is_current_spatial && state & 0xc000 != 0 {
                    return Err("new Tome destination cell is not unvisited".into());
                }
                let Some(item) = pool.allocate(
                    destination,
                    &mut self.cell_states,
                    coverage,
                    process_recycled,
                )?
                else {
                    return Ok(false);
                };
                item
            }
        };

        // Another spatial slot can already hold a pending node for the same
        // destination. A second tentative node must not erase that node's
        // accumulated coverage when its portal adds no bits.
        let previous_coverage = coverage.coverage(destination).cloned();
        let changed = match coverage.propagate_portal(destination, pixels, incoming, path) {
            Ok(changed) => changed,
            Err(error) => {
                if newly_allocated {
                    pool.release_unused(item)?;
                    if previous_coverage.is_none() {
                        coverage.remove(destination);
                    }
                }
                return Err(error);
            }
        };
        if !changed {
            if newly_allocated {
                pool.release_unused(item)?;
                if previous_coverage.is_none() {
                    coverage.remove(destination);
                }
            }
            return Ok(false);
        }

        let scheduled = if newly_allocated {
            if destination_is_current_spatial {
                self.prepend_active_cell(item)
            } else {
                self.prepend_pending_cell(pending_slot, item)
            }
        } else {
            Ok(())
        };
        if let Err(error) = scheduled {
            if newly_allocated {
                pool.release_unused(item)?;
                match previous_coverage {
                    Some(previous) => {
                        coverage.masks.insert(destination, previous);
                    }
                    None => coverage.remove(destination),
                }
            }
            return Err(error);
        }
        Ok(true)
    }

    /// Resolve the destination's pending-list slot with FUN_140a84f10, then
    /// schedule the cross-spatial cell using FUN_140a85bd0's node-pool path.
    /// FUN_140a84f10 scans the global branch-state table at query +0x94,
    /// distinct from this record's local cell states at +0xc80. The
    /// destination index must already include any collection-Tome remap;
    /// this helper currently models the direct single-Tome path.
    #[allow(clippy::too_many_arguments)]
    pub fn propagate_and_schedule_cross_spatial_cell_from_pool<F>(
        &mut self,
        mapper: &mut SpatialCellSlotMapper,
        pool: &mut CellWorkNodePool,
        coverage: &mut VisitedCellCoverage,
        rows: &SpatialCellSlotRows,
        destination: TraversalCellId,
        global_branch_states: &mut [u16],
        pixels: [i32; 4],
        incoming: &PortalTileMask,
        path: PortalCoveragePath,
        process_recycled: F,
    ) -> Result<bool, String>
    where
        F: FnMut(CellWorkItem, TraversalCellId, &VisitedCellCoverage) -> Result<(), String>,
    {
        let pending_slot = rows.resolve(mapper, destination.spatial_index, global_branch_states)?;
        let already_pending =
            self.pending_cell(pending_slot as usize, destination.cell_index as u16);
        let owner = match already_pending {
            Some(item) => {
                pool.owners[item.entry_id as usize].ok_or("pending Tome work node has no owner")?
            }
            None => destination,
        };
        let changed = self.propagate_and_schedule_unvisited_cell_from_pool(
            pool,
            coverage,
            owner,
            pending_slot as usize,
            false,
            pixels,
            incoming,
            path,
            process_recycled,
        )?;
        if changed && already_pending.is_none() {
            // FUN_140a85bd0 stores the new work-node ID in the global
            // pending-list head at query +0x270 before marking the cached
            // intermediate slots. FUN_140a85510 reads this same table to
            // select the next spatial record.
            let head = self
                .pending_by_slot
                .get(&(pending_slot as usize))
                .and_then(|items| items.front())
                .ok_or("scheduled Tome cross-record cell has no pending head")?;
            *global_branch_states
                .get_mut(pending_slot as usize)
                .ok_or("Tome pending slot exceeds global branch states")? = head.entry_id;
            mapper.mark_intermediate_processed(global_branch_states)?;
        }
        Ok(changed)
    }

    /// Move a selected spatial slot's pending nodes into the active list and
    /// set each target cell state to `entry_id | 0x8000`, matching
    /// FUN_140a82340. The caller supplies the mapped list slot returned by
    /// FUN_140a84f10/FUN_140a85510.
    pub fn activate_pending_spatial(&mut self, slot: usize) -> Result<usize, String> {
        if !self.active.is_empty() {
            return Err("cannot initialize a spatial cell list while work remains active".into());
        }
        let Some(pending) = self.pending_by_slot.get(&slot) else {
            return Ok(0);
        };
        for item in pending {
            if item.cell_index as usize >= self.cell_states.len() {
                return Err("Tome cell-state table is too short for pending cell".into());
            }
        }
        let pending = self.pending_by_slot.remove(&slot).unwrap();
        let count = pending.len();
        for item in pending {
            self.cell_states[item.cell_index as usize] = item.entry_id | 0x8000;
            self.active.entries.push_back(item);
        }
        Ok(count)
    }

    /// A link target can resolve to a coarser spatial slot. The original
    /// pending node holds only a local cell index; FUN_140a82340 loads the
    /// selected record when that slot is activated. Re-key the Rust coverage
    /// and pool owner to that selected record before processing its cells.
    pub fn bind_active_cells_to_spatial_record(
        &self,
        spatial_index: usize,
        pool: &mut CellWorkNodePool,
        coverage: &mut VisitedCellCoverage,
    ) -> Result<(), String> {
        for item in &self.active.entries {
            let owner =
                pool.owners[item.entry_id as usize].ok_or("active Tome work node has no owner")?;
            if owner.cell_index != item.cell_index as usize {
                return Err("active Tome work node owner has another cell index".into());
            }
            let selected = TraversalCellId {
                spatial_index,
                cell_index: owner.cell_index,
            };
            if selected == owner {
                continue;
            }
            let old_mask = coverage
                .masks
                .remove(&owner)
                .ok_or("pending Tome cell has no portal coverage")?;
            if let Some(existing) = coverage.masks.get_mut(&selected) {
                existing.grow_to_include(old_mask.rect)?;
                existing.merge_new_coverage(&old_mask)?;
            } else {
                coverage.masks.insert(selected, old_mask);
            }
            pool.owners[item.entry_id as usize] = Some(selected);
        }
        Ok(())
    }

    /// Connect FUN_140a85510's selected-node/returned-slot pair to
    /// FUN_140a82340's starting-cell activation.
    pub fn activate_spatial_candidate(
        &mut self,
        candidate: SpatialQueryBranchCandidate,
    ) -> Result<SpatialRecordActivation, String> {
        let activated_cells = self.activate_pending_spatial(candidate.cell_state_index as usize)?;
        Ok(SpatialRecordActivation {
            spatial_node_index: candidate.spatial_node_index,
            pending_slot: candidate.cell_state_index,
            activated_cells,
        })
    }

    pub fn active_worklist(&self) -> &CellTraversalWorklist {
        &self.active
    }

    pub fn active_worklist_mut(&mut self) -> &mut CellTraversalWorklist {
        &mut self.active
    }

    pub fn cell_states(&self) -> &[u16] {
        &self.cell_states
    }

    pub fn cell_states_mut(&mut self) -> &mut [u16] {
        &mut self.cell_states
    }

    /// Select the next active work item using its spatial record's parsed
    /// Tome link records, connecting FUN_140a82340 initialization to
    /// FUN_140a85790 processing.
    pub fn select_active_cell(
        &mut self,
        cells: &CellTree,
        bytes: &[u8],
        spatial_index: usize,
        direction: LinkQueryDirection,
    ) -> Result<Option<CellWorkItem>, String> {
        cells.select_same_spatial_worklist_entry(
            bytes,
            spatial_index,
            direction,
            &mut self.cell_states,
            &mut self.active,
        )
    }
}

/// Per-(spatial record, cell) portal coverage used by the original query to
/// decide whether another visit carries new screen-tile coverage.
#[derive(Debug, Default)]
pub struct VisitedCellCoverage {
    masks: HashMap<TraversalCellId, PortalTileMask>,
}

impl VisitedCellCoverage {
    fn remove(&mut self, cell: TraversalCellId) {
        self.masks.remove(&cell);
    }

    /// Merge a newly projected portal into the cell's accumulated mask.
    /// FUN_140a85bd0 schedules more cell work only when FUN_140a89b20 reports
    /// newly set coverage bits. This stores that exact state predicate; it
    /// does not choose worklist order or model the original pool allocator.
    pub fn merge_incoming(
        &mut self,
        cell: TraversalCellId,
        incoming: &PortalTileMask,
    ) -> Result<bool, String> {
        if let Some(existing) = self.masks.get_mut(&cell) {
            existing.grow_to_include(incoming.rect)?;
            existing.merge_new_coverage(incoming)
        } else {
            let mut accumulated = PortalTileMask::new(incoming.rect)?;
            let changed = accumulated.merge_new_coverage(incoming)?;
            self.masks.insert(cell, accumulated);
            Ok(changed)
        }
    }

    /// Propagate coverage to a destination cell using FUN_140a85bd0's two
    /// portal branches. With the no-portal sentinel, the incoming coverage is
    /// merged directly; otherwise FUN_140a8a080 clips the projected portal
    /// against that incoming coverage before setting destination bits.
    ///
    /// `pixels` is the clipped portal rectangle already computed by the
    /// caller. Query traversal and work-list scheduling remain caller duties.
    pub fn propagate_portal(
        &mut self,
        cell: TraversalCellId,
        pixels: [i32; 4],
        incoming: &PortalTileMask,
        path: PortalCoveragePath,
    ) -> Result<bool, String> {
        let tile_rect = portal_tile_rect(pixels)?;
        let mut destination = match self.masks.get(&cell) {
            Some(mask) => mask.clone(),
            None => PortalTileMask::new(tile_rect)?,
        };
        destination.grow_to_include(tile_rect)?;
        let changed = match path {
            PortalCoveragePath::Unclipped => destination.merge_new_coverage(incoming)?,
            PortalCoveragePath::Clipped { projected, face } => {
                destination.rasterize_projected_box(incoming, pixels, projected, face)?
            }
        };
        self.masks.insert(cell, destination);
        Ok(changed)
    }

    pub fn coverage(&self, cell: TraversalCellId) -> Option<&PortalTileMask> {
        self.masks.get(&cell)
    }
}

/// The branch selected in FUN_140a85bd0 before portal coverage reaches a
/// destination cell. The clipped form carries the exact vectors supplied to
/// FUN_140a8a080 by the query caller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PortalCoveragePath {
    Unclipped,
    Clipped {
        projected: [[f32; 4]; 4],
        face: [[f32; 4]; 3],
    },
}

/// FUN_140a85bd0's face selector after an authored traversal link passes its
/// gate/direction precheck. None is the no-portal sentinel: the incoming mask
/// is propagated without rasterizing a projected portal rectangle.
pub fn admitted_link_portal_face(
    bytes: &[u8],
    record_offset: u32,
    link: [u32; 4],
    direction: LinkQueryDirection,
    query_point: [f32; 3],
    point_mode_face: Option<u8>,
) -> Result<Option<u8>, String> {
    if query_point.iter().any(|value| !value.is_finite()) {
        return Err("nonfinite Tome portal query point".into());
    }
    if let Some(geometry) = packed_link_geometry(link) {
        let axis = geometry.axis as usize;
        if axis >= 3 {
            return Err("unsupported fourth-axis Tome portal".into());
        }
        let base_face = geometry.axis * 2 + u8::from(geometry.positive_side);
        let LinkQueryDirection::Volume { center, tolerance } = direction else {
            return Ok(Some(base_face));
        };
        let plane = i32::from(geometry.plane);
        let opposite_difference = if geometry.positive_side {
            plane.wrapping_sub(center[axis])
        } else {
            center[axis].wrapping_sub(plane)
        };
        if opposite_difference > tolerance[axis] {
            return Ok(Some(base_face));
        }
        let mut face = None;
        let mut greatest = 0;
        for span in 0..2 {
            let other = (axis + span + 1) % 3;
            let lower_gap = i32::from(geometry.spans[span][0])
                .wrapping_sub(center[other])
                .wrapping_sub(tolerance[other]);
            if lower_gap > greatest {
                greatest = lower_gap;
                face = Some((other * 2 + 1) as u8);
            }
            let upper_gap = center[other]
                .wrapping_sub(i32::from(geometry.spans[span][1]))
                .wrapping_sub(tolerance[other]);
            if upper_gap > greatest {
                greatest = upper_gap;
                face = Some((other * 2) as u8);
            }
        }
        return Ok(face);
    }
    match direction {
        LinkQueryDirection::Point { .. } => point_mode_face
            .filter(|&face| face < 6)
            .map(Some)
            .ok_or_else(|| "missing Tome point-mode portal face".into()),
        LinkQueryDirection::Volume { .. } => {
            let bounds = gated_link_query_bounds(bytes, record_offset, link)?
                .ok_or("missing gated Tome portal bounds")?;
            let mut face = None;
            let mut greatest = 0.0;
            for axis in 0..3 {
                let lower_gap = bounds[0][axis] - query_point[axis];
                if lower_gap > greatest {
                    greatest = lower_gap;
                    face = Some((axis * 2 + 1) as u8);
                } else {
                    let upper_gap = query_point[axis] - bounds[1][axis];
                    if upper_gap > greatest {
                        greatest = upper_gap;
                        face = Some((axis * 2) as u8);
                    }
                }
            }
            Ok(face)
        }
    }
}

/// The portal rectangle and raster inputs passed from FUN_140a85bd0 into
/// FUN_140a89d80/FUN_140a8a080. The caller still resolves the destination,
/// merges coverage, and schedules a cell only when that merge adds bits.
pub fn projected_link_portal_transition(
    bytes: &[u8],
    record_offset: u32,
    link: [u32; 4],
    face_code: Option<u8>,
    query_matrix: [[f32; 4]; 4],
    winding_flip: bool,
    active_planes: &[usize],
    viewport: [i32; 4],
    incoming: &PortalTileMask,
) -> Result<Option<([i32; 4], PortalCoveragePath)>, String> {
    let Some(face_code) = face_code else {
        let rect = incoming.rect;
        return Ok(Some((
            [
                i32::from(rect[0]) * 8,
                i32::from(rect[1]) * 4,
                i32::from(rect[2]) * 8,
                i32::from(rect[3]) * 4,
            ],
            PortalCoveragePath::Unclipped,
        )));
    };
    let bounds = if let Some(geometry) = packed_link_geometry(link) {
        packed_link_query_bounds(bytes, record_offset, geometry)?
    } else {
        gated_link_query_bounds(bytes, record_offset, link)?
            .ok_or("missing gated Tome portal bounds")?
    };
    let (projected, face) =
        portal_raster_inputs_from_bounds(bounds, face_code, query_matrix, winding_flip)?;
    let mut box_corners = [[0.0; 4]; 8];
    for corner in 0..4 {
        box_corners[corner] = [
            projected[0][corner],
            projected[1][corner],
            0.0,
            projected[2][corner],
        ];
        box_corners[corner + 4] = [
            projected[0][corner] + projected[3][0],
            projected[1][corner] + projected[3][1],
            0.0,
            projected[2][corner] + projected[3][3],
        ];
    }
    let (pixels, _) = projected_portal_box_pixel_rect(box_corners, active_planes, viewport)?;
    Ok(pixels.map(|pixels| (pixels, PortalCoveragePath::Clipped { projected, face })))
}

/// One direct, unvisited-cell link transition in FUN_140a85bd0. The outer
/// record scheduler supplies the resolved destination slot and current work
/// mask; boundary-face and already-visited branches remain separate paths.
#[allow(clippy::too_many_arguments)]
pub fn traverse_direct_unvisited_link_from_pool<F>(
    bytes: &[u8],
    record_offset: u32,
    link: [u32; 4],
    direction: LinkQueryDirection,
    gate_states: Option<&[u32]>,
    query_point: [f32; 3],
    point_mode_face: Option<u8>,
    query_matrix: [[f32; 4]; 4],
    winding_flip: bool,
    active_planes: &[usize],
    viewport: [i32; 4],
    incoming: &PortalTileMask,
    source_spatial_index: usize,
    destination: TraversalCellId,
    pending_slot: usize,
    worklists: &mut SpatialCellWorklists,
    pool: &mut CellWorkNodePool,
    coverage: &mut VisitedCellCoverage,
    process_recycled: F,
) -> Result<bool, String>
where
    F: FnMut(CellWorkItem, TraversalCellId, &VisitedCellCoverage) -> Result<(), String>,
{
    let target = direct_spatial_link_target(link)
        .ok_or("direct Tome link transition requires a spatial destination")?;
    if target as usize != destination.spatial_index
        || (link[1] >> 16) as usize != destination.cell_index
    {
        return Err("direct Tome link destination disagrees with resolved cell".into());
    }
    if !traversal_link_precheck(bytes, link, direction, gate_states)?.allows_traversal() {
        return Ok(false);
    }
    let face = admitted_link_portal_face(
        bytes,
        record_offset,
        link,
        direction,
        query_point,
        point_mode_face,
    )?;
    let Some((pixels, path)) = projected_link_portal_transition(
        bytes,
        record_offset,
        link,
        face,
        query_matrix,
        winding_flip,
        active_planes,
        viewport,
        incoming,
    )?
    else {
        return Ok(false);
    };
    worklists.propagate_and_schedule_unvisited_cell_from_pool(
        pool,
        coverage,
        destination,
        pending_slot,
        destination.spatial_index == source_spatial_index,
        pixels,
        incoming,
        path,
        process_recycled,
    )
}

/// Same-record direct-link state branches in FUN_140a85bd0: allocate an
/// unvisited cell, merge into an already queued cell, or reactivate a node
/// whose object query was deferred on the recycled chain.
#[allow(clippy::too_many_arguments)]
pub fn traverse_same_spatial_direct_link_from_pool<F>(
    bytes: &[u8],
    record_offset: u32,
    link: [u32; 4],
    direction: LinkQueryDirection,
    gate_states: Option<&[u32]>,
    query_point: [f32; 3],
    point_mode_face: Option<u8>,
    query_matrix: [[f32; 4]; 4],
    winding_flip: bool,
    active_planes: &[usize],
    viewport: [i32; 4],
    incoming: &PortalTileMask,
    spatial_index: usize,
    worklists: &mut SpatialCellWorklists,
    pool: &mut CellWorkNodePool,
    coverage: &mut VisitedCellCoverage,
    process_recycled: F,
) -> Result<bool, String>
where
    F: FnMut(CellWorkItem, TraversalCellId, &VisitedCellCoverage) -> Result<(), String>,
{
    if direct_spatial_link_target(link) != Some(spatial_index as u32) {
        return Err("same-spatial Tome link has a different destination".into());
    }
    let cell_index = (link[1] >> 16) as usize;
    let destination = TraversalCellId {
        spatial_index,
        cell_index,
    };
    let state = *worklists
        .cell_states
        .get(cell_index)
        .ok_or("same-spatial Tome link cell exceeds state table")?;
    if state & 0xc000 == 0 {
        return traverse_direct_unvisited_link_from_pool(
            bytes,
            record_offset,
            link,
            direction,
            gate_states,
            query_point,
            point_mode_face,
            query_matrix,
            winding_flip,
            active_planes,
            viewport,
            incoming,
            spatial_index,
            destination,
            0,
            worklists,
            pool,
            coverage,
            process_recycled,
        );
    }
    if !traversal_link_precheck(bytes, link, direction, gate_states)?.allows_traversal() {
        return Ok(false);
    }
    let face = admitted_link_portal_face(
        bytes,
        record_offset,
        link,
        direction,
        query_point,
        point_mode_face,
    )?;
    let Some((pixels, path)) = projected_link_portal_transition(
        bytes,
        record_offset,
        link,
        face,
        query_matrix,
        winding_flip,
        active_planes,
        viewport,
        incoming,
    )?
    else {
        return Ok(false);
    };
    let item = CellWorkItem {
        entry_id: state & CellTraversalState::ENTRY_MASK,
        cell_index: cell_index as u16,
    };
    match state & 0xc000 {
        0x4000 => worklists.reactivate_processed_active_cell(
            pool,
            coverage,
            destination,
            item,
            pixels,
            incoming,
            path,
        ),
        0x8000 => coverage.propagate_portal(destination, pixels, incoming, path),
        _ => Err("active Tome cell link state needs its current-chain branch".into()),
    }
}

/// Direct cross-record link through FUN_140a84f10's global branch-state row,
/// portal coverage, and the destination record's pending work list.
#[allow(clippy::too_many_arguments)]
pub fn traverse_cross_spatial_direct_link_from_pool<F>(
    bytes: &[u8],
    record_offset: u32,
    link: [u32; 4],
    direction: LinkQueryDirection,
    gate_states: Option<&[u32]>,
    query_point: [f32; 3],
    point_mode_face: Option<u8>,
    query_matrix: [[f32; 4]; 4],
    winding_flip: bool,
    active_planes: &[usize],
    viewport: [i32; 4],
    incoming: &PortalTileMask,
    source_spatial_index: usize,
    rows: &SpatialCellSlotRows,
    mapper: &mut SpatialCellSlotMapper,
    global_branch_states: &mut [u16],
    worklists: &mut SpatialCellWorklists,
    pool: &mut CellWorkNodePool,
    coverage: &mut VisitedCellCoverage,
    process_recycled: F,
) -> Result<bool, String>
where
    F: FnMut(CellWorkItem, TraversalCellId, &VisitedCellCoverage) -> Result<(), String>,
{
    let target_spatial = direct_spatial_link_target(link)
        .ok_or("cross-spatial Tome transition requires a direct link")?
        as usize;
    if target_spatial == source_spatial_index {
        return Err("cross-spatial Tome link points to current record".into());
    }
    if !traversal_link_precheck(bytes, link, direction, gate_states)?.allows_traversal() {
        return Ok(false);
    }
    let face = admitted_link_portal_face(
        bytes,
        record_offset,
        link,
        direction,
        query_point,
        point_mode_face,
    )?;
    let Some((pixels, path)) = projected_link_portal_transition(
        bytes,
        record_offset,
        link,
        face,
        query_matrix,
        winding_flip,
        active_planes,
        viewport,
        incoming,
    )?
    else {
        return Ok(false);
    };
    worklists.propagate_and_schedule_cross_spatial_cell_from_pool(
        mapper,
        pool,
        coverage,
        rows,
        TraversalCellId {
            spatial_index: target_spatial,
            cell_index: (link[1] >> 16) as usize,
        },
        global_branch_states,
        pixels,
        incoming,
        path,
        process_recycled,
    )
}

/// Process the next active local cell's authored direct links in source order
/// and retire its work node. FUN_140a85bd0 defers that node's object query
/// until pool pressure or the record-end recycled-list flush. Boundary-face
/// and collection-Tome paths are reported explicitly to the outer scheduler.
#[allow(clippy::too_many_arguments)]
pub fn process_next_active_direct_cell<F>(
    cells: &CellTree,
    bytes: &[u8],
    record_offset: u32,
    spatial_index: usize,
    direction: LinkQueryDirection,
    link_target_mask: &[u32],
    gate_states: Option<&[u32]>,
    query_point: [f32; 3],
    point_mode_face: Option<u8>,
    query_matrix: [[f32; 4]; 4],
    winding_flip: bool,
    active_planes: &[usize],
    rows: &SpatialCellSlotRows,
    mapper: &mut SpatialCellSlotMapper,
    global_branch_states: &mut [u16],
    worklists: &mut SpatialCellWorklists,
    pool: &mut CellWorkNodePool,
    coverage: &mut VisitedCellCoverage,
    mut process_recycled: F,
) -> Result<Option<CellWorkItem>, String>
where
    F: FnMut(CellWorkItem, TraversalCellId, &VisitedCellCoverage) -> Result<(), String>,
{
    let Some(item) = worklists.select_active_cell(cells, bytes, spatial_index, direction)? else {
        return Ok(None);
    };
    let source = TraversalCellId {
        spatial_index,
        cell_index: item.cell_index as usize,
    };
    if pool.owners[item.entry_id as usize] != Some(source) {
        return Err(format!(
            "active Tome spatial slot selected work node {} owned by {:?}, expected {source:?}",
            item.entry_id, pool.owners[item.entry_id as usize]
        ));
    }
    let incoming = coverage
        .coverage(source)
        .ok_or_else(|| {
            format!(
                "active Tome cell {} work node {} has no portal coverage",
                source.cell_index, item.entry_id
            )
        })?
        .clone();
    // FUN_140a85bd0 derives iStack_278..iStack_26c from the selected work
    // node's tile rectangle, then clamps each projected portal to those
    // bounds. A global 64-unit query viewport is too wide for later cells.
    let viewport = [
        i32::from(incoming.rect[0]) * 8,
        i32::from(incoming.rect[1]) * 4,
        i32::from(incoming.rect[2]) * 8,
        i32::from(incoming.rect[3]) * 4,
    ];
    for link in cells.traversal_records(bytes, source.cell_index)? {
        let Some(target) = direct_spatial_link_target(link) else {
            // A bit-28 boundary face merges into the query's separate
            // exterior mask node (+0x1c7e). It does not queue a local cell or
            // add Tome object indices in the ordinary single-Tome result.
            // The exterior mask output is not requested by this API.
            continue;
        };
        let target = target as usize;
        if target == spatial_index {
            traverse_same_spatial_direct_link_from_pool(
                bytes,
                record_offset,
                link,
                direction,
                gate_states,
                query_point,
                point_mode_face,
                query_matrix,
                winding_flip,
                active_planes,
                viewport,
                &incoming,
                spatial_index,
                worklists,
                pool,
                coverage,
                &mut process_recycled,
            )
            .map_err(|error| direct_cell_link_error(source, item, incoming.rect, link, error))?;
        } else {
            if link_target_mask
                .get(target / 32)
                .is_none_or(|word| word & (1 << (target % 32)) == 0)
            {
                continue;
            }
            traverse_cross_spatial_direct_link_from_pool(
                bytes,
                record_offset,
                link,
                direction,
                gate_states,
                query_point,
                point_mode_face,
                query_matrix,
                winding_flip,
                active_planes,
                viewport,
                &incoming,
                spatial_index,
                rows,
                mapper,
                global_branch_states,
                worklists,
                pool,
                coverage,
                &mut process_recycled,
            )
            .map_err(|error| direct_cell_link_error(source, item, incoming.rect, link, error))?;
        }
    }
    pool.release_processed(item, worklists.cell_states_mut())?;
    Ok(Some(item))
}

fn direct_cell_link_error(
    source: TraversalCellId,
    item: CellWorkItem,
    rect: [u8; 4],
    link: [u32; 4],
    error: String,
) -> String {
    format!(
        "cell {} node {} rect {rect:?} link {link:08x?} -> spatial {:?} cell {}: {error}",
        source.cell_index,
        item.entry_id,
        direct_spatial_link_target(link),
        link[1] >> 16
    )
}

/// Query progress survives an error, unlike its deliberately discarded output.
/// This is diagnostic state only; it never changes traversal or acceptance.
#[derive(Debug, Default)]
pub struct OrdinaryQueryDiagnostics {
    pub stage: &'static str,
    pub root_accepted: bool,
    pub global_nodes_visited: usize,
    pub link_target_records: usize,
    pub seeded_cells: Vec<[usize; 2]>,
    pub spatial_records_activated: usize,
    pub cells_processed: usize,
    pub object_passes: usize,
    pub accepted_objects: usize,
    pub accepted_cells: usize,
    pub distance_rejected_objects: usize,
    pub current_spatial_record: Option<usize>,
    pub current_object_cell: Option<usize>,
    pub active_planes: Vec<usize>,
    pub portal_rect: Option<[u8; 4]>,
}

/// Assemble the ordinary camera-point query for a direct single Tome. This
/// follows the global seed, selected spatial records, active local cells,
/// portal links, and deferred per-cell object pass. Collection/boundary links
/// and unsupported current-chain states return an error rather than yielding
/// an incomplete visibility result.
#[derive(Clone, Copy)]
pub struct OrdinaryDirectQueryInput<'a> {
    pub camera_point: [f32; 3],
    pub render_rows: [[f32; 4]; 4],
    pub cropped_rows: [[f32; 4]; 4],
    pub initial_tile_rect: [u8; 4],
    pub camera_imprecision: [f32; 3],
    pub object_imprecision: f32,
    pub minimum_area_metric: f32,
    /// Original `umbra_accurateOcclusionThreshold` value passed to the
    /// global point-tree walk. The installed game's default is 70.0.
    pub global_distance_threshold: f32,
    pub winding_flip: bool,
    pub gate_states: Option<&'a [u32]>,
    pub output_capacity: usize,
}

impl<'a> OrdinaryDirectQueryInput<'a> {
    /// FUN_140a77560/FUN_140a77c00: each worker retains the full projection
    /// for object metrics and portal rasterization, but crops its side planes
    /// and initial coverage to the assigned viewport tile.
    pub fn for_render_job(&self, job: u32, count: u32) -> Result<Self, String> {
        let tile = render_query_job_tile(job, count)?;
        Ok(Self {
            cropped_rows: crop_query_rows(self.render_rows, tile)?,
            initial_tile_rect: query_viewport_tile_bounds(tile),
            ..*self
        })
    }
}

#[derive(Debug)]
pub struct RenderQueryJobDiagnostics {
    pub job_index: u32,
    pub viewport: Option<[i32; 4]>,
    pub elapsed_ns: u128,
    pub progress: OrdinaryQueryDiagnostics,
    pub error: Option<String>,
}

/// Typed direct-Tome output descriptor. Derived planes/masks remain derived
/// from the copied rows rather than retaining the native object's padding.
/// `None` at the call site represents native `+0x598 == 0` (invalid state).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderQueryState {
    pub render_rows: [[f32; 4]; 4],
    pub cropped_rows: [[f32; 4]; 4],
    pub camera_point: [f32; 3],
    pub camera_imprecision: [f32; 3],
    pub winding_flip: bool,
    pub viewport: [i32; 4],
    pub maximum_depth: f32,
    /// Native +0x5a0: 16x16 blocks of 4x4 floats, not row-major pixels.
    pub depth_grid: Option<Box<[f32; 4096]>>,
}

impl RenderQueryState {
    /// FUN_140a788c0 copies the query descriptor before traversal, marks the
    /// requested output valid, and initializes its maximum depth to zero.
    /// The live direct-object path does not request a depth-grid output yet.
    pub fn from_input(input: &OrdinaryDirectQueryInput<'_>, viewport: [i32; 4]) -> Self {
        Self {
            render_rows: input.render_rows,
            cropped_rows: input.cropped_rows,
            camera_point: input.camera_point,
            camera_imprecision: input.camera_imprecision,
            winding_flip: input.winding_flip,
            viewport,
            maximum_depth: 0.0,
            depth_grid: None,
        }
    }
}

/// The live no-depth spatial-instance caller does not consume the area.
pub fn render_query_aabb_visible_without_depth(
    state: Option<&RenderQueryState>,
    bounds: [[f32; 3]; 2],
) -> Result<bool, String> {
    render_query_aabb_without_depth(state, bounds, false).map(|(visible, _)| visible)
}

/// FUN_140a764c0 flag 0 / FUN_140a79460's no-grid branch. An invalid
/// descriptor accepts without touching the caller's area (represented by
/// None). Descriptor bounds follow FUN_140a77c00; depth-grid projection
/// remains unsupported.
pub fn render_query_aabb_visibility_without_depth(
    state: Option<&RenderQueryState>,
    bounds: [[f32; 3]; 2],
) -> Result<(bool, Option<f32>), String> {
    render_query_aabb_without_depth(state, bounds, true)
}

/// The original descriptor prepares bounds, planes and projection once before
/// its repeated FUN_140a764c0 calls. Immutable within one merged query/frame.
pub struct RenderAabbQuery<'a> {
    camera_imprecision: [f32; 3],
    planes: [[f32; 4]; 6],
    query_bounds: [[f32; 4]; 2],
    projection_columns: [[f32; 4]; 4],
    viewport: [i32; 4],
    depth_grid: Option<&'a [f32; 4096]>,
}

/// Compatibility name for existing no-grid callers. The descriptor, not
/// the name, determines which original FUN_140a79460 branch is used.
pub type NoDepthAabbQuery<'a> = RenderAabbQuery<'a>;

impl<'a> RenderAabbQuery<'a> {
    pub fn from_state(state: &'a RenderQueryState) -> Result<Self, String> {
        let mut planes = query_planes_from_rows(state.render_rows, state.cropped_rows);
        // FUN_140a77560 normalizes only the first depth plane.
        let [x, y, z, _] = planes[0];
        let reciprocal_length = 1.0 / (y * y + x * x + z * z).sqrt();
        for coefficient in &mut planes[0] {
            *coefficient *= reciprocal_length;
        }
        if planes
            .iter()
            .flatten()
            .chain(state.camera_imprecision.iter())
            .any(|value| !value.is_finite())
        {
            return Err("nonfinite Tome spatial-query box or plane".into());
        }
        Ok(Self {
            camera_imprecision: state.camera_imprecision,
            query_bounds: query_bounds_from_planes(&planes, state.camera_imprecision)?,
            planes,
            // Native +0x40 is the transpose of full-view +0x00; job cropping
            // changes side planes, not this projection matrix.
            projection_columns: std::array::from_fn(|column| {
                std::array::from_fn(|row| state.render_rows[row][column])
            }),
            viewport: state.viewport,
            depth_grid: state.depth_grid.as_deref(),
        })
    }

    fn expanded_bounds(&self, bounds: [[f32; 3]; 2]) -> Result<[[f32; 4]; 2], String> {
        let expanded: [[f32; 4]; 2] = std::array::from_fn(|side| {
            std::array::from_fn(|axis| {
                if axis == 3 {
                    1.0
                } else if side == 0 {
                    bounds[side][axis] - self.camera_imprecision[axis]
                } else {
                    bounds[side][axis] + self.camera_imprecision[axis]
                }
            })
        });
        if expanded.iter().flatten().any(|value| !value.is_finite()) {
            return Err("nonfinite Tome spatial-query box or plane".into());
        }
        Ok(expanded)
    }

    pub fn visible(&self, bounds: [[f32; 3]; 2]) -> Result<bool, String> {
        if self.depth_grid.is_some() {
            return self.visibility_and_area(bounds).map(|(visible, _)| visible);
        }
        // No-grid descriptors keep the original fast predicate: unused area
        // cannot affect visibility, so no projection is needed here.
        query_bounds_accepts_aabb(
            self.query_bounds,
            self.expanded_bounds(bounds)?,
            &self.planes,
            &[0, 1, 2, 3, 4, 5],
        )
    }

    pub fn visibility_and_area(&self, bounds: [[f32; 3]; 2]) -> Result<(bool, f32), String> {
        let expanded = self.expanded_bounds(bounds)?;
        if let Some(grid) = self.depth_grid {
            query_aabb_visibility_with_depth(
                self.query_bounds, &self.planes, expanded,
                self.projection_columns, self.viewport, grid,
            )
        } else {
            query_aabb_visibility_without_depth(
                self.query_bounds, &self.planes, expanded, self.projection_columns,
            )
        }
    }
}

fn render_query_aabb_without_depth(
    state: Option<&RenderQueryState>,
    bounds: [[f32; 3]; 2],
    area_requested: bool,
) -> Result<(bool, Option<f32>), String> {
    let Some(state) = state else {
        return Ok((true, None));
    };
    let query = NoDepthAabbQuery::from_state(state)?;
    if area_requested {
        query
            .visibility_and_area(bounds)
            .map(|(visible, area)| (visible, Some(area)))
    } else {
        query.visible(bounds).map(|visible| (visible, None))
    }
}

/// Clip the expanded box exactly once for either FUN_140a79460 branch.
#[cfg(target_arch = "x86_64")]
fn clipped_query_aabb(
    query_bounds: [[f32; 4]; 2],
    planes: &[[f32; 4]],
    expanded: [[f32; 4]; 2],
    rows: [[f32; 4]; 4],
) -> Result<Option<[[f32; 4]; 2]>, String> {
    if query_bounds
        .iter()
        .flatten()
        .chain(expanded.iter().flatten())
        .chain(planes.iter().flatten())
        .chain(rows.iter().flatten())
        .any(|value| !value.is_finite())
        || (0..4).any(|axis| expanded[0][axis] > expanded[1][axis])
    {
        return Err("invalid Tome projected query box input".into());
    }
    let all_planes = (0..planes.len()).collect::<Vec<_>>();
    if !query_bounds_accepts_aabb(query_bounds, expanded, planes, &all_planes)? {
        return Ok(None);
    }
    let active = active_query_planes(planes, expanded);
    let clipped = if active.is_empty() {
        expanded
    } else {
        clip_box_to_active_query_planes(query_bounds, expanded, planes, &active)?
    };
    Ok(Some(clipped))
}

/// Preserve native SSE reciprocal and extent-addition order. The no-grid
/// branch has W-crossing masks; the depth branch deliberately does not.
#[cfg(target_arch = "x86_64")]
fn project_clipped_query_aabb(
    clipped: [[f32; 4]; 2],
    rows: [[f32; 4]; 4],
    handle_w_crossing: bool,
) -> [f32; 4] {
    use std::arch::x86_64::*;
    // Keep the native operation order: z*row2 + row3, then y*row1,
    // then x*row0; the other corners add the three extent vectors.
    let corners = unsafe {
        let matrix = rows.map(|row| _mm_loadu_ps(row.as_ptr()));
        let extent: [f32; 3] = std::array::from_fn(|axis| clipped[1][axis] - clipped[0][axis]);
        let dx = _mm_mul_ps(_mm_set1_ps(extent[0]), matrix[0]);
        let dy = _mm_mul_ps(_mm_set1_ps(extent[1]), matrix[1]);
        let dz = _mm_mul_ps(_mm_set1_ps(extent[2]), matrix[2]);
        let base = _mm_add_ps(
            _mm_add_ps(
                _mm_add_ps(_mm_mul_ps(_mm_set1_ps(clipped[0][2]), matrix[2]), matrix[3]),
                _mm_mul_ps(_mm_set1_ps(clipped[0][1]), matrix[1]),
            ),
            _mm_mul_ps(_mm_set1_ps(clipped[0][0]), matrix[0]),
        );
        let x = _mm_add_ps(dx, base);
        let y = _mm_add_ps(base, dy);
        let xy = _mm_add_ps(x, dy);
        [
            base,
            x,
            xy,
            y,
            _mm_add_ps(dz, base),
            _mm_add_ps(dz, x),
            _mm_add_ps(dz, xy),
            _mm_add_ps(dz, y),
        ]
        .map(|corner| {
            let mut values = [0.0; 4];
            _mm_storeu_ps(values.as_mut_ptr(), corner);
            values
        })
    };
    let crossing = handle_w_crossing && corners.iter().any(|corner| corner[3] < 0.0);
    let minps = |a: f32, b: f32| if a < b { a } else { b };
    let maxps = |a: f32, b: f32| if a > b { a } else { b };
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    for corner in corners {
        let w = corner[3];
        let denominator = if w == 0.0 { 1.0 } else { w };
        let reciprocal = unsafe { _mm_cvtss_f32(_mm_rcp_ps(_mm_set1_ps(denominator))) };
        for axis in 0..2 {
            let coordinate = corner[axis];
            let projected = coordinate * reciprocal;
            let (minimum, maximum) = if !crossing {
                (projected, projected)
            } else {
                // A corner behind W=0 can span [-1,1]. This is the
                // executable's mask branch, not generic edge clipping.
                let inside_low = -w < coordinate;
                let inside_high = coordinate < w;
                (
                    if inside_low {
                        if inside_high {
                            projected
                        } else {
                            1.0
                        }
                    } else {
                        -1.0
                    },
                    if inside_high {
                        if inside_low {
                            projected
                        } else {
                            -1.0
                        }
                    } else {
                        1.0
                    },
                )
            };
            low[axis] = minps(low[axis], minimum);
            high[axis] = maxps(high[axis], maximum);
        }
    }
    [low[0], low[1], high[0], high[1]]
}

/// FUN_140a79460's no-grid branch, including caller-visible projected area.
#[cfg(target_arch = "x86_64")]
pub fn query_aabb_visibility_without_depth(
    query_bounds: [[f32; 4]; 2],
    planes: &[[f32; 4]],
    expanded: [[f32; 4]; 2],
    rows: [[f32; 4]; 4],
) -> Result<(bool, f32), String> {
    let Some(clipped) = clipped_query_aabb(query_bounds, planes, expanded, rows)? else {
        return Ok((false, 0.0));
    };
    let rectangle = project_clipped_query_aabb(clipped, rows, true);
    Ok((true, projected_object_area_metric(rectangle)))
}

/// FUN_140a79460 +0x590 != 0: nearest-Z corner, near-plane acceptance,
/// native reciprocal projection and biased pixel rectangle, then 784c0.
/// This consumes Umbra query depth, not a substituted GPU Z-buffer.
#[cfg(target_arch = "x86_64")]
pub fn query_aabb_visibility_with_depth(
    query_bounds: [[f32; 4]; 2],
    planes: &[[f32; 4]],
    expanded: [[f32; 4]; 2],
    rows: [[f32; 4]; 4],
    viewport: [i32; 4],
    depths: &[f32; 4096],
) -> Result<(bool, f32), String> {
    let Some(clipped) = clipped_query_aabb(query_bounds, planes, expanded, rows)? else {
        return Ok((false, 0.0));
    };
    // Descriptor +0x340 marks strictly positive depth coefficients. Zero
    // (including -0) selects the upper bound, too: although it contributes
    // nothing to Z, that same corner's X/Y still contribute to clip W.
    let nearest: [f32; 4] = std::array::from_fn(|axis| {
        clipped[usize::from(!(rows[axis][2] > 0.0))][axis]
    });
    let dot = |component: usize| {
        ((nearest[1] * rows[1][component] + nearest[0] * rows[0][component])
            + nearest[2] * rows[2][component]) + nearest[3] * rows[3][component]
    };
    let z = dot(2);
    let threshold = if z > 0.0 { z / dot(3) } else { 0.0 };
    if threshold == 0.0 {
        return Ok((true, 1.0));
    }
    let rectangle = project_clipped_query_aabb(clipped, rows, false);
    let area = projected_object_area_metric(rectangle);
    // 141e68090/141e680a0: raw projection is converted before viewport
    // intersection; only the area, not these inputs, clamps to [-1,1].
    // CVTTPS2DQ at 140a79c57 returns INT_MIN for overflow/NaN;
    // Rust's saturating `as i32` is not equivalent at a W crossing.
    let pixel = unsafe {
        use std::arch::x86_64::*;
        let bias = [32.0, 32.0, f32::from_bits(0x4203_ffff), f32::from_bits(0x4203_ffff)];
        let scaled = _mm_add_ps(
            _mm_mul_ps(_mm_loadu_ps(rectangle.as_ptr()), _mm_set1_ps(32.0)),
            _mm_loadu_ps(bias.as_ptr()),
        );
        let mut pixel = [0; 4];
        _mm_storeu_si128(pixel.as_mut_ptr().cast(), _mm_cvttps_epi32(scaled));
        pixel
    };
    let visible = depth_grid_intersects_pixels(
        depths,
        [pixel[0].max(viewport[0]), pixel[1].max(viewport[1])],
        [pixel[2].min(viewport[2]), pixel[3].min(viewport[3])],
        threshold,
    )?;
    Ok((visible, area))
}

#[cfg(not(target_arch = "x86_64"))]
pub fn query_aabb_visibility_without_depth(
    _: [[f32; 4]; 2],
    _: &[[f32; 4]],
    _: [[f32; 4]; 2],
    _: [[f32; 4]; 4],
) -> Result<(bool, f32), String> {
    Err("original Tome projected box query requires x86_64 SSE".into())
}

#[cfg(not(target_arch = "x86_64"))]
pub fn query_aabb_visibility_with_depth(
    _: [[f32; 4]; 2], _: &[[f32; 4]], _: [[f32; 4]; 2],
    _: [[f32; 4]; 4], _: [i32; 4], _: &[f32; 4096],
) -> Result<(bool, f32), String> {
    Err("original Tome projected box query requires x86_64 SSE".into())
}

/// FUN_140377440 scales an inclusive renderer rectangle; FUN_140374b60
/// derives +0x7848/+0x784c from its resulting extents. Finite, positive
/// scale only; CRT exceptional-input handling remains unsupported.
pub fn scaled_render_viewport(rect: [i32; 4], scale: [f32; 2]) -> Result<[i32; 4], String> {
    let width = rect[2].checked_sub(rect[0]).and_then(|v| v.checked_add(1));
    let height = rect[3].checked_sub(rect[1]).and_then(|v| v.checked_add(1));
    let (Some(width), Some(height)) = (width, height) else {
        return Err("original render viewport extent overflow".into());
    };
    if width <= 0 || height <= 0 || scale.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return Err("invalid original render viewport or scale".into());
    }
    let rounded = |v: i32, scale: f32| -> Result<i32, String> {
        let value = (v as f32 * scale + 0.5).floor();
        if !value.is_finite() || value < i32::MIN as f32 || value >= 2147483648.0 {
            return Err("original scaled render viewport coordinate overflow".into());
        }
        Ok(value as i32)
    };
    let left = rounded(rect[0], scale[0])?;
    let top = rounded(rect[1], scale[1])?;
    let width = rounded(width, scale[0])?;
    let height = rounded(height, scale[1])?;
    let right = width.checked_sub(1).and_then(|v| left.checked_add(v));
    let bottom = height.checked_sub(1).and_then(|v| top.checked_add(v));
    match (right, bottom) {
        (Some(right), Some(bottom)) => Ok([left, top, right, bottom]),
        _ => Err("original scaled render viewport extent overflow".into()),
    }
}

pub fn render_viewport_dimensions(rect: [i32; 4]) -> Result<[i32; 2], String> {
    let width = rect[2].checked_sub(rect[0]).and_then(|v| v.checked_add(1));
    let height = rect[3].checked_sub(rect[1]).and_then(|v| v.checked_add(1));
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => Ok([width, height]),
        _ => Err("invalid original render viewport dimensions".into()),
    }
}

/// FUN_14031bcc0 loads DAT_141ac8fec (64.0), then divides by the
/// caller's +0x7848/+0x784c viewport product before comparing the area.
/// These are caller-supplied dimensions, not an assumed physical-pixel scale.
pub fn minimum_light_projected_area_metric(width: i32, height: i32) -> Result<f32, String> {
    if width <= 0 || height <= 0 {
        return Err("invalid Tome light query viewport dimensions".into());
    }
    let pixels = width
        .checked_mul(height)
        .ok_or("Tome light query viewport product overflow")?;
    Ok(64.0 / pixels as f32)
}

/// FUN_140a71980 / FUN_140a6ff70 / FUN_140a78d70: ignore invalid sources,
/// deep-copy the first valid descriptor/grid, then union viewports and merge
/// only the incoming tile's depth blocks. Native MAXPS chooses its second
/// operand for equal or unordered lanes; f32::max would change NaN/zero bits.
pub fn merge_render_query_state(
    destination: &mut Option<RenderQueryState>,
    source: Option<&RenderQueryState>,
) -> Result<(), String> {
    let Some(source) = source else {
        return Ok(());
    };
    let Some(destination) = destination else {
        *destination = Some(source.clone());
        return Ok(());
    };
    let viewport = [
        destination.viewport[0].min(source.viewport[0]),
        destination.viewport[1].min(source.viewport[1]),
        destination.viewport[2].max(source.viewport[2]),
        destination.viewport[3].max(source.viewport[3]),
    ];
    let cropped_rows = crop_query_rows(destination.render_rows, viewport)?;
    if destination.depth_grid.is_some() && source.depth_grid.is_some() {
        let [left, top, right, bottom] = source.viewport;
        if left < 0
            || top < 0
            || right > 64
            || bottom > 64
            || left >= right
            || top >= bottom
            || left % 8 != 0
            || right % 8 != 0
            || top % 4 != 0
            || bottom % 4 != 0
        {
            return Err("unsupported Tome depth-merge tile alignment".into());
        }
    }
    destination.viewport = viewport;
    destination.cropped_rows = cropped_rows;
    // The scalar original comparison differs from MAXPS: an unordered
    // maximum leaves the destination unchanged, including its NaN payload.
    if destination.maximum_depth < source.maximum_depth {
        destination.maximum_depth = source.maximum_depth;
    }
    if let (Some(destination_grid), Some(source_grid)) =
        (&mut destination.depth_grid, &source.depth_grid)
    {
        let [left, top, right, bottom] = source.viewport;
        for block_y in top / 4..bottom / 4 {
            for block_x in left / 4..right / 4 {
                let first = (block_y * 16 + block_x) as usize * 16;
                for i in first..first + 16 {
                    let previous = destination_grid[i];
                    let incoming = source_grid[i];
                    destination_grid[i] = if previous > incoming {
                        previous
                    } else {
                        incoming
                    };
                }
            }
        }
    }
    Ok(())
}

pub struct RenderQueryJob {
    pub diagnostics: RenderQueryJobDiagnostics,
    pub state: Option<RenderQueryState>,
    pub result: Result<Option<VisibleObjectIndexOutput>, String>,
}

/// FUN_14037e1d0's supported ordinary, direct-Tome world-object/cell query.
/// The later light query, collection traversal and depth-buffer output remain
/// unsupported by this path. A failed query exposes progress, never partial indices.
pub fn ordinary_direct_tome_render_job(
    tree: &SpatialTree,
    bytes: &[u8],
    data: &DirectTomeQueryData,
    input: &OrdinaryDirectQueryInput<'_>,
    job: u32,
    count: u32,
) -> RenderQueryJob {
    let started = std::time::Instant::now();
    let mut diagnostics = RenderQueryJobDiagnostics {
        job_index: job,
        viewport: None,
        elapsed_ns: 0,
        progress: OrdinaryQueryDiagnostics::default(),
        error: None,
    };
    let mut state = None;
    let result = input.for_render_job(job, count).and_then(|job_input| {
        let viewport = render_query_job_tile(job, count)?;
        diagnostics.viewport = Some(viewport);
        state = Some(RenderQueryState::from_input(&job_input, viewport));
        ordinary_direct_tome_visible_objects_cached_with_diagnostics(
            tree,
            bytes,
            data,
            &job_input,
            &mut diagnostics.progress,
        )
    });
    diagnostics.elapsed_ns = started.elapsed().as_nanos();
    diagnostics.error = result.as_ref().err().cloned();
    RenderQueryJob {
        diagnostics,
        state,
        result,
    }
}

/// FUN_14037ef90 appends per-job world-index lists in job-index order through
/// FUN_1407db560, which grows the destination for the combined count. The
/// 0x10000 query capacity is per worker, not a cap on this merged list. Do not
/// deduplicate across jobs: the original bitset is local to each query, and
/// texture demand consumes their aligned full-view projected-area metrics.
#[derive(Default)]
pub struct RenderQueryObjectOutput {
    indices: Vec<u32>,
    area_metrics: Vec<Option<f32>>,
    cell_indices: Vec<u32>,
    pub state: Option<RenderQueryState>,
}

impl RenderQueryObjectOutput {
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }
    pub fn area_metrics(&self) -> &[Option<f32>] {
        &self.area_metrics
    }
    pub fn cell_indices(&self) -> &[u32] {
        &self.cell_indices
    }
}

pub fn merge_render_query_outputs(
    jobs: &[RenderQueryJob],
) -> Result<Option<RenderQueryObjectOutput>, String> {
    let mut output = None;
    let mut state = None;
    for (expected, job) in jobs.iter().enumerate() {
        if job.diagnostics.job_index as usize != expected {
            return Err("Tome render-query jobs are not in dispatch order".into());
        }
        // FUN_14037ef90 merges every initialized worker descriptor, even
        // when that worker's object/cell lists were cleared after an error.
        merge_render_query_state(&mut state, job.state.as_ref())?;
        match &job.result {
            Err(_) => {
                // FUN_140166830 returns; after 0x14037e9cb the failed worker's
                // lists are cleared, while successful siblings still merge.
                output.get_or_insert_with(RenderQueryObjectOutput::default);
            }
            Ok(None) => {}
            Ok(Some(job_output)) => {
                let output = output.get_or_insert_with(RenderQueryObjectOutput::default);
                output.indices.extend_from_slice(job_output.indices());
                output
                    .area_metrics
                    .extend_from_slice(job_output.area_metrics());
                if let Some(cells) = job_output.cell_output() {
                    // Native lists concatenate in dispatch order. Duplicate
                    // cells across jobs remain until the later mask builder.
                    output.cell_indices.extend_from_slice(cells.indices());
                }
            }
        }
    }
    if output.is_none() && state.is_some() {
        output = Some(RenderQueryObjectOutput::default());
    }
    if let Some(output) = &mut output {
        output.state = state;
    }
    Ok(output)
}

/// Default registered by the installed game for `umbra_accurateOcclusionThreshold`.
pub const ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD: f32 = 70.0;

pub fn ordinary_direct_tome_visible_objects(
    tree: &SpatialTree,
    bytes: &[u8],
    input: &OrdinaryDirectQueryInput<'_>,
) -> Result<Option<VisibleObjectIndexOutput>, String> {
    ordinary_direct_tome_visible_objects_with_diagnostics(
        tree,
        bytes,
        input,
        &mut OrdinaryQueryDiagnostics::default(),
    )
}

pub fn ordinary_direct_tome_visible_objects_with_diagnostics(
    tree: &SpatialTree,
    bytes: &[u8],
    input: &OrdinaryDirectQueryInput<'_>,
    diagnostics: &mut OrdinaryQueryDiagnostics,
) -> Result<Option<VisibleObjectIndexOutput>, String> {
    ordinary_direct_tome_visible_objects_inner(tree, bytes, input, None, diagnostics)
}

pub fn ordinary_direct_tome_visible_objects_cached_with_diagnostics(
    tree: &SpatialTree,
    bytes: &[u8],
    data: &DirectTomeQueryData,
    input: &OrdinaryDirectQueryInput<'_>,
    diagnostics: &mut OrdinaryQueryDiagnostics,
) -> Result<Option<VisibleObjectIndexOutput>, String> {
    ordinary_direct_tome_visible_objects_inner(tree, bytes, input, Some(data), diagnostics)
}

fn ordinary_direct_tome_visible_objects_inner(
    tree: &SpatialTree,
    bytes: &[u8],
    input: &OrdinaryDirectQueryInput<'_>,
    data: Option<&DirectTomeQueryData>,
    diagnostics: &mut OrdinaryQueryDiagnostics,
) -> Result<Option<VisibleObjectIndexOutput>, String> {
    *diagnostics = OrdinaryQueryDiagnostics::default();
    diagnostics.stage = "setup";
    let planes = query_planes_from_rows(input.render_rows, input.cropped_rows);
    let stored_matrix = stored_query_matrix_from_render_rows(input.render_rows);
    let initial_mask = PortalTileMask::full(input.initial_tile_rect)?;
    let parsed_rows;
    let rows = if let Some(data) = data {
        &data.slot_rows
    } else {
        parsed_rows = SpatialCellSlotRows::parse(bytes)?;
        &parsed_rows
    };
    let mut worklists = SpatialCellWorklists::new_for_original_query();
    let mut pool = CellWorkNodePool::new_for_original_query()?;
    let mut coverage = VisitedCellCoverage::default();
    diagnostics.stage = "global_seed";
    let Some(mut global) = tree.seed_ordinary_camera_global_query(
        bytes,
        data,
        input.camera_point,
        &planes,
        input.camera_imprecision,
        input.global_distance_threshold,
        &mut worklists,
        &mut pool,
        &mut coverage,
        &initial_mask,
    )?
    else {
        diagnostics.stage = "outside_tome_root";
        return Err("Tome query input outside root bounds (error 5)".into());
    };
    diagnostics.root_accepted = true;
    diagnostics.global_nodes_visited = global.nodes_visited;
    diagnostics.link_target_records = global
        .link_target_mask
        .iter()
        .map(|word| word.count_ones() as usize)
        .sum();
    diagnostics.seeded_cells = global
        .seeded_records
        .iter()
        .map(|seed| [seed.spatial_node_index, seed.selection.cell_index])
        .collect();
    diagnostics.stage = "spatial_activation";
    let mut active_tree_nodes = global.link_target_mask.clone();
    let mut mapper = SpatialCellSlotMapper::default();
    let object_count = match data {
        Some(data) => data.object_count,
        None => word(bytes, 0x40)? as usize,
    };
    let mut output = VisibleObjectIndexOutput::new(object_count, input.output_capacity);
    let cell_count = match data {
        Some(data) => data.cell_count,
        None => word(bytes, 0x7c)? as usize,
    };
    output.cells = Some(VisibleCellIndexOutput::new(
        cell_count,
        ORIGINAL_VISIBLE_CELL_OUTPUT_CAPACITY,
    ));
    while let Some(activation) = global.activate_next_spatial_record(
        tree,
        input.camera_point,
        &mut active_tree_nodes,
        &mut worklists,
    )? {
        if activation.activated_cells == 0 {
            continue;
        }
        diagnostics.spatial_records_activated += 1;
        diagnostics.current_spatial_record = Some(activation.spatial_node_index);
        diagnostics.stage = "record_setup";
        worklists.bind_active_cells_to_spatial_record(
            activation.spatial_node_index,
            &mut pool,
            &mut coverage,
        )?;
        let record_offset = tree
            .record_offset(activation.spatial_node_index)
            .filter(|&offset| offset != 0)
            .ok_or("active Tome spatial node has no record")?;
        let parsed_cells;
        let cells = if let Some(data) = data {
            data.cells(activation.spatial_node_index)?
        } else {
            parsed_cells = CellTree::parse(bytes, record_offset)?;
            &parsed_cells
        };
        let setup = setup_spatial_record_query(
            bytes,
            record_offset,
            input.camera_point,
            input.camera_imprecision,
            &planes,
        )?;
        diagnostics.active_planes.clone_from(&setup.active_planes);
        diagnostics.stage = "cell_traversal";
        let direction = LinkQueryDirection::Volume {
            center: setup.link_center,
            tolerance: setup.link_tolerance,
        };
        let mut process_recycled = |item: CellWorkItem,
                                    owner: TraversalCellId,
                                    state: &VisitedCellCoverage|
         -> Result<(), String> {
            if owner.spatial_index != activation.spatial_node_index {
                return Err("recycled Tome node belongs to another spatial record".into());
            }
            let portal = state
                .coverage(owner)
                .ok_or("recycled Tome node has no portal coverage")?;
            diagnostics.stage = "object_pass";
            diagnostics.current_object_cell = Some(item.cell_index as usize);
            diagnostics.portal_rect = Some(portal.rect);
            diagnostics.object_passes += 1;
            let query = CellObjectVisibilityQuery {
                record_query_bounds: setup.bounds,
                query_planes: &planes,
                active_planes: &setup.active_planes,
                projection_rows: stored_matrix,
                camera_translation_imprecision: input.object_imprecision,
                object_distance_point: input.camera_point,
                object_distance_scale_squared: [ORIGINAL_DEFAULT_OBJECT_DISTANCE_SCALE_SQUARED; 4],
                minimum_area_metric: input.minimum_area_metric,
                viewport_tile_bounds: portal.rect,
                portal,
                depth_grid: None,
            };
            let result = cells
                .append_visible_object_candidates(
                    bytes,
                    item.cell_index as usize,
                    &query,
                    &mut output,
                )
                .and_then(|()| {
                    cells.append_visible_cell_indices(
                        bytes,
                        item.cell_index as usize,
                        None,
                        output.cells.as_mut(),
                    )
                });
            diagnostics.accepted_cells = output
                .cells
                .as_ref()
                .map_or(0, VisibleCellIndexOutput::count);
            diagnostics.accepted_objects = output.count();
            diagnostics.distance_rejected_objects = output.distance_rejected_objects;
            result.map_err(|error| {
                format!(
                    "spatial {} cell {} object pass: {error}",
                    owner.spatial_index, item.cell_index
                )
            })?;
            diagnostics.stage = "cell_traversal";
            Ok(())
        };
        let mut cells_processed = 0;
        let record_result = (|| -> Result<(), String> {
            while process_next_active_direct_cell(
                cells,
                bytes,
                record_offset,
                activation.spatial_node_index,
                direction,
                &active_tree_nodes,
                input.gate_states,
                input.camera_point,
                None,
                stored_matrix,
                input.winding_flip,
                &setup.active_planes,
                rows,
                &mut mapper,
                &mut global.branch_states,
                &mut worklists,
                &mut pool,
                &mut coverage,
                &mut process_recycled,
            )
            .map_err(|error| {
                format!(
                    "spatial {} direct pass: {error}",
                    activation.spatial_node_index
                )
            })?
            .is_some()
            {
                cells_processed += 1;
            }
            pool.flush_recycled(
                worklists.cell_states_mut(),
                &mut coverage,
                &mut process_recycled,
            )?;
            Ok(())
        })();
        diagnostics.cells_processed += cells_processed;
        record_result?;
        diagnostics.stage = "spatial_activation";
    }
    diagnostics.stage = "complete";
    diagnostics.accepted_objects = output.count();
    diagnostics.distance_rejected_objects = output.distance_rejected_objects;
    Ok(Some(output))
}

/// Transform a world-space portal corner with the row-major matrix stored in
/// the Umbra query context at +0x40..+0x7c. FUN_140a85bd0 computes each output
/// component as x*M[0][j] + y*M[1][j] + z*M[2][j] + M[3][j]. This is the
/// projective clip-space input used before portal rectangle clipping; it does
/// not perform the original clipping or viewport conversion itself.
pub fn transform_portal_corner_by_query_matrix(
    point: [f32; 3],
    matrix: [[f32; 4]; 4],
) -> Result<[f32; 4], String> {
    if point
        .iter()
        .chain(matrix.iter().flatten())
        .any(|value| !value.is_finite())
    {
        return Err("nonfinite Tome portal projection input".into());
    }
    let projected = std::array::from_fn(|column| {
        point[0] * matrix[0][column]
            + point[1] * matrix[1][column]
            + point[2] * matrix[2][column]
            + matrix[3][column]
    });
    if projected.iter().any(|value| !value.is_finite()) {
        return Err("nonfinite projected Tome portal corner".into());
    }
    Ok(projected)
}

/// Build the exact four SIMD vectors that FUN_140a85bd0 places at
/// RBP+0x120..0x150 before calling FUN_140a8a080: four corner X values,
/// four corner Y values, four corner W values, and the projected normal
/// extent. `face_code` is the 0..5
/// face selector assembled in EBX; `winding_flip` is query-context +0x4b4.
/// The caller supplies the already padded low/high world bounds and the
/// query matrix at +0x40. This ports the caller-side corner construction;
/// it does not run the Tome traversal or rasterizer.
pub fn portal_raster_inputs_from_bounds(
    bounds: [[f32; 3]; 2],
    face_code: u8,
    matrix: [[f32; 4]; 4],
    winding_flip: bool,
) -> Result<([[f32; 4]; 4], [[f32; 4]; 3]), String> {
    if face_code >= 6
        || bounds
            .iter()
            .flatten()
            .chain(matrix.iter().flatten())
            .any(|v| !v.is_finite())
        || (0..3).any(|axis| bounds[0][axis] > bounds[1][axis])
    {
        return Err("invalid Tome portal raster inputs".into());
    }

    // The original SSE path starts from the low corner, forms each extent
    // vector, then selects the two cyclic transverse axes for this face.
    let base: [f32; 4] = std::array::from_fn(|column| {
        ((bounds[0][2] * matrix[2][column] + matrix[3][column]) + bounds[0][1] * matrix[1][column])
            + bounds[0][0] * matrix[0][column]
    });
    let extents: [[f32; 4]; 3] = std::array::from_fn(|axis| {
        let extent = bounds[1][axis] - bounds[0][axis];
        std::array::from_fn(|column| extent * matrix[axis][column])
    });

    let axis = (face_code / 2) as usize;
    let mut u = extents[(axis + 1) % 3];
    let mut v = extents[(axis + 2) % 3];
    if (face_code & 1 != 0) != winding_flip {
        std::mem::swap(&mut u, &mut v);
    }

    // FUN_140a85bd0 places the low-face cycle into three four-lane X/Y/W
    // vectors and the perpendicular extent into the fourth vector. The
    // rasterizer selects the parallel face per edge using that extent.
    let normal = extents[axis];
    let low_u: [f32; 4] = std::array::from_fn(|lane| base[lane] + u[lane]);
    let low_uv: [f32; 4] = std::array::from_fn(|lane| low_u[lane] + v[lane]);
    let low_v: [f32; 4] = std::array::from_fn(|lane| base[lane] + v[lane]);
    let corners = [base, low_u, low_uv, low_v];
    let projected = [
        std::array::from_fn(|corner| corners[corner][0]),
        std::array::from_fn(|corner| corners[corner][1]),
        std::array::from_fn(|corner| corners[corner][3]),
        normal,
    ];
    let face = portal_face_vectors_from_query_matrix(matrix, winding_flip)?[face_code as usize];
    if projected.iter().flatten().any(|v| !v.is_finite()) {
        return Err("nonfinite projected Tome portal corner".into());
    }
    Ok((projected, face))
}

/// Build FUN_140a81500's six query-face vector rows from the matrix stored at
/// query context +0x40. The original stores three float4 vectors per face in
/// the +0x13e table. `winding_flip` is the query-context byte at +0x4b4.
/// These rows are consumed by FUN_140a8a080 when a clipped portal is rasterized.
pub fn portal_face_vectors_from_query_matrix(
    matrix: [[f32; 4]; 4],
    winding_flip: bool,
) -> Result<[[[f32; 4]; 3]; 6], String> {
    if matrix.iter().flatten().any(|value| !value.is_finite()) {
        return Err("nonfinite Tome portal query matrix".into());
    }

    // FUN_140a81500 reads columns x, y, and w from the first three stored
    // matrix rows, then writes the three cyclic cross products to its stack
    // cofactor table. Preserve the executable's scalar multiply/subtract order.
    let rows =
        std::array::from_fn::<_, 3, _>(|row| [matrix[row][0], matrix[row][1], matrix[row][3]]);
    let [a, b, c] = rows;
    let cofactors = [
        [
            b[1] * c[2] - b[2] * c[1],
            c[0] * b[2] - c[2] * b[0],
            b[0] * c[1] - b[1] * c[0],
        ],
        [
            c[1] * a[2] - c[2] * a[1],
            c[2] * a[0] - c[0] * a[2],
            c[0] * a[1] - c[1] * a[0],
        ],
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ],
    ];
    if cofactors.iter().flatten().any(|value| !value.is_finite()) {
        return Err("nonfinite Tome portal face cofactor".into());
    }

    let sign_flip = |value: f32| f32::from_bits(value.to_bits() ^ 0x8000_0000);
    let mut faces = [[[0.0; 4]; 3]; 6];
    // FUN_140a81500 rotates the one-hot axis sequence 1, 2, 0 and pairs it
    // with (1 << axis) & 3, yielding (1,2), (2,0), and (0,1).
    for (pair, (current, next)) in [(1usize, 2usize), (2, 0), (0, 1)].into_iter().enumerate() {
        let mut primary = [[0.0; 4]; 3];
        let mut opposite = [[0.0; 4]; 3];
        for component in 0..3 {
            let current_value = cofactors[current][component];
            let next_value = cofactors[next][component];
            primary[component] = [
                current_value,
                sign_flip(next_value),
                sign_flip(current_value),
                next_value,
            ];
            opposite[component] = [
                sign_flip(next_value),
                current_value,
                next_value,
                sign_flip(current_value),
            ];
        }

        // The byte at query +0x4b4 selects which member of each face pair
        // receives the primary versus opposite vector ordering.
        let even = pair * 2;
        let (even_vectors, odd_vectors) = if winding_flip {
            (primary, opposite)
        } else {
            (opposite, primary)
        };
        faces[even] = even_vectors;
        faces[even + 1] = odd_vectors;
    }
    Ok(faces)
}

/// FUN_140a85bd0 converts its clipped portal pixel rectangle to the tile
/// rectangle supplied to FUN_140a89d80 (eight pixels wide, four high).
///
/// Before that tile conversion, FUN_140a85bd0 maps the clipped NDC rectangle
/// to viewport pixels with the vectors at 0x141e68090 and 0x141e680a0. The
/// first vector is [32, 32, 32, 32]; the second is [32, 32, 32.999996,
/// 32.999996]. CVTTPS2DQ truncates the four results, then the function clamps
/// the minimum edges upward and maximum edges downward to its query viewport.
pub fn projected_portal_pixel_rect(
    projected: [f32; 4],
    viewport: [i32; 4],
) -> Result<Option<[i32; 4]>, String> {
    if projected.iter().any(|value| !value.is_finite())
        || viewport[0] > viewport[2]
        || viewport[1] > viewport[3]
    {
        return Err("invalid projected Tome portal rectangle or viewport".into());
    }
    let [min_x, min_y, max_x, max_y] = projected;
    let pixels = [
        min_x * 32.0 + 32.0,
        min_y * 32.0 + 32.0,
        max_x * 32.0 + f32::from_bits(0x4203_ffff),
        max_y * 32.0 + f32::from_bits(0x4203_ffff),
    ];
    if pixels
        .iter()
        .any(|value| !value.is_finite() || *value < i32::MIN as f32 || *value >= i32::MAX as f32)
    {
        return Err("projected Tome portal pixel coordinate exceeds integer range".into());
    }
    let mut pixels = pixels.map(|value| value.trunc() as i32);
    pixels[0] = pixels[0].max(viewport[0]);
    pixels[1] = pixels[1].max(viewport[1]);
    pixels[2] = pixels[2].min(viewport[2]);
    pixels[3] = pixels[3].min(viewport[3]);
    Ok((pixels[2] > pixels[0] && pixels[3] > pixels[1]).then_some(pixels))
}

/// FUN_140a85bd0's fallback extrema branch (LAB_140a866e5) projects both
/// parallel faces of the padded portal box, then reduces their eight
/// homogeneous corners to [min x, min y, max x, max y]. The active-plane
/// crossing branch must not use this helper; it adds edge-derived extrema.
#[cfg(target_arch = "x86_64")]
pub fn projected_portal_box_fallback_bounds(corners: [[f32; 4]; 8]) -> Result<[f32; 4], String> {
    use std::arch::x86_64::*;

    if corners.iter().flatten().any(|value| !value.is_finite()) {
        return Err("nonfinite homogeneous Tome portal box corner".into());
    }

    let mut ndc = [[0.0f32; 2]; 8];
    for group in 0..2 {
        let base = group * 4;
        let mut x_lanes = [0.0f32; 4];
        let mut y_lanes = [0.0f32; 4];
        unsafe {
            let w = _mm_setr_ps(
                corners[base][3],
                corners[base + 1][3],
                corners[base + 2][3],
                corners[base + 3][3],
            );
            let zero_mask = _mm_cmpeq_ps(w, _mm_setzero_ps());
            // FUN_140a85bd0 substitutes 1.0 for exactly-zero W lanes before RCPPS.
            let denominator = _mm_or_ps(
                _mm_and_ps(zero_mask, _mm_set1_ps(1.0)),
                _mm_andnot_ps(zero_mask, w),
            );
            let reciprocal = _mm_rcp_ps(denominator);
            let x = _mm_mul_ps(
                _mm_setr_ps(
                    corners[base][0],
                    corners[base + 1][0],
                    corners[base + 2][0],
                    corners[base + 3][0],
                ),
                reciprocal,
            );
            let y = _mm_mul_ps(
                _mm_setr_ps(
                    corners[base][1],
                    corners[base + 1][1],
                    corners[base + 2][1],
                    corners[base + 3][1],
                ),
                reciprocal,
            );
            _mm_storeu_ps(x_lanes.as_mut_ptr(), x);
            _mm_storeu_ps(y_lanes.as_mut_ptr(), y);
        }
        for lane in 0..4 {
            ndc[base + lane] = [x_lanes[lane], y_lanes[lane]];
        }
    }

    let mut bounds = [ndc[0][0], ndc[0][1], ndc[0][0], ndc[0][1]];
    for point in &ndc[1..] {
        bounds[0] = bounds[0].min(point[0]);
        bounds[1] = bounds[1].min(point[1]);
        bounds[2] = bounds[2].max(point[0]);
        bounds[3] = bounds[3].max(point[1]);
    }
    if bounds.iter().any(|value| !value.is_finite()) {
        return Err("nonfinite projected Tome portal bounds".into());
    }
    Ok(bounds)
}

/// The edge-derived branch of FUN_140a85bd0 clips the projected box against
/// the four homogeneous viewport side planes (x+w, w-x, y+w, w-y) before
/// reducing its bounds. `corners` must use the two face cycles emitted by the
/// original axis-vector construction: 0..4 are the query-facing face and
/// 4..8 its parallel opposite face. This is selected only when active plane
/// zero is first and at least one projected W lane is negative.
pub fn projected_portal_box_crossing_bounds(
    corners: [[f32; 4]; 8],
) -> Result<Option<[f32; 4]>, String> {
    if corners.iter().flatten().any(|value| !value.is_finite()) {
        return Err("nonfinite homogeneous Tome portal box corner".into());
    }

    const FACES: [[usize; 4]; 6] = [
        [0, 1, 2, 3],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ];
    let distances = |point: [f32; 4]| {
        [
            point[0] + point[3],
            point[3] - point[0],
            point[1] + point[3],
            point[3] - point[1],
        ]
    };
    let mut projected = Vec::with_capacity(48);
    for face in FACES {
        let mut polygon = face.map(|index| corners[index]).to_vec();
        for plane in 0..4 {
            if polygon.is_empty() {
                break;
            }
            let mut clipped = Vec::with_capacity(polygon.len() + 1);
            let mut previous = *polygon.last().unwrap();
            let mut previous_distance = distances(previous)[plane];
            for current in polygon {
                let current_distance = distances(current)[plane];
                let previous_inside = previous_distance >= 0.0;
                let current_inside = current_distance >= 0.0;
                if previous_inside != current_inside {
                    let denominator = previous_distance - current_distance;
                    if denominator != 0.0 {
                        let t = previous_distance / denominator;
                        clipped.push(std::array::from_fn(|lane| {
                            previous[lane] + (current[lane] - previous[lane]) * t
                        }));
                    }
                }
                if current_inside {
                    clipped.push(current);
                }
                previous = current;
                previous_distance = current_distance;
            }
            polygon = clipped;
        }
        projected.extend(polygon);
    }

    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for point in projected {
        if point[3] <= 0.0 {
            continue;
        }
        let x = point[0] / point[3];
        let y = point[1] / point[3];
        bounds[0] = bounds[0].min(x);
        bounds[1] = bounds[1].min(y);
        bounds[2] = bounds[2].max(x);
        bounds[3] = bounds[3].max(y);
    }
    if bounds.iter().all(|value| value.is_finite()) {
        Ok(Some(bounds))
    } else {
        Ok(None)
    }
}

/// Select the two FUN_140a85bd0 projected-bound paths from the record's
/// retained active-plane indices and the projected box W lanes.
pub fn projected_portal_box_bounds(
    corners: [[f32; 4]; 8],
    active_planes: &[usize],
) -> Result<Option<([f32; 4], bool)>, String> {
    let crossing_path =
        active_planes.first() == Some(&0) && corners.iter().any(|corner| corner[3] < 0.0);
    if crossing_path {
        Ok(projected_portal_box_crossing_bounds(corners)?.map(|bounds| (bounds, true)))
    } else {
        projected_portal_box_fallback_bounds(corners).map(|bounds| Some((bounds, false)))
    }
}

/// Compose FUN_140a85bd0's selected portal-bound branch with its NDC-to-pixel
/// conversion and viewport clamp. The boolean reports whether the active
/// plane-zero crossing branch supplied the bounds.
pub fn projected_portal_box_pixel_rect(
    corners: [[f32; 4]; 8],
    active_planes: &[usize],
    viewport: [i32; 4],
) -> Result<(Option<[i32; 4]>, bool), String> {
    if viewport[0] > viewport[2] || viewport[1] > viewport[3] {
        return Err("invalid projected Tome portal viewport".into());
    }
    let Some((bounds, used_crossing_path)) = projected_portal_box_bounds(corners, active_planes)?
    else {
        return Ok((None, active_planes.first() == Some(&0)));
    };
    Ok((
        projected_portal_pixel_rect(bounds, viewport)?,
        used_crossing_path,
    ))
}

#[cfg(not(target_arch = "x86_64"))]
pub fn projected_portal_box_fallback_bounds(_: [[f32; 4]; 8]) -> Result<[f32; 4], String> {
    Err("original Tome portal projection requires x86_64 SSE".into())
}

pub fn portal_tile_rect(pixels: [i32; 4]) -> Result<[u8; 4], String> {
    let [left, top, right, bottom] = pixels;
    if left < 0 || top < 0 || right <= left || bottom <= top {
        return Err("invalid clipped Tome portal rectangle".into());
    }
    let edges = [
        left >> 3,
        top >> 2,
        right.checked_add(7).ok_or("Tome tile edge overflow")? >> 3,
        bottom.checked_add(3).ok_or("Tome tile edge overflow")? >> 2,
    ];
    if edges.iter().any(|&edge| edge > u8::MAX as i32) {
        return Err("Tome portal tile exceeds byte coordinates".into());
    }
    Ok(edges.map(|edge| edge as u8))
}

impl PortalTileMask {
    pub fn new(rect: [u8; 4]) -> Result<Self, String> {
        let width = rect[2]
            .checked_sub(rect[0])
            .ok_or("invalid Tome tile rectangle")?;
        let height = rect[3]
            .checked_sub(rect[1])
            .ok_or("invalid Tome tile rectangle")?;
        if width == 0 || height == 0 {
            return Err("empty Tome tile rectangle".into());
        }
        Ok(Self {
            rect,
            bits: vec![0; width as usize * height as usize],
        })
    }

    /// Build the initial query coverage mask from FUN_140a81500 and
    /// FUN_140a89f90. The game initializes every per-tile coverage word in
    /// the viewport rectangle to all ones before seeding the camera's cell.
    pub fn full(rect: [u8; 4]) -> Result<Self, String> {
        let mut mask = Self::new(rect)?;
        mask.bits.fill(u32::MAX);
        Ok(mask)
    }

    /// The original function expands to the coordinate-wise union and
    /// zero-fills newly added rows and columns around the old row-major mask.
    pub fn grow_to_include(&mut self, rect: [u8; 4]) -> Result<(), String> {
        if rect[2] <= rect[0] || rect[3] <= rect[1] {
            return Err("invalid Tome tile rectangle".into());
        }
        let old = self.rect;
        let expanded = [
            old[0].min(rect[0]),
            old[1].min(rect[1]),
            old[2].max(rect[2]),
            old[3].max(rect[3]),
        ];
        if expanded == old {
            return Ok(());
        }
        let mut grown = Self::new(expanded)?;
        let old_width = (old[2] - old[0]) as usize;
        let grown_width = (expanded[2] - expanded[0]) as usize;
        let x_offset = (old[0] - expanded[0]) as usize;
        let y_offset = (old[1] - expanded[1]) as usize;
        for row in 0..(old[3] - old[1]) as usize {
            let start = (row + y_offset) * grown_width + x_offset;
            grown.bits[start..start + old_width]
                .copy_from_slice(&self.bits[row * old_width..(row + 1) * old_width]);
        }
        *self = grown;
        Ok(())
    }

    pub fn merge_new_coverage(&mut self, incoming: &Self) -> Result<bool, String> {
        let [left, top, right, bottom] = incoming.rect;
        if left < self.rect[0]
            || top < self.rect[1]
            || right > self.rect[2]
            || bottom > self.rect[3]
        {
            return Err("incoming Tome tile mask exceeds destination".into());
        }
        let width = (self.rect[2] - self.rect[0]) as usize;
        let incoming_width = (right - left) as usize;
        let mut changed = 0u32;
        for row in 0..(bottom - top) as usize {
            let destination = (top - self.rect[1]) as usize * width
                + row * width
                + (left - self.rect[0]) as usize;
            let source = row * incoming_width;
            for column in 0..incoming_width {
                let old = self.bits[destination + column];
                let merged = old | incoming.bits[source + column];
                changed |= merged ^ old;
                self.bits[destination + column] = merged;
            }
        }
        Ok(changed != 0)
    }

    /// FUN_140a8b2d0 tests a projected object's pixel rectangle against the
    /// portal coverage mask. These four tables are the original executable's
    /// constants at 0x141e68530, 0x141e68550, 0x141e68570, 0x141e68580.
    pub fn intersects_pixels(&self, pixels: [i32; 4]) -> Result<bool, String> {
        const LEFT: [u32; 8] = [
            0xffff_ffff,
            0xffff_eeee,
            0xffff_cccc,
            0xffff_8888,
            0xffff_0000,
            0xeeee_0000,
            0xcccc_0000,
            0x8888_0000,
        ];
        const RIGHT: [u32; 8] = [
            0xffff_ffff,
            0x0000_1111,
            0x0000_3333,
            0x0000_7777,
            0x0000_ffff,
            0x1111_ffff,
            0x3333_ffff,
            0x7777_ffff,
        ];
        const TOP: [u32; 4] = [0xffff_ffff, 0xfff0_fff0, 0xff00_ff00, 0xf000_f000];
        const BOTTOM: [u32; 4] = [0xffff_ffff, 0x000f_000f, 0x00ff_00ff, 0x0fff_0fff];
        let tile = portal_tile_rect(pixels)?;
        if tile[0] < self.rect[0]
            || tile[1] < self.rect[1]
            || tile[2] > self.rect[2]
            || tile[3] > self.rect[3]
        {
            return Err("Tome object rectangle exceeds portal tile mask".into());
        }
        let width = (self.rect[2] - self.rect[0]) as usize;
        for y in tile[1]..tile[3] {
            let y_mask = (if y == tile[1] {
                TOP[pixels[1] as usize & 3]
            } else {
                u32::MAX
            }) & (if y + 1 == tile[3] {
                BOTTOM[pixels[3] as usize & 3]
            } else {
                u32::MAX
            });
            for x in tile[0]..tile[2] {
                let x_mask = (if x == tile[0] {
                    LEFT[pixels[0] as usize & 7]
                } else {
                    u32::MAX
                }) & (if x + 1 == tile[2] {
                    RIGHT[pixels[2] as usize & 7]
                } else {
                    u32::MAX
                });
                let index = (y - self.rect[1]) as usize * width + (x - self.rect[0]) as usize;
                if self.bits[index] & x_mask & y_mask != 0 {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// FUN_140a8b430 accepts an object only where its projected rectangle
    /// overlaps a set portal bit and that same pixel's query depth reaches
    /// the supplied threshold. The depth grid uses 4x4 tiles; this mask uses
    /// one u32 for each 8x4 tile.
    pub fn intersects_depth_pixels(
        &self,
        depths: &[f32],
        pixels: [i32; 4],
        threshold: f32,
    ) -> Result<bool, String> {
        if depths.len() != 64 * 64 {
            return Err("invalid Tome depth grid length".into());
        }
        let tile = portal_tile_rect(pixels)?;
        if tile[0] < self.rect[0]
            || tile[1] < self.rect[1]
            || tile[2] > self.rect[2]
            || tile[3] > self.rect[3]
            || pixels[2] > 64
            || pixels[3] > 64
        {
            return Err("Tome object rectangle exceeds portal mask or depth grid".into());
        }
        let width = (self.rect[2] - self.rect[0]) as usize;
        for y in pixels[1] as usize..pixels[3] as usize {
            for x in pixels[0] as usize..pixels[2] as usize {
                let mask_index =
                    (y / 4 - self.rect[1] as usize) * width + (x / 8 - self.rect[0] as usize);
                let mask_bit = ((x / 4) & 1) * 16 + (y & 3) * 4 + (x & 3);
                if self.bits[mask_index] & (1 << mask_bit) == 0 {
                    continue;
                }
                let depth_index = ((y / 4) * 16 + x / 4) * 16 + (y & 3) * 4 + (x & 3);
                if threshold <= depths[depth_index] {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// FUN_140a8a080's projected portal raster. `projected` is its four
    /// float4 vectors and `face` its three float4 vectors.
    pub fn rasterize_projected_box(
        &mut self,
        incoming: &Self,
        pixels: [i32; 4],
        projected: [[f32; 4]; 4],
        face: [[f32; 4]; 3],
    ) -> Result<bool, String> {
        let [left, top, right, bottom] = pixels;
        let tile = portal_tile_rect(pixels)?;
        if tile[0] < self.rect[0]
            || tile[1] < self.rect[1]
            || tile[2] > self.rect[2]
            || tile[3] > self.rect[3]
            || tile[0] < incoming.rect[0]
            || tile[1] < incoming.rect[1]
            || tile[2] > incoming.rect[2]
            || tile[3] > incoming.rect[3]
        {
            return Err("projected Tome portal exceeds tile masks".into());
        }
        let x0 = left >> 2;
        let y0 = top >> 2;
        let x1 = right.checked_add(3).ok_or("Tome raster edge overflow")? >> 2;
        let y1 = bottom.checked_add(3).ok_or("Tome raster edge overflow")? >> 2;
        let width = x1 - x0;
        let height = y1 - y0;
        let medium = width * height >= 11 && width != 1 && height != 1;
        let large = medium && width * height > 40;
        if projected
            .iter()
            .flatten()
            .chain(face.iter().flatten())
            .any(|value| !value.is_finite())
        {
            return Err("nonfinite Tome portal raster input".into());
        }
        let mut edge_x = [0.0f32; 4];
        let mut edge_y = [0.0f32; 4];
        let mut edge_origin = [0.0f32; 4];
        for lane in 0..4 {
            let positive = 0.0
                < face[2][lane] * projected[2][lane]
                    + face[1][lane] * projected[1][lane]
                    + face[0][lane] * projected[0][lane];
            let next = (lane + 1) & 3;
            let offset_x = if positive { projected[3][0] } else { 0.0 };
            let offset_y = if positive { projected[3][1] } else { 0.0 };
            let offset_z = if positive { projected[3][3] } else { 0.0 };
            let px = projected[0][lane] + offset_x;
            let py = projected[1][lane] + offset_y;
            let pz = projected[2][lane] + offset_z;
            let qx = projected[0][next] + offset_x;
            let qy = projected[1][next] + offset_y;
            let qz = projected[2][next] + offset_z;
            edge_x[lane] = (qz * py - qy * pz) * (1.0 / 32.0);
            edge_y[lane] = (qx * pz - qz * px) * (1.0 / 32.0);
            let constant = qy * px - qx * py;
            let bias =
                (edge_x[lane].abs() + edge_y[lane].abs()).max(f32::from_bits(0x3400_0000)) * -0.5;
            edge_origin[lane] = constant + edge_x[lane] * -31.5 + edge_y[lane] * -31.5 + bias;
        }
        let macro_x0 = x0 >> 2;
        let macro_y0 = y0 >> 2;
        let macro_width = ((x1 + 3) >> 2) - macro_x0;
        let mut macro_classes = Vec::new();
        if large {
            for macro_y in macro_y0..((y1 + 3) >> 2) {
                for macro_x in macro_x0..((x1 + 3) >> 2) {
                    let mut possible = true;
                    let mut full = true;
                    for lane in 0..4 {
                        let base = edge_origin[lane]
                            + edge_x[lane] * (macro_x * 16) as f32
                            + edge_y[lane] * (macro_y * 16) as f32;
                        let dx = edge_x[lane] * 15.0;
                        let dy = edge_y[lane] * 15.0;
                        let min = base + dx.min(0.0) + dy.min(0.0);
                        let max = base + dx.max(0.0) + dy.max(0.0);
                        possible &= min.to_bits() & 0x8000_0000 != 0;
                        full &= max.to_bits() & 0x8000_0000 != 0;
                    }
                    macro_classes.push(if !possible {
                        0
                    } else if full {
                        2
                    } else {
                        1
                    });
                }
            }
        }
        let destination_width = (self.rect[2] - self.rect[0]) as usize;
        let source_width = (incoming.rect[2] - incoming.rect[0]) as usize;
        let mut changed = 0u32;
        for half_y in y0..y1 {
            // The original large-block copy operates on whole u32 mask words
            // (two 4x4 cells), even when the rectangle ends in one half.
            let iteration_x0 = if large { x0 & !1 } else { x0 };
            let iteration_x1 = if large { (x1 + 1) & !1 } else { x1 };
            for half_x in iteration_x0..iteration_x1 {
                let mut coverage = None;
                if large {
                    let class = macro_classes[(((half_y >> 2) - macro_y0) * macro_width
                        + ((half_x >> 2) - macro_x0))
                        as usize];
                    if class == 0 || (half_x < x0 || half_x >= x1) && class != 2 {
                        coverage = Some(0);
                    } else if class == 2 {
                        coverage = Some(u16::MAX);
                    }
                }
                if medium && coverage.is_none() {
                    let mut possible = true;
                    let mut full = true;
                    for lane in 0..4 {
                        let base = edge_origin[lane]
                            + edge_x[lane] * (half_x * 4) as f32
                            + edge_y[lane] * (half_y * 4) as f32;
                        let dx = edge_x[lane] * 3.0;
                        let dy = edge_y[lane] * 3.0;
                        let min = base + dx.min(0.0) + dy.min(0.0);
                        let max = base + dx.max(0.0) + dy.max(0.0);
                        possible &= min.to_bits() & 0x8000_0000 != 0;
                        full &= max.to_bits() & 0x8000_0000 != 0;
                    }
                    if !possible {
                        coverage = Some(0);
                    } else if full {
                        coverage = Some(u16::MAX);
                    }
                }
                let coverage = coverage.unwrap_or_else(|| {
                    let mut coverage = 0u16;
                    for py in 0..4 {
                        for px in 0..4 {
                            let mut inside = true;
                            for lane in 0..4 {
                                let value = edge_origin[lane]
                                    + edge_x[lane] * (half_x * 4 + px) as f32
                                    + edge_y[lane] * (half_y * 4 + py) as f32;
                                inside &= value.to_bits() & 0x8000_0000 != 0;
                            }
                            if inside {
                                coverage |= 1 << (py * 4 + px);
                            }
                        }
                    }
                    coverage
                });
                let tx = (half_x >> 1) as u8;
                let ty = half_y as u8;
                let shift = ((half_x & 1) * 16) as u32;
                let destination =
                    (ty - self.rect[1]) as usize * destination_width + (tx - self.rect[0]) as usize;
                let source = (ty - incoming.rect[1]) as usize * source_width
                    + (tx - incoming.rect[0]) as usize;
                let old = self.bits[destination];
                let merged = old | ((coverage as u32) << shift & incoming.bits[source]);
                changed |= merged ^ old;
                self.bits[destination] = merged;
            }
        }
        Ok(changed != 0)
    }
}

/// FUN_140a784c0 tests a projected pixel rectangle against the original
/// 64x64 query-depth array at context +0x5a0. Each 4x4 tile stores 16 floats
/// in row-major pixel order; tiles form a 16x16 grid. The caller supplies
/// the projected depth threshold.
pub fn depth_grid_intersects_pixels(
    depths: &[f32],
    lower: [i32; 2],
    upper: [i32; 2],
    threshold: f32,
) -> Result<bool, String> {
    if depths.len() != 64 * 64 {
        return Err("invalid Tome depth grid length".into());
    }
    let x0 = lower[0].max(0);
    let y0 = lower[1].max(0);
    let x1 = upper[0].min(64);
    let y1 = upper[1].min(64);
    for y in y0..y1 {
        for x in x0..x1 {
            let tile = (y as usize >> 2) * 16 + (x as usize >> 2);
            let pixel = ((y as usize & 3) << 2) + (x as usize & 3);
            if threshold <= depths[tile * 16 + pixel] {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// FUN_140a8b660 expands an original 8x4-pixel mask rectangle into the
/// query's tiled 64x64 float grid. The installed executable confirms one
/// little-endian u32 mask per tile, with bit N mapping to depth slot N.
/// The context also tracks the greatest depth value passed to this writer.
pub fn rasterize_depth_mask(
    depths: &mut [f32],
    rect: [u8; 4],
    bits: &[u32],
    value: f32,
    previous_max: f32,
) -> Result<f32, String> {
    if depths.len() != 64 * 64 {
        return Err("invalid Tome depth grid length".into());
    }
    let [left, top, right, bottom] = rect.map(usize::from);
    if left >= right || top >= bottom || right > 8 || bottom > 16 {
        return Err("invalid Tome depth mask rectangle".into());
    }
    let width = right - left;
    if bits.len() != width * (bottom - top) {
        return Err("invalid Tome depth mask word count".into());
    }
    if !value.is_finite() || !previous_max.is_finite() {
        return Err("nonfinite Tome depth mask value".into());
    }
    for (tile, &word) in bits.iter().enumerate() {
        let tile_x = left + tile % width;
        let tile_y = top + tile / width;
        let start = tile_y * 256 + tile_x * 32;
        let mut remaining = word;
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            depths[start + bit] = depths[start + bit].max(value);
            remaining &= remaining - 1;
        }
    }
    Ok(previous_max.max(value))
}

/// FUN_140a85bd0 expands a packed portal rectangle into world coordinates
/// using the record's bounds and +0x30 padding. Direct Tome initialization
/// supplies zero additional offset. This is the geometry supplied to later
/// clipping, not a visibility decision by itself.
pub fn packed_link_query_bounds(
    bytes: &[u8],
    record_offset: u32,
    geometry: PackedLinkGeometry,
) -> Result<[[f32; 3]; 2], String> {
    let axis = geometry.axis as usize;
    if axis >= 3 {
        return Err("unsupported fourth-axis Tome portal".into());
    }
    let base = record_offset as usize;
    let record = recorded_bounds(bytes, base)?;
    let padding = float(bytes, base + 0x30)?;
    if !padding.is_finite() {
        return Err("invalid Tome portal padding".into());
    }
    let mut quantized = [[0u16; 3]; 2];
    for side in &mut quantized {
        side[axis] = geometry.plane;
    }
    for span in 0..2 {
        let other = (axis + span + 1) % 3;
        quantized[0][other] = geometry.spans[span][0];
        quantized[1][other] = geometry.spans[span][1];
    }
    let mut bounds = [[0.0f32; 3]; 2];
    for side in 0..2 {
        for dimension in 0..3 {
            let scale = (record[1][dimension] - record[0][dimension]) * (1.0 / 65535.0);
            let coordinate = quantized[side][dimension] as f32 * scale + record[0][dimension];
            bounds[side][dimension] = if side == 0 {
                coordinate - padding
            } else {
                coordinate + padding
            };
        }
    }
    if (0..3).any(|dimension| {
        !bounds[0][dimension].is_finite()
            || !bounds[1][dimension].is_finite()
            || bounds[0][dimension] > bounds[1][dimension]
    }) {
        return Err("invalid Tome packed-link query bounds".into());
    }
    Ok(bounds)
}

/// Bit-27 links use the header-relative +0x70 table instead of packed
/// rectangle coordinates. FUN_140a85bd0 reads two consecutive float3s
/// beginning at entry (word3 >> 12), with a 12-byte table stride.
pub fn gated_link_bounds(bytes: &[u8], link: [u32; 4]) -> Result<Option<[[f32; 3]; 2]>, String> {
    if link[0] & (1 << 27) == 0 {
        return Ok(None);
    }
    let index = (link[3] >> 12) as usize;
    let count = word(bytes, 0x74)? as usize;
    if index.checked_add(1).is_none_or(|next| next >= count) {
        return Err("Tome gated-link bounds exceed geometry table".into());
    }
    let at = (word(bytes, 0x70)? as usize)
        .checked_add(
            index
                .checked_mul(12)
                .ok_or("Tome gated-link index overflow")?,
        )
        .ok_or("Tome gated-link offset overflow")?;
    let bounds = [
        [
            float(bytes, at)?,
            float(bytes, at + 4)?,
            float(bytes, at + 8)?,
        ],
        [
            float(bytes, at + 12)?,
            float(bytes, at + 16)?,
            float(bytes, at + 20)?,
        ],
    ];
    if (0..3).any(|axis| {
        !bounds[0][axis].is_finite()
            || !bounds[1][axis].is_finite()
            || bounds[0][axis] > bounds[1][axis]
    }) {
        return Err("invalid Tome gated-link bounds".into());
    }
    Ok(Some(bounds))
}

/// FUN_140a85bd0 applies the current spatial record's +0x30 padding to
/// the gated link's float-table bounds before later query clipping.
pub fn gated_link_query_bounds(
    bytes: &[u8],
    record_offset: u32,
    link: [u32; 4],
) -> Result<Option<[[f32; 3]; 2]>, String> {
    let Some(raw) = gated_link_bounds(bytes, link)? else {
        return Ok(None);
    };
    let padding = float(bytes, record_offset as usize + 0x30)?;
    if !padding.is_finite() {
        return Err("invalid Tome gated-link padding".into());
    }
    let expanded = [
        std::array::from_fn(|axis| raw[0][axis] - padding),
        std::array::from_fn(|axis| raw[1][axis] + padding),
    ];
    if expanded.iter().flatten().any(|value| !value.is_finite()) {
        return Err("invalid Tome gated-link query bounds".into());
    }
    Ok(Some(expanded))
}

/// The render world allocates ceil(gates / 32) words at Tome load and fills
/// them with 0xff (FUN_14023a5d0, FUN_140a767d0, FUN_140a6f8f0).
pub fn initial_gate_states(bytes: &[u8]) -> Result<Vec<u32>, String> {
    let count = word(bytes, 0x68)? as usize;
    Ok(vec![
        u32::MAX;
        count
            .checked_add(31)
            .ok_or("Tome gate count overflow")?
            / 32
    ])
}

/// FUN_140a76430 sets or clears one gate bit after the game resolves its ID.
pub fn set_gate_state(
    bytes: &[u8],
    states: &mut [u32],
    gate: u32,
    enabled: bool,
) -> Result<(), String> {
    if gate >= word(bytes, 0x68)? {
        return Err("Tome gate index exceeds table".into());
    }
    let slot = states
        .get_mut((gate / 32) as usize)
        .ok_or("Tome gate-state bitset is too short")?;
    let bit = 1u32 << (gate % 32);
    if enabled {
        *slot |= bit;
    } else {
        *slot &= !bit;
    }
    Ok(())
}

fn recorded_bounds(bytes: &[u8], at: usize) -> Result<[[f32; 3]; 2], String> {
    let end = at.checked_add(24).ok_or("Tome bounds offset overflow")?;
    if bytes.get(at..end).is_none() {
        return Err("truncated Tome spatial bounds".into());
    }
    let mut result = [[0.0; 3]; 2];
    for side in 0..2 {
        for axis in 0..3 {
            result[side][axis] = float(bytes, at + (side * 3 + axis) * 4)?;
        }
    }
    if (0..3).any(|axis| {
        !result[0][axis].is_finite()
            || !result[1][axis].is_finite()
            || result[0][axis] > result[1][axis]
    }) {
        return Err("invalid Tome spatial bounds".into());
    }
    Ok(result)
}

impl SpatialTree {
    /// FUN_140a85bd0 resolves a direct link's low-26-bit spatial index and
    /// +6 halfword destination cell index. Collection-Tome remapping is not
    /// part of this single-Tome path.
    pub fn direct_link_destination(
        &self,
        bytes: &[u8],
        link: [u32; 4],
    ) -> Result<Option<DirectLinkDestination>, String> {
        let Some(spatial_index) = direct_spatial_link_target(link) else {
            return Ok(None);
        };
        let spatial_index = spatial_index as usize;
        let record_offset = *self
            .offsets
            .get(spatial_index)
            .ok_or("Tome direct link target exceeds spatial tree")?;
        if record_offset == 0 {
            return Err("Tome direct link target has no record".into());
        }
        let cell_index = (link[1] >> 16) as usize;
        let cell_count = (word(bytes, record_offset as usize + 0x34)? & 0xffff) as usize;
        if cell_index >= cell_count {
            return Err("Tome direct link destination exceeds target cell table".into());
        }
        Ok(Some(DirectLinkDestination {
            spatial_index,
            record_offset,
            cell_index,
        }))
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if word(bytes, 0)? != 0xd600_0012 {
            return Err("unsupported Tome version".into());
        }
        if word(bytes, 8)? as usize != bytes.len() {
            return Err("Tome size mismatch".into());
        }
        let bit_count = word(bytes, 0x2c)? as usize;
        if bit_count == 0 || bit_count % 32 != 0 {
            return Err("unsupported Tome spatial count".into());
        }
        let count = bit_count / 32;
        let codes_at = word(bytes, 0x30)? as usize;
        let planes_at = word(bytes, 0x3c)? as usize;
        let distance_scales_at = word(bytes, 0x9c)? as usize;
        let offsets_at = word(bytes, 0xa0)? as usize;
        let plane_count = word(bytes, 0x38)? as usize;
        if plane_count < count {
            return Err("Tome implicit split planes are not decoded".into());
        }
        let code_bytes = count
            .checked_mul(2)
            .and_then(|bits| bits.checked_add(7))
            .map(|bits| bits / 8)
            .ok_or("Tome code count overflow")?;
        let codes = bytes
            .get(
                codes_at
                    ..codes_at
                        .checked_add(code_bytes)
                        .ok_or("Tome code range overflow")?,
            )
            .ok_or("truncated Tome spatial codes")?;
        let plane_bytes = count.checked_mul(4).ok_or("Tome plane count overflow")?;
        if bytes
            .get(
                planes_at
                    ..planes_at
                        .checked_add(plane_bytes)
                        .ok_or("Tome plane range overflow")?,
            )
            .is_none()
        {
            return Err("truncated Tome split planes".into());
        }
        if bytes
            .get(
                offsets_at
                    ..offsets_at
                        .checked_add(plane_bytes)
                        .ok_or("Tome offset table overflow")?,
            )
            .is_none()
        {
            return Err("truncated Tome spatial offsets".into());
        }
        let global_distance_scales = if distance_scales_at == 0 {
            None
        } else {
            let scale_bytes = count
                .checked_mul(4)
                .ok_or("Tome global distance scale count overflow")?;
            let scales = bytes
                .get(
                    distance_scales_at
                        ..distance_scales_at
                            .checked_add(scale_bytes)
                            .ok_or("Tome global distance scale range overflow")?,
                )
                .ok_or("truncated Tome global distance scales")?;
            Some(
                (0..count)
                    .map(|index| float(scales, index * 4))
                    .collect::<Result<Vec<_>, _>>()?,
            )
        };
        let mut axes = Vec::with_capacity(count);
        let mut planes = Vec::with_capacity(count);
        let mut offsets = Vec::with_capacity(count);
        for index in 0..count {
            axes.push((codes[index / 4] >> ((index % 4) * 2)) & 3);
            planes.push(float(bytes, planes_at + index * 4)?);
            offsets.push(word(bytes, offsets_at + index * 4)?);
        }
        let mut bounds = vec![[[0.0; 3]; 2]; count];
        let mut visited = vec![false; count];
        let mut children = vec![None; count];
        bounds[0] = recorded_bounds(bytes, 0x14)?;
        visited[0] = true;
        let mut internal_rank = 0usize;
        for index in 0..count {
            if !visited[index] {
                return Err(format!("unreachable Tome spatial node {index}"));
            }
            let node = bounds[index];
            if offsets[index] != 0 && recorded_bounds(bytes, offsets[index] as usize)? != node {
                return Err(format!(
                    "Tome spatial record {index} disagrees with split tree"
                ));
            }
            let axis = axes[index] as usize;
            if axis == 3 {
                continue;
            }
            let split = planes[index];
            if !split.is_finite() || split < node[0][axis] || split > node[1][axis] {
                return Err(format!("invalid Tome split plane at node {index}"));
            }
            internal_rank += 1;
            let high = internal_rank
                .checked_mul(2)
                .ok_or("Tome child index overflow")?;
            if high >= count {
                return Err("Tome spatial child exceeds node table".into());
            }
            let low = high - 1;
            if visited[low] || visited[high] {
                return Err("duplicate Tome spatial child".into());
            }
            bounds[low] = node;
            bounds[low][1][axis] = split;
            bounds[high] = node;
            bounds[high][0][axis] = split;
            visited[low] = true;
            visited[high] = true;
            children[index] = Some([low, high]);
        }
        if internal_rank * 2 + 1 != count {
            return Err("incomplete Tome spatial tree".into());
        }
        Ok(Self {
            bounds,
            children,
            axes,
            planes,
            offsets,
            global_distance_scales,
        })
    }

    pub fn node_count(&self) -> usize {
        self.bounds.len()
    }
    pub fn record_count(&self) -> usize {
        self.offsets.iter().filter(|&&offset| offset != 0).count()
    }
    pub fn bounds(&self, index: usize) -> Option<[[f32; 3]; 2]> {
        self.bounds.get(index).copied()
    }
    pub fn record_offset(&self, index: usize) -> Option<u32> {
        self.offsets.get(index).copied()
    }

    /// Run the standard camera-point first pass of FUN_140a80a50. This
    /// initializes global branch states and seeds each authored camera cell
    /// reached through FUN_140a85af0/FUN_140a84370. The later FUN_140a85bd0
    /// coordinator still has to drain these lists, cross portals, and accept
    /// objects; the seeded cells are not the visibility result.
    #[allow(clippy::too_many_arguments)]
    pub fn seed_ordinary_camera_global_query(
        &self,
        bytes: &[u8],
        data: Option<&DirectTomeQueryData>,
        point: [f32; 3],
        query_planes: &[[f32; 4]],
        camera_imprecision: [f32; 3],
        global_distance_threshold: f32,
        worklists: &mut SpatialCellWorklists,
        pool: &mut CellWorkNodePool,
        coverage: &mut VisitedCellCoverage,
        initial_mask: &PortalTileMask,
    ) -> Result<Option<OrdinaryGlobalCameraQuery>, String> {
        if query_planes.len() != 6
            || query_planes
                .iter()
                .flatten()
                .any(|value| !value.is_finite())
            || camera_imprecision.iter().any(|value| !value.is_finite())
            || !global_distance_threshold.is_finite()
        {
            return Err("invalid ordinary Tome global query planes or imprecision".into());
        }
        if !original_query_accepts_tome_root(self, OriginalSpatialQueryVolume::Point(point))? {
            return Ok(None);
        }
        let mut branch_stack = SpatialQueryBranchStack::from_original_root(self)?;
        write_u32(&mut branch_stack.entries[0].bytes, 0x24, 1);
        let mut branch_states = vec![0u16; ORIGINAL_QUERY_CELL_STATE_COUNT];
        let mut next_state = 2usize;
        let mut seeded_records = Vec::new();
        let mut nodes_visited = 0;
        let mut link_target_mask = vec![0u32; self.node_count().div_ceil(32)];
        while branch_stack.active {
            let entry = *branch_stack.current();
            let index = entry.spatial_node_index() as usize;
            let state_index = entry.query_state() as usize;
            let bounds = *self
                .bounds
                .get(index)
                .ok_or("Tome global branch exceeds spatial tree")?;
            nodes_visited += 1;
            let contains_point =
                (0..3).all(|axis| bounds[0][axis] <= point[axis] && point[axis] <= bounds[1][axis]);
            let record_offset = self.offsets[index];
            let mut found_camera_cell = false;
            if contains_point && record_offset != 0 {
                // FUN_140a85af0 rejects records without both a cell table and
                // a local split tree before clearing the reusable cell states.
                let at = record_offset as usize;
                if word(bytes, at + 0x38)? != 0 && word(bytes, at + 0x1c)? != 0 {
                    let parsed_cells;
                    let cells = if let Some(data) = data {
                        data.cells(index)?
                    } else {
                        parsed_cells = CellTree::parse(bytes, record_offset)?;
                        &parsed_cells
                    };
                    worklists.reset_for_record(cells.cell_count())?;
                    if let Some(selection) = cells.cell_for_point(bytes, point)? {
                        found_camera_cell = true;
                        let destination = TraversalCellId {
                            spatial_index: index,
                            cell_index: selection.cell_index,
                        };
                        if let Some(work_item) = worklists.seed_initial_camera_cell(
                            pool,
                            coverage,
                            destination,
                            state_index,
                            initial_mask,
                        )? {
                            branch_states[state_index] = work_item.entry_id;
                            seeded_records.push(InitialCameraCellSeed {
                                spatial_node_index: index,
                                pending_slot: state_index as u16,
                                selection,
                                work_item,
                            });
                        }
                    }
                }
            }
            let has_record = record_offset != 0;
            let overlaps_query =
                global_node_overlaps_query_planes(bounds, query_planes, camera_imprecision);
            let distance_cutoff = if has_record
                && self.children[index].is_some()
                && !overlaps_query
                && !contains_point
                && global_distance_threshold > 0.0
            {
                self.global_distance_scales
                    .as_ref()
                    .and_then(|scales| scales.get(index))
                    .copied()
                    .is_some_and(|scale| {
                        let distance_squared = (0..3)
                            .map(|axis| {
                                let delta = if point[axis] < bounds[0][axis] {
                                    bounds[0][axis] - point[axis]
                                } else if point[axis] > bounds[1][axis] {
                                    point[axis] - bounds[1][axis]
                                } else {
                                    0.0
                                };
                                delta * delta
                            })
                            .sum::<f32>();
                        distance_squared.sqrt() > scale * global_distance_threshold
                    })
            } else {
                false
            };
            let keep_node = !has_record
                || (contains_point && found_camera_cell)
                || overlaps_query
                || (!distance_cutoff && self.children[index].is_some());
            if !keep_node || self.children[index].is_none() {
                if has_record {
                    link_target_mask[index / 32] |= 1 << (index % 32);
                }
                if branch_stack.top == 0 {
                    branch_stack.active = false;
                } else {
                    branch_stack.top -= 1;
                }
                continue;
            }
            if next_state + 1 >= branch_states.len() {
                return Err("ordinary Tome global query state table is full".into());
            }
            branch_states[state_index] = (next_state as u16) | 0x8000;
            branch_stack.push_split(
                self,
                3,
                self.planes[index].to_bits(),
                true,
                next_state as u32,
                (next_state + 1) as u32,
            )?;
            next_state += 2;
        }
        if branch_states[1] & 0x7fff != 0 {
            branch_stack = SpatialQueryBranchStack::from_original_root(self)?;
            write_u32(&mut branch_stack.entries[0].bytes, 0x24, 1);
        }
        Ok(Some(OrdinaryGlobalCameraQuery {
            seeded_records,
            branch_states,
            branch_stack,
            nodes_visited,
            link_target_mask,
        }))
    }

    /// Continue a selected FUN_140a85510 spatial candidate through the
    /// ordinary `-1` record path: FUN_140a85af0 resolves its local CellTree,
    /// FUN_140a84370 finds the camera cell, and FUN_140a82160 queues it at
    /// the candidate's returned state/list slot. The caller still owns the
    /// global query and must pass only candidates it actually selected.
    #[allow(clippy::too_many_arguments)]
    pub fn seed_initial_camera_cell_for_candidate(
        &self,
        bytes: &[u8],
        point: [f32; 3],
        candidate: SpatialQueryBranchCandidate,
        worklists: &mut SpatialCellWorklists,
        pool: &mut CellWorkNodePool,
        coverage: &mut VisitedCellCoverage,
        initial_mask: &PortalTileMask,
    ) -> Result<Option<InitialCameraCellSeed>, String> {
        let record_offset = self
            .record_offset(candidate.spatial_node_index)
            .filter(|&offset| offset != 0)
            .ok_or("selected Tome spatial node has no local record")?;
        let cells = CellTree::parse(bytes, record_offset)?;
        worklists.reset_for_record(cells.cell_count())?;
        let Some((selection, work_item)) = cells.seed_camera_point_cell(
            bytes,
            point,
            candidate.spatial_node_index,
            candidate.cell_state_index as usize,
            worklists,
            pool,
            coverage,
            initial_mask,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(InitialCameraCellSeed {
            spatial_node_index: candidate.spatial_node_index,
            pending_slot: candidate.cell_state_index,
            selection,
            work_item,
        }))
    }

    pub fn is_leaf(&self, index: usize) -> Option<bool> {
        self.children.get(index).map(Option::is_none)
    }

    /// Global-tree prefilter for the query's AABB mode. FUN_140a80a50 uses
    /// FUN_1411f8930's inclusive box overlap before processing a leaf's
    /// spatial record. The flags-zero world-render call starts with a
    /// camera-point volume (type 2); this is not its initial selection.
    pub fn record_leaves_intersecting_aabb(&self, query: [[f32; 3]; 2]) -> Vec<usize> {
        if (0..3).any(|axis| {
            !query[0][axis].is_finite()
                || !query[1][axis].is_finite()
                || query[0][axis] > query[1][axis]
        }) {
            return Vec::new();
        }
        let mut result = Vec::new();
        let mut pending = vec![0usize];
        while let Some(index) = pending.pop() {
            let node = self.bounds[index];
            if (0..3).any(|axis| node[0][axis] > query[1][axis] || query[0][axis] > node[1][axis]) {
                continue;
            }
            if let Some([low, high]) = self.children[index] {
                pending.push(high);
                pending.push(low);
            } else if self.offsets[index] != 0 {
                result.push(index);
            }
        }
        result
    }

    /// Geometric point-containment prefilter for FUN_140a80a50's type-2
    /// volume. A point exactly on a split can touch both neighboring leaves.
    /// This deliberately does not model that function's frustum-plane tests,
    /// radius expansion, branch state, or final query result.
    pub fn record_leaves_containing_point(&self, point: [f32; 3]) -> Vec<usize> {
        if point.iter().any(|coordinate| !coordinate.is_finite()) {
            return Vec::new();
        }
        self.record_leaves_intersecting_aabb([point, point])
    }

    /// Geometric record-node prefilter for FUN_140a80a50's standard type-2
    /// query. The original walk can inspect a record attached to an internal
    /// node as well as one attached to a leaf, so retain every intersecting
    /// node with a record offset while continuing through its children.
    /// This omits the game's node/gate state and bitset output; it is not the
    /// final selected-record set or object visibility.
    pub fn record_nodes_for_camera_query_prefilter(
        &self,
        point: [f32; 3],
        planes: &[[f32; 4]],
        camera_imprecision: [f32; 3],
    ) -> Result<Vec<usize>, String> {
        if point
            .iter()
            .chain(camera_imprecision.iter())
            .any(|v| !v.is_finite())
            || planes.len() != 6
            || planes.iter().flatten().any(|v| !v.is_finite())
        {
            return Err("invalid original Tome camera-query prefilter input".into());
        }

        let mut result = Vec::new();
        let mut pending = vec![0usize];
        while let Some(index) = pending.pop() {
            let bounds = self.bounds[index];
            let contains_point =
                (0..3).all(|axis| bounds[0][axis] <= point[axis] && point[axis] <= bounds[1][axis]);
            let overlaps_frustum = !contains_point
                && global_node_overlaps_query_planes(bounds, planes, camera_imprecision);
            if !contains_point && !overlaps_frustum {
                continue;
            }
            if self.offsets[index] != 0 {
                result.push(index);
            }
            if let Some([low, high]) = self.children[index] {
                pending.push(high);
                pending.push(low);
            }
        }
        Ok(result)
    }

    /// Apply the original query-volume root gate, then the geometric node
    /// tests for the ordinary world query's initial volume. The flags-zero
    /// renderer normally supplies an inverted sentinel, so this dispatches to
    /// the camera-point path. This does not reproduce the query-state gates,
    /// branch bitsets, or visible objects.
    pub fn record_nodes_for_ordinary_query_prefilter(
        &self,
        bounds: [f32; 6],
        camera_position: [f32; 3],
        planes: &[[f32; 4]],
        camera_imprecision: [f32; 3],
    ) -> Result<Vec<usize>, String> {
        match ordinary_spatial_query_volume(bounds, camera_position)? {
            OrdinarySpatialQueryVolume::CameraPoint(point) => {
                let volume = OriginalSpatialQueryVolume::Point(point);
                if !original_query_accepts_tome_root(self, volume)? {
                    return Ok(Vec::new());
                }
                self.record_nodes_for_camera_query_prefilter(point, planes, camera_imprecision)
            }
            OrdinarySpatialQueryVolume::Bounds(query) => {
                if planes.len() != 6
                    || planes.iter().flatten().any(|value| !value.is_finite())
                    || camera_imprecision.iter().any(|value| !value.is_finite())
                {
                    return Err("invalid ordinary Tome query prefilter input".into());
                }
                if !original_query_accepts_tome_root(
                    self,
                    OriginalSpatialQueryVolume::Bounds(query),
                )? {
                    return Ok(Vec::new());
                }
                let mut result = Vec::new();
                let mut pending = vec![0usize];
                while let Some(index) = pending.pop() {
                    let node = self.bounds[index];
                    if (0..3).any(|axis| {
                        node[0][axis] > query[1][axis] || query[0][axis] > node[1][axis]
                    }) {
                        continue;
                    }
                    if self.offsets[index] != 0 {
                        result.push(index);
                    }
                    if let Some([low, high]) = self.children[index] {
                        pending.push(high);
                        pending.push(low);
                    }
                }
                Ok(result)
            }
        }
    }

    /// FUN_140a80a50's optional type-1 four-corner volume test: visit
    /// global-tree nodes whose authored bounds overlap the query volume.
    pub fn record_leaves_intersecting_quad(&self, quad: [[f32; 3]; 4]) -> Vec<usize> {
        let mut result = Vec::new();
        let mut pending = vec![0usize];
        while let Some(index) = pending.pop() {
            if !quad_intersects_aabb(self.bounds[index], quad) {
                continue;
            }
            if let Some([low, high]) = self.children[index] {
                pending.push(high);
                pending.push(low);
            } else if self.offsets[index] != 0 {
                result.push(index);
            }
        }
        result
    }

    /// Compose source-derived tile, crop, plane, corner, and optional type-1
    /// global-tree stages for a geometric diagnostic. This is not the
    /// flags-zero render query's initial camera-point selection.
    pub fn record_leaves_for_quad_diagnostic(
        &self,
        base: [[f32; 4]; 4],
        job: u32,
        count: u32,
    ) -> Result<Vec<usize>, String> {
        Ok(self.record_leaves_intersecting_quad(render_query_job_quad(base, job, count)?))
    }

    /// Selects the original global tree leaf for a point inside Tome bounds.
    pub fn leaf_for_point(&self, point: [f32; 3]) -> Option<usize> {
        if point.iter().any(|value| !value.is_finite())
            || (0..3).any(|axis| {
                point[axis] < self.bounds[0][0][axis] || point[axis] > self.bounds[0][1][axis]
            })
        {
            return None;
        }
        let mut index = 0;
        while let Some([low, high]) = self.children[index] {
            let axis = self.axes[index] as usize;
            index = if point[axis] >= self.planes[index] {
                high
            } else {
                low
            };
        }
        Some(index)
    }
}

/// Original render-query tile and first-depth four-corner volume.
pub fn render_query_job_quad(
    base: [[f32; 4]; 4],
    job: u32,
    count: u32,
) -> Result<[[f32; 3]; 4], String> {
    let tile = render_query_job_tile(job, count)?;
    let cropped = crop_query_rows(base, tile)?;
    first_depth_quad(query_planes_from_rows(base, cropped))
}

impl CellTree {
    pub fn parse(bytes: &[u8], record_offset: u32) -> Result<Self, String> {
        let base = record_offset as usize;
        let root = recorded_bounds(bytes, base)?;
        let packed_count = word(bytes, base + 0x18)?;
        let count = (packed_count >> 5) as usize;
        let code_width = (packed_count & 0x1f) as usize;
        if (count == 0 && packed_count != 0) || (count != 0 && (code_width == 0 || code_width > 31))
        {
            return Err("unsupported Tome cell-code width".into());
        }
        let codes_at = base
            .checked_add(word(bytes, base + 0x1c)? as usize)
            .ok_or("Tome cell codes overflow")?;
        let cell_codes_at = base
            .checked_add(word(bytes, base + 0x20)? as usize)
            .ok_or("Tome cell references overflow")?;
        let plane_count = word(bytes, base + 0x24)? as usize;
        let planes_at = base
            .checked_add(word(bytes, base + 0x28)? as usize)
            .ok_or("Tome cell planes overflow")?;
        let cell_descriptor = word(bytes, base + 0x34)?;
        let cell_count = (cell_descriptor & 0xffff) as usize;
        let direct_cell_ids_available = cell_descriptor >> 16 != 0;
        let cell_table = base
            .checked_add(word(bytes, base + 0x38)? as usize)
            .ok_or("Tome cell table overflow")?;
        let links_at = base
            .checked_add(word(bytes, base + 0x3c)? as usize)
            .ok_or("Tome cell traversal table overflow")?;
        // FUN_140a82710 reads the packed candidate stream from the selected
        // Tome's header, while its start/count live in the spatial cell entry.
        let candidate_format = word(bytes, 0x54)?;
        let candidate_index_bits = (candidate_format & 0x1f) as usize;
        let candidate_run_bits = ((candidate_format >> 5) & 0x1f) as usize;
        let candidate_record_count = word(bytes, 0x5c)? as usize;
        let candidates_at = word(bytes, 0x58)? as usize;
        let object_count = word(bytes, 0x40)? as usize;
        let candidate_width = candidate_index_bits + candidate_run_bits;
        if candidate_width == 0 || candidate_width > 32 {
            return Err("unsupported Tome candidate record width".into());
        }
        let candidate_bytes = candidate_record_count
            .checked_mul(candidate_width)
            .and_then(|bits| bits.checked_add(31))
            .map(|bits| bits / 32)
            .and_then(|words| words.checked_mul(4))
            .ok_or("Tome candidate stream size overflow")?;
        if bytes
            .get(
                candidates_at
                    ..candidates_at
                        .checked_add(candidate_bytes)
                        .ok_or("Tome candidate stream offset overflow")?,
            )
            .is_none()
        {
            return Err("truncated Tome candidate stream".into());
        }
        let portal_node_offset = word(bytes, base + 0x40)? as usize;
        let portal_plane_offset = word(bytes, base + 0x48)? as usize;
        let portal_nodes_at = if portal_node_offset != 0 {
            Some(
                base.checked_add(portal_node_offset)
                    .ok_or("Tome portal node offset overflow")?,
            )
        } else {
            None
        };
        let portal_planes_at = if portal_plane_offset != 0 {
            Some(
                base.checked_add(portal_plane_offset)
                    .ok_or("Tome portal plane offset overflow")?,
            )
        } else {
            None
        };
        let code_bytes = count
            .checked_mul(2)
            .and_then(|bits| bits.checked_add(7))
            .map(|bits| bits / 8)
            .ok_or("Tome cell node count overflow")?;
        let codes = bytes
            .get(
                codes_at
                    ..codes_at
                        .checked_add(code_bytes)
                        .ok_or("Tome cell code range overflow")?,
            )
            .ok_or("truncated Tome cell node codes")?;
        let plane_bytes = plane_count
            .checked_mul(4)
            .ok_or("Tome cell plane count overflow")?;
        if bytes
            .get(
                planes_at
                    ..planes_at
                        .checked_add(plane_bytes)
                        .ok_or("Tome cell plane range overflow")?,
            )
            .is_none()
        {
            return Err("truncated Tome cell planes".into());
        }
        let mut axes = Vec::with_capacity(count);
        for index in 0..count {
            axes.push((codes[index / 4] >> ((index % 4) * 2)) & 3);
        }
        let leaf_count = axes.iter().filter(|&&axis| axis == 3).count();
        let cell_code_bytes = leaf_count
            .checked_mul(code_width)
            .and_then(|bits| bits.checked_add(7))
            .map(|bits| bits / 8)
            .ok_or("Tome cell reference count overflow")?;
        let packed_cell_codes = bytes
            .get(
                cell_codes_at
                    ..cell_codes_at
                        .checked_add(cell_code_bytes)
                        .ok_or("Tome cell reference range overflow")?,
            )
            .ok_or("truncated Tome cell references")?;
        let mut cell_codes = Vec::with_capacity(leaf_count);
        for rank in 0..leaf_count {
            let start_bit = rank * code_width;
            let mut code = 0u32;
            for bit in 0..code_width {
                code |= (((packed_cell_codes[(start_bit + bit) / 8] >> ((start_bit + bit) % 8)) & 1)
                    as u32)
                    << bit;
            }
            cell_codes.push(code);
        }
        let mut cell_ids = Vec::with_capacity(cell_count);
        let mut cell_visibility_counts = Vec::with_capacity(cell_count);
        let mut cell_bounds = Vec::with_capacity(cell_count);
        let mut cell_link_ranges = Vec::with_capacity(cell_count);
        let mut cell_candidate_ranges = Vec::with_capacity(cell_count);
        for index in 0..cell_count {
            let at = cell_table
                .checked_add(index.checked_mul(0x24).ok_or("Tome cell index overflow")?)
                .ok_or("Tome cell entry overflow")?;
            let start = word(bytes, at)?;
            let count = word(bytes, at + 4)?;
            let end = (start as usize)
                .checked_add(count as usize)
                .and_then(|entries| entries.checked_mul(16))
                .and_then(|size| links_at.checked_add(size))
                .ok_or("Tome cell traversal range overflow")?;
            if bytes.get(links_at..end).is_none() {
                return Err("Tome cell traversal range exceeds file".into());
            }
            cell_link_ranges.push((start, count));
            let candidate_start = word(bytes, at + 8)?;
            let candidate_count = word(bytes, at + 12)?;
            if candidate_count != 0 && candidate_start as usize >= candidate_record_count {
                return Err("Tome cell candidate start exceeds stream".into());
            }
            cell_candidate_ranges.push((candidate_start, candidate_count));
            cell_ids.push(word(bytes, at + 0x10)?);
            cell_visibility_counts.push(word(bytes, at + 0x14)?);
            // FUN_140a84760 reconstructs each cell's AABB from six packed
            // u16 coordinates. FUN_140a82710 reads the same three words.
            let packed = [
                word(bytes, at + 0x18)?,
                word(bytes, at + 0x1c)?,
                word(bytes, at + 0x20)?,
            ];
            let quantized = [
                [packed[0] >> 16, packed[0] & 0xffff, packed[1] >> 16],
                [packed[1] & 0xffff, packed[2] >> 16, packed[2] & 0xffff],
            ];
            let mut decoded = [[0.0; 3]; 2];
            for side in 0..2 {
                for axis in 0..3 {
                    decoded[side][axis] = root[0][axis]
                        + (root[1][axis] - root[0][axis])
                            * (quantized[side][axis] as f32 * (1.0 / 65535.0));
                }
            }
            if (0..3).any(|axis| decoded[0][axis] > decoded[1][axis]) {
                return Err(format!("inverted Tome cell bounds at {index}"));
            }
            cell_bounds.push(decoded);
        }
        let all_ones = if count == 0 {
            0
        } else {
            (1u32 << code_width) - 1
        };
        let high_bit = if count == 0 {
            0
        } else {
            1u32 << (code_width - 1)
        };
        if cell_codes
            .iter()
            .any(|&code| code != all_ones && code & high_bit == 0 && code as usize >= cell_count)
        {
            return Err("Tome direct cell code exceeds record's cell table".into());
        }
        let mut bounds = vec![[[0.0; 3]; 2]; count];
        let mut visited = vec![false; count];
        let mut children = vec![None; count];
        let mut planes = vec![0.0; count];
        let mut leaf_ranks = vec![None; count];
        if count != 0 {
            bounds[0] = root;
            visited[0] = true;
        }
        let mut internal_rank = 0usize;
        for index in 0..count {
            if !visited[index] {
                return Err(format!("unreachable Tome cell node {index}"));
            }
            let axis = axes[index] as usize;
            if axis == 3 {
                leaf_ranks[index] = Some(index - internal_rank);
                continue;
            }
            let node = bounds[index];
            let split = if index < plane_count {
                float(bytes, planes_at + index * 4)?
            } else {
                (node[0][axis] + node[1][axis]) * 0.5
            };
            if !split.is_finite() || split < node[0][axis] || split > node[1][axis] {
                return Err(format!("invalid Tome cell split plane at node {index}"));
            }
            planes[index] = split;
            internal_rank += 1;
            let high = internal_rank
                .checked_mul(2)
                .ok_or("Tome cell child overflow")?;
            if high >= count {
                return Err("Tome cell child exceeds node table".into());
            }
            let low = high - 1;
            if visited[low] || visited[high] {
                return Err("duplicate Tome cell child".into());
            }
            bounds[low] = node;
            bounds[low][1][axis] = split;
            bounds[high] = node;
            bounds[high][0][axis] = split;
            visited[low] = true;
            visited[high] = true;
            children[index] = Some([low, high]);
        }
        if count != 0 && internal_rank * 2 + 1 != count {
            return Err("incomplete Tome cell tree".into());
        }
        Ok(Self {
            bounds,
            children,
            axes,
            planes,
            leaf_ranks,
            cell_codes,
            cell_ids,
            cell_visibility_counts,
            direct_cell_ids_available,
            cell_bounds,
            cell_link_ranges,
            cell_candidate_ranges,
            links_at,
            candidates_at,
            candidate_index_bits,
            candidate_run_bits,
            candidate_record_count,
            object_count,
            code_width,
            portal_nodes_at,
            portal_planes_at,
        })
    }

    pub fn node_count(&self) -> usize {
        self.bounds.len()
    }
    pub fn cell_count(&self) -> usize {
        self.cell_ids.len()
    }
    pub fn cell_bounds(&self, index: usize) -> Option<[[f32; 3]; 2]> {
        self.cell_bounds.get(index).copied()
    }
    /// FUN_140a7b890: +0x34 high bits must be nonzero before the +0x10
    /// cell-table value is meaningful as a point-cell ID. Otherwise return
    /// UINT_MAX without reading the table (also for an unused index).
    pub fn point_cell_id(&self, index: usize) -> Result<u32, String> {
        if !self.direct_cell_ids_available {
            return Ok(u32::MAX);
        }
        self.cell_ids
            .get(index)
            .copied()
            .ok_or_else(|| "Tome point-cell index exceeds table".into())
    }

    /// FUN_140a82710 [0x140a83f77,0x140a84350): append the cell output
    /// after the object pass, even if that pass accepted no objects. +0x14
    /// zero emits +0x10 directly; otherwise +0x10 indexes packed (first,run)
    /// rows at root+0x60, with widths in root+0x54 bits 10..19. Collection
    /// remapping is wrapping u32 addition before the deduplication test.
    pub fn append_visible_cell_indices(
        &self,
        bytes: &[u8],
        cell_index: usize,
        collection_cell_base: Option<u32>,
        output: Option<&mut VisibleCellIndexOutput>,
    ) -> Result<(), String> {
        let Some(output) = output else {
            return Ok(());
        };
        let mut first = *self
            .cell_ids
            .get(cell_index)
            .ok_or("Tome visible-cell index exceeds table")?;
        let mut remaining = *self
            .cell_visibility_counts
            .get(cell_index)
            .ok_or("Tome visible-cell count is absent")?;
        let base = collection_cell_base.unwrap_or(0);
        if remaining == 0 {
            output.push_unique(first.wrapping_add(base))?;
            return Ok(());
        }
        let format = word(bytes, 0x54)?;
        let index_bits = ((format >> 10) & 31) as usize;
        let run_bits = ((format >> 15) & 31) as usize;
        if run_bits == 0 {
            return Err("zero-width Tome visible-cell run".into());
        }
        let width = index_bits + run_bits;
        let stream = word(bytes, 0x60)? as usize;
        while remaining != 0 {
            let start_bit = (first as usize)
                .checked_mul(width)
                .ok_or("Tome visible-cell bit offset overflow")?;
            let at = stream
                .checked_add(
                    (start_bit / 32)
                        .checked_mul(4)
                        .ok_or("Tome visible-cell word offset overflow")?,
                )
                .ok_or("Tome visible-cell stream offset overflow")?;
            let shift = start_bit % 32;
            let mut packed = 0u128;
            for lane in 0..(shift + width).div_ceil(32) {
                packed |= (word(
                    bytes,
                    at.checked_add(lane * 4)
                        .ok_or("Tome visible-cell read overflow")?,
                )? as u128)
                    << (lane * 32);
            }
            packed >>= shift;
            let start = (packed & ((1u128 << index_bits) - 1)) as u32;
            let run = ((packed >> index_bits) & ((1u128 << run_bits) - 1)) as u32;
            if run == 0 {
                return Err("zero-length Tome visible-cell run".into());
            }
            let emitted = remaining.min(run);
            for offset in 0..emitted {
                output.push_unique(start.wrapping_add(offset).wrapping_add(base))?;
            }
            remaining -= emitted;
            first = first
                .checked_add(1)
                .ok_or("Tome visible-cell row overflow")?;
        }
        Ok(())
    }

    /// Geometric cell AABB prefilter from FUN_140a84760. This does not
    /// perform portal traversal or emit final visible object indices.
    pub fn cells_intersecting_quad(&self, quad: [[f32; 3]; 4]) -> Vec<usize> {
        self.cell_bounds
            .iter()
            .enumerate()
            .filter_map(|(index, &bounds)| quad_intersects_aabb(bounds, quad).then_some(index))
            .collect()
    }
    pub fn traversal_record_count(&self) -> usize {
        self.cell_link_ranges
            .iter()
            .map(|&(_, count)| count as usize)
            .sum()
    }
    pub fn candidate_count(&self) -> usize {
        self.cell_candidate_ranges
            .iter()
            .map(|&(_, count)| count as usize)
            .sum()
    }

    /// Object indices from the Tome header's packed (start, run length)
    /// stream, consumed by FUN_140a82710 before its object visibility tests.
    pub fn object_candidates(&self, bytes: &[u8], cell_index: usize) -> Result<Vec<u32>, String> {
        let (_, count) = *self
            .cell_candidate_ranges
            .get(cell_index)
            .ok_or("Tome cell index exceeds table")?;
        let mut result = Vec::with_capacity(count as usize);
        self.for_each_object_candidate_batch(bytes, cell_index, |batch| {
            result.extend_from_slice(batch);
            Ok(())
        })?;
        debug_assert_eq!(result.len(), count as usize);
        Ok(result)
    }

    /// Visit packed candidates in the same maximum 32-index batches used by
    /// FUN_140a82710. A caller can process each batch without materializing
    /// the full candidate list for a cell.
    pub fn for_each_object_candidate_batch<F>(
        &self,
        bytes: &[u8],
        cell_index: usize,
        mut visit: F,
    ) -> Result<(), String>
    where
        F: FnMut(&[u32]) -> Result<(), String>,
    {
        let (start, count) = *self
            .cell_candidate_ranges
            .get(cell_index)
            .ok_or("Tome cell index exceeds table")?;
        let width = self
            .candidate_index_bits
            .checked_add(self.candidate_run_bits)
            .filter(|&width| (1..=32).contains(&width))
            .ok_or("unsupported Tome candidate record width")?;
        let run_mask = (1u64 << self.candidate_index_bits) - 1;
        let record_mask = (1u64 << width) - 1;
        let mut record = start as usize;
        let mut emitted = 0usize;
        let mut run_next = 0usize;
        let mut run_remaining = 0usize;
        let mut batch = [0u32; ORIGINAL_OBJECT_QUERY_BATCH_SIZE];

        while emitted < count as usize {
            let batch_len = (count as usize - emitted).min(ORIGINAL_OBJECT_QUERY_BATCH_SIZE);
            for candidate in &mut batch[..batch_len] {
                if run_remaining == 0 {
                    if record >= self.candidate_record_count {
                        return Err("Tome cell candidate range exceeds stream".into());
                    }
                    let bit = record
                        .checked_mul(width)
                        .ok_or("Tome candidate bit offset overflow")?;
                    let at = self
                        .candidates_at
                        .checked_add(bit / 8)
                        .ok_or("Tome candidate byte offset overflow")?;
                    let bytes_needed = (bit % 8 + width + 7) / 8;
                    let packed_bytes = bytes
                        .get(at..at + bytes_needed)
                        .ok_or("truncated Tome candidate record")?;
                    let mut packed = 0u64;
                    for (index, &byte) in packed_bytes.iter().enumerate() {
                        packed |= (byte as u64) << (index * 8);
                    }
                    let value = (packed >> (bit % 8)) & record_mask;
                    let first = (value & run_mask) as usize;
                    let run = (value >> self.candidate_index_bits) as usize;
                    if run == 0
                        || first
                            .checked_add(run)
                            .is_none_or(|end| end > self.object_count)
                        || emitted
                            .checked_add(run)
                            .is_none_or(|end| end > count as usize)
                    {
                        return Err("invalid Tome cell candidate run".into());
                    }
                    run_next = first;
                    run_remaining = run;
                    record += 1;
                }

                *candidate = u32::try_from(run_next)
                    .map_err(|_| "Tome object candidate index exceeds 32 bits")?;
                run_next += 1;
                run_remaining -= 1;
                emitted += 1;
            }
            visit(&batch[..batch_len])?;
        }
        Ok(())
    }

    /// Process one cell's packed object candidates through the established
    /// FUN_140a82710 acceptance stages. This composes candidate decoding,
    /// early tested-bit rejection, optional authored near/far distance gate,
    /// authored-bound loading, record-local bounds/plane clipping, projection,
    /// projected-area cutoff, viewport
    /// clipping, and portal/depth acceptance. The outer FUN_140a85bd0
    /// scheduler still owns cell selection and portal-mask propagation.
    pub fn append_visible_object_candidates(
        &self,
        bytes: &[u8],
        cell_index: usize,
        query: &CellObjectVisibilityQuery<'_>,
        output: &mut VisibleObjectIndexOutput,
    ) -> Result<(), String> {
        self.for_each_object_candidate_batch(bytes, cell_index, |batch| {
            for &object_index in batch {
                // FUN_140a82710 checks this bit before either bounds table.
                if output.contains(object_index)? {
                    continue;
                }
                if let Some(distance_bounds) = object_distance_bounds(bytes, object_index)? {
                    if object_distance_rejection_mask(
                        distance_bounds,
                        query.object_distance_point,
                        query.object_distance_scale_squared,
                    ) != 0
                    {
                        // The distance point is constant across cells. The original
                        // marks this rejection in the bitset without appending an
                        // index or incrementing the accepted-output count.
                        output.seen[object_index as usize / 32] |= 1 << (object_index % 32);
                        output.distance_rejected_objects += 1;
                        continue;
                    }
                }
                let bounds = object_bounds(bytes, object_index)?;
                let expanded = expand_object_bounds_for_camera_imprecision(
                    bounds,
                    query.camera_translation_imprecision,
                );
                let input_bounds = [
                    [expanded[0][0], expanded[0][1], expanded[0][2], 1.0],
                    [expanded[1][0], expanded[1][1], expanded[1][2], 1.0],
                ];
                let clipped = clip_box_to_active_query_planes(
                    query.record_query_bounds,
                    input_bounds,
                    query.query_planes,
                    query.active_planes,
                )?;
                if (0..3).any(|axis| clipped[0][axis] > clipped[1][axis]) {
                    continue;
                }
                let projected_bounds = [
                    [clipped[0][0], clipped[0][1], clipped[0][2]],
                    [clipped[1][0], clipped[1][1], clipped[1][2]],
                ];
                let Some(projected) =
                    project_object_bounds_clipped_w(projected_bounds, query.projection_rows)?
                else {
                    continue;
                };
                output.push_if_projected_object_visible(
                    object_index,
                    projected,
                    query.minimum_area_metric,
                    query.viewport_tile_bounds,
                    query.portal,
                    query.depth_grid,
                )?;
            }
            Ok(())
        })
    }

    /// Raw 16-byte records consumed by FUN_140a85bd0 for one cell.
    pub fn traversal_records(
        &self,
        bytes: &[u8],
        cell_index: usize,
    ) -> Result<Vec<[u32; 4]>, String> {
        let mut result = Vec::new();
        self.for_each_traversal_record_while(bytes, cell_index, &mut |record| {
            result.push(record);
            Ok(true)
        })?;
        Ok(result)
    }

    fn for_each_traversal_record_while(
        &self,
        bytes: &[u8],
        cell_index: usize,
        visit: &mut dyn FnMut([u32; 4]) -> Result<bool, String>,
    ) -> Result<(), String> {
        let (start, count) = *self.cell_link_ranges.get(cell_index).ok_or_else(|| {
            format!(
                "Tome cell index {cell_index} exceeds table of {}",
                self.cell_link_ranges.len()
            )
        })?;
        for index in start
            ..start
                .checked_add(count)
                .ok_or("Tome traversal index overflow")?
        {
            let at = self
                .links_at
                .checked_add(
                    (index as usize)
                        .checked_mul(16)
                        .ok_or("Tome traversal offset overflow")?,
                )
                .ok_or("Tome traversal offset overflow")?;
            let keep_going = visit([
                word(bytes, at)?,
                word(bytes, at + 4)?,
                word(bytes, at + 8)?,
                word(bytes, at + 12)?,
            ])?;
            if !keep_going {
                break;
            }
        }
        Ok(())
    }

    /// Run FUN_140a85790's same-spatial chain using this parsed local cell
    /// tree's link records and the original cell-state/work-list conventions.
    pub fn select_same_spatial_worklist_entry(
        &self,
        bytes: &[u8],
        spatial_index: usize,
        direction: LinkQueryDirection,
        cell_states: &mut [u16],
        worklist: &mut CellTraversalWorklist,
    ) -> Result<Option<CellWorkItem>, String> {
        worklist.select_same_spatial_chain(
            spatial_index,
            direction,
            cell_states,
            |cell_index, visit| self.for_each_traversal_record_while(bytes, cell_index, visit),
        )
    }

    /// Find the first target cell selected by FUN_140a85790 from this cell's
    /// traversal records. `cell_states` is the original +0xc80 word table,
    /// indexed by target cell-table index (the link's high 16 bits). This models the
    /// same-spatial-record link scan only; list mutation and later per-cell
    /// visibility work remain in the outer scheduler. The returned target
    /// preserves the cell-table index separately from its queued work-list entry ID.
    pub fn first_ready_same_spatial_cell_target(
        &self,
        bytes: &[u8],
        spatial_index: usize,
        source_cell: usize,
        direction: LinkQueryDirection,
        cell_states: &[u16],
    ) -> Result<Option<ReadyCellTarget>, String> {
        let mut selected = None;
        self.for_each_traversal_record_while(bytes, source_cell, &mut |link| {
            // FUN_140a85790 exits this same-spatial scan on the first
            // boundary-face or different-spatial link; it does not skip it.
            let Some(destination) = direct_spatial_link_target(link) else {
                return Ok(false);
            };
            if destination as usize != spatial_index {
                return Ok(false);
            }
            let target_cell_index = (link[1] >> 16) as usize;
            let state = *cell_states
                .get(target_cell_index)
                .ok_or("Tome traversal cell-state table is too short")?;
            if let Some(target) =
                ready_same_spatial_cell_target(link, spatial_index, direction, state)
            {
                selected = Some(target);
                return Ok(false);
            }
            Ok(true)
        })?;
        Ok(selected)
    }

    pub fn code_counts(&self) -> (usize, usize, usize) {
        if self.code_width == 0 {
            return (0, 0, 0);
        }
        let all_ones = (1u32 << self.code_width) - 1;
        let high_bit = 1u32 << (self.code_width - 1);
        let mut direct = 0;
        let mut portal = 0;
        let mut empty = 0;
        for &code in &self.cell_codes {
            if code == all_ones {
                empty += 1;
            } else if code & high_bit != 0 {
                portal += 1;
            } else {
                direct += 1;
            }
        }
        (direct, portal, empty)
    }

    pub fn portal_leaf_centers(&self) -> impl Iterator<Item = [f32; 3]> + '_ {
        let high_bit = if self.code_width == 0 {
            0
        } else {
            1u32 << (self.code_width - 1)
        };
        let all_ones = (1u32 << self.code_width) - 1;
        self.leaf_ranks
            .iter()
            .enumerate()
            .filter_map(move |(node, rank)| {
                let code = *self.cell_codes.get((*rank)?)?;
                if code == all_ones || code & high_bit == 0 {
                    return None;
                }
                let bounds = self.bounds[node];
                Some(std::array::from_fn(|axis| {
                    (bounds[0][axis] + bounds[1][axis]) * 0.5
                }))
            })
    }

    pub fn cell_for_point(
        &self,
        bytes: &[u8],
        point: [f32; 3],
    ) -> Result<Option<CellSelection>, String> {
        if self.bounds.is_empty() {
            return Ok(None);
        }
        if point.iter().any(|value| !value.is_finite())
            || (0..3).any(|axis| {
                point[axis] < self.bounds[0][0][axis] || point[axis] > self.bounds[0][1][axis]
            })
        {
            return Ok(None);
        }
        let mut index = 0;
        while let Some([low, high]) = self.children[index] {
            index = if point[self.axes[index] as usize] >= self.planes[index] {
                high
            } else {
                low
            };
        }
        let leaf_rank = self.leaf_ranks[index].unwrap();
        let code = self.cell_codes[leaf_rank];
        let all_ones = (1u32 << self.code_width) - 1;
        if code == all_ones {
            return Ok(None);
        }
        let cell_index = if code & (1 << (self.code_width - 1)) != 0 {
            match self.resolve_portal_code(bytes, code & !(1 << (self.code_width - 1)), point)? {
                Some(index) => index,
                None => return Ok(None),
            }
        } else {
            code as usize
        };
        let cell_id = self.point_cell_id(cell_index)?;
        Ok(Some(CellSelection {
            tree_node: index,
            leaf_rank,
            cell_index,
            cell_id,
            link_count: self.cell_link_ranges[cell_index].1,
            bounds: self.bounds[index],
        }))
    }

    /// Compose FUN_140a84370's point-to-cell lookup with its
    /// FUN_140a82160 initial work-node enqueue.
    #[allow(clippy::too_many_arguments)]
    pub fn seed_camera_point_cell(
        &self,
        bytes: &[u8],
        point: [f32; 3],
        spatial_index: usize,
        pending_slot: usize,
        worklists: &mut SpatialCellWorklists,
        pool: &mut CellWorkNodePool,
        coverage: &mut VisitedCellCoverage,
        initial_mask: &PortalTileMask,
    ) -> Result<Option<(CellSelection, CellWorkItem)>, String> {
        let Some(selection) = self.cell_for_point(bytes, point)? else {
            return Ok(None);
        };
        let destination = TraversalCellId {
            spatial_index,
            cell_index: selection.cell_index,
        };
        let Some(item) = worklists.seed_initial_camera_cell(
            pool,
            coverage,
            destination,
            pending_slot,
            initial_mask,
        )?
        else {
            return Ok(None);
        };
        Ok(Some((selection, item)))
    }

    // FUN_140a89350: a high-bit leaf code indexes a compact binary plane tree.
    // Each eight-byte entry holds a plane index, two terminal flags, and two
    // 16-bit child values. A terminal value is a cell index (or 0xffff).
    fn resolve_portal_code(
        &self,
        bytes: &[u8],
        mut index: u32,
        point: [f32; 3],
    ) -> Result<Option<usize>, String> {
        let nodes_at = self
            .portal_nodes_at
            .ok_or("Tome portal node table is absent")?;
        let planes_at = self
            .portal_planes_at
            .ok_or("Tome portal plane table is absent")?;
        for _ in 0..=u16::MAX {
            let at = nodes_at
                .checked_add(
                    (index as usize)
                        .checked_mul(8)
                        .ok_or("Tome portal node index overflow")?,
                )
                .ok_or("Tome portal node offset overflow")?;
            let descriptor = word(bytes, at)?;
            let children = word(bytes, at + 4)?;
            let plane_index = (descriptor & 0x3fff_ffff) as usize;
            let plane_at = planes_at
                .checked_add(
                    plane_index
                        .checked_mul(16)
                        .ok_or("Tome portal plane index overflow")?,
                )
                .ok_or("Tome portal plane offset overflow")?;
            let signed_distance = point[0] * float(bytes, plane_at)?
                + point[1] * float(bytes, plane_at + 4)?
                + point[2] * float(bytes, plane_at + 8)?
                + float(bytes, plane_at + 12)?;
            let (child, terminal) = if signed_distance >= 0.0 {
                (children >> 16, descriptor & 0x8000_0000 != 0)
            } else {
                (children & 0xffff, descriptor & 0x4000_0000 != 0)
            };
            if terminal {
                return Ok((child != 0xffff).then_some(child as usize));
            }
            index = child;
        }
        Err("Tome portal tree cycles".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires local original-game EOTW Tome fixture"]
    fn ordinary_global_pass_seeds_authored_eotw_camera_cell() {
        let bytes = std::fs::read("artifacts/eotw_p_lowchaos.tome")
            .expect("missing local EOTW Tome fixture");
        let tree = SpatialTree::parse(&bytes).unwrap();
        let distance_scales = tree.global_distance_scales.as_ref().unwrap();
        assert_eq!(distance_scales.len(), tree.node_count());
        assert_eq!(distance_scales[0], -1.0);
        assert!(distance_scales[1] > 0.0);
        assert_eq!(
            SpatialCellSlotRows::parse(&bytes)
                .unwrap()
                .spatial_record_count(),
            tree.node_count()
        );
        let point = [-43.0, 138.0, 44.0];
        let mut worklists = SpatialCellWorklists::new_for_original_query();
        let mut pool = CellWorkNodePool::new_for_original_query().unwrap();
        let mut coverage = VisitedCellCoverage::default();
        let mask = PortalTileMask::full([0, 0, 8, 16]).unwrap();
        let planes = [[0.0, 0.0, 0.0, 1.0]; 6];
        let result = tree
            .seed_ordinary_camera_global_query(
                &bytes,
                None,
                point,
                &planes,
                [0.0; 3],
                0.0,
                &mut worklists,
                &mut pool,
                &mut coverage,
                &mask,
            )
            .unwrap()
            .expect("camera point inside the Tome root");
        assert_eq!(result.nodes_visited, tree.node_count());
        assert!(result
            .seeded_records
            .iter()
            .any(|seed| { seed.spatial_node_index == 903 && seed.selection.cell_id == 1346 }));
        assert_eq!(result.branch_states[1] & 0x8000, 0x8000);
        assert!(result.branch_stack.is_active());
        for seed in &result.seeded_records {
            assert_eq!(
                worklists
                    .pending_cell(seed.pending_slot as usize, seed.selection.cell_index as u16),
                Some(seed.work_item)
            );
        }
        let pose = [
            [1.0, 0.0, 0.0, point[0]],
            [0.0, 1.0, 0.0, point[1]],
            [0.0, 0.0, 1.0, point[2]],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let matrix =
            original_render_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0).unwrap();
        let planes = query_planes_from_rows(matrix, matrix);
        let mut worklists = SpatialCellWorklists::new_for_original_query();
        let mut pool = CellWorkNodePool::new_for_original_query().unwrap();
        let mut coverage = VisitedCellCoverage::default();
        let mut frustum_result = tree
            .seed_ordinary_camera_global_query(
                &bytes,
                None,
                point,
                &planes,
                [0.6; 3],
                ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                &mut worklists,
                &mut pool,
                &mut coverage,
                &mask,
            )
            .unwrap()
            .expect("camera point inside the Tome root");
        assert!(frustum_result.nodes_visited < tree.node_count());
        let mut unbounded_worklists = SpatialCellWorklists::new_for_original_query();
        let mut unbounded_pool = CellWorkNodePool::new_for_original_query().unwrap();
        let mut unbounded_coverage = VisitedCellCoverage::default();
        let unbounded_result = tree
            .seed_ordinary_camera_global_query(
                &bytes,
                None,
                point,
                &planes,
                [0.6; 3],
                0.0,
                &mut unbounded_worklists,
                &mut unbounded_pool,
                &mut unbounded_coverage,
                &mask,
            )
            .unwrap()
            .expect("camera point inside the Tome root");
        assert!(frustum_result.nodes_visited < unbounded_result.nodes_visited);
        assert!(frustum_result
            .seeded_records
            .iter()
            .any(|seed| { seed.spatial_node_index == 903 && seed.selection.cell_id == 1346 }));
        let mut active_nodes = vec![u32::MAX; tree.node_count().div_ceil(32)];
        let activation = frustum_result
            .activate_next_spatial_record(&tree, point, &mut active_nodes, &mut worklists)
            .unwrap()
            .expect("first seeded spatial record");
        assert!(activation.activated_cells > 0);
        assert_eq!(
            frustum_result.branch_states[activation.pending_slot as usize],
            0
        );
        assert!(frustum_result.seeded_records.iter().any(|seed| {
            seed.spatial_node_index == activation.spatial_node_index
                && seed.pending_slot == activation.pending_slot
        }));
        assert!(tree
            .seed_ordinary_camera_global_query(
                &bytes,
                None,
                [100_000.0, 0.0, 0.0],
                &planes,
                [0.6; 3],
                ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                &mut worklists,
                &mut pool,
                &mut coverage,
                &mask,
            )
            .unwrap()
            .is_none());
    }

    #[test]
    #[ignore = "requires local original-game EOTW Tome fixture"]
    fn authored_eotw_record_direct_loop_accepts_roof_objects() {
        let bytes = std::fs::read("artifacts/eotw_p_lowchaos.tome")
            .expect("missing local EOTW Tome fixture");
        let tree = SpatialTree::parse(&bytes).unwrap();
        let point = [-43.0, 138.0, 44.0];
        let record = tree.record_offset(903).unwrap();
        let cells = CellTree::parse(&bytes, record).unwrap();
        let cell = cells.cell_for_point(&bytes, point).unwrap().unwrap();
        assert_eq!(cell.cell_id, 1346);
        let viewpoint = [point[0], point[1], 100.0];
        let pose = [
            [1.0, 0.0, 0.0, viewpoint[0]],
            [0.0, 1.0, 0.0, viewpoint[1]],
            [0.0, 0.0, 1.0, viewpoint[2]],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let matrix =
            original_render_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0).unwrap();
        let planes = query_planes_from_rows(matrix, matrix);
        let stored_matrix = stored_query_matrix_from_render_rows(matrix);
        let setup =
            setup_spatial_record_query(&bytes, record, viewpoint, [0.6; 3], &planes).unwrap();
        let portal = PortalTileMask::full([0, 0, 8, 16]).unwrap();
        let query = CellObjectVisibilityQuery {
            record_query_bounds: setup.bounds,
            query_planes: &planes,
            active_planes: &setup.active_planes,
            projection_rows: stored_matrix,
            camera_translation_imprecision: 0.6,
            object_distance_point: viewpoint,
            object_distance_scale_squared: [1.0; 4],
            minimum_area_metric: 25.0 / (1920.0 * 1080.0),
            viewport_tile_bounds: portal.rect,
            portal: &portal,
            depth_grid: None,
        };
        let mut output = VisibleObjectIndexOutput::new(object_ids(&bytes).unwrap().len(), 32768);
        cells
            .append_visible_object_candidates(&bytes, cell.cell_index, &query, &mut output)
            .unwrap();
        assert_eq!(
            cells.object_candidates(&bytes, cell.cell_index).unwrap(),
            [28684, 28685, 28686]
        );
        assert_eq!(output.indices(), &[28686]);
        assert!(output.area_metrics().iter().all(|area| area.is_some()));
        let direction = LinkQueryDirection::Volume {
            center: setup.link_center,
            tolerance: setup.link_tolerance,
        };
        let gates = initial_gate_states(&bytes).unwrap();
        let links = cells.traversal_records(&bytes, cell.cell_index).unwrap();
        assert_eq!(links.len(), 12);
        let link = links[0];
        assert_eq!(direct_spatial_link_target(link), Some(903));
        assert!(
            traversal_link_precheck(&bytes, link, direction, Some(&gates))
                .unwrap()
                .allows_traversal()
        );
        let face =
            admitted_link_portal_face(&bytes, record, link, direction, viewpoint, None).unwrap();
        assert_eq!(face, Some(0));
        let (pixels, path) = projected_link_portal_transition(
            &bytes,
            record,
            link,
            face,
            stored_matrix,
            false,
            &setup.active_planes,
            [0, 0, 64, 64],
            &portal,
        )
        .unwrap()
        .unwrap();
        assert_eq!(pixels, [29, 29, 30, 41]);
        assert!(matches!(path, PortalCoveragePath::Clipped { .. }));
        let mut worklists = SpatialCellWorklists::new_for_original_query();
        let mut pool = CellWorkNodePool::new_for_original_query().unwrap();
        let mut coverage = VisitedCellCoverage::default();
        let changed = traverse_same_spatial_direct_link_from_pool(
            &bytes,
            record,
            link,
            direction,
            Some(&gates),
            viewpoint,
            None,
            stored_matrix,
            false,
            &setup.active_planes,
            [0, 0, 64, 64],
            &portal,
            903,
            &mut worklists,
            &mut pool,
            &mut coverage,
            |_, _, _| Ok(()),
        )
        .unwrap();
        let target_cell = (link[1] >> 16) as usize;
        assert!(!changed);
        assert_eq!(worklists.cell_states()[target_cell], 0);
        assert!(coverage
            .coverage(TraversalCellId {
                spatial_index: 903,
                cell_index: target_cell
            })
            .is_none());

        let rows = SpatialCellSlotRows::parse(&bytes).unwrap();
        assert_eq!(rows.spatial_record_count(), tree.node_count());
        let mut cross_worklists = SpatialCellWorklists::new_for_original_query();
        let mut cross_pool = CellWorkNodePool::new_for_original_query().unwrap();
        let mut cross_coverage = VisitedCellCoverage::default();
        let mut global = tree
            .seed_ordinary_camera_global_query(
                &bytes,
                None,
                point,
                &[[0.0, 0.0, 0.0, 1.0]; 6],
                [0.6; 3],
                ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                &mut cross_worklists,
                &mut cross_pool,
                &mut cross_coverage,
                &portal,
            )
            .unwrap()
            .unwrap();
        let cross_link = links
            .iter()
            .copied()
            .find(|&link| direct_spatial_link_target(link) == Some(725))
            .unwrap();
        let mut mapper = SpatialCellSlotMapper::default();
        let changed = traverse_cross_spatial_direct_link_from_pool(
            &bytes,
            record,
            cross_link,
            direction,
            Some(&gates),
            viewpoint,
            None,
            stored_matrix,
            false,
            &setup.active_planes,
            [0, 0, 64, 64],
            &portal,
            903,
            &rows,
            &mut mapper,
            &mut global.branch_states,
            &mut cross_worklists,
            &mut cross_pool,
            &mut cross_coverage,
            |_, _, _| Ok(()),
        )
        .unwrap();
        assert!(!changed);
        let cross_cell = (cross_link[1] >> 16) as u16;
        let slot = rows
            .resolve(&mut mapper, 725, &global.branch_states)
            .unwrap();
        assert!(cross_worklists
            .pending_cell(slot as usize, cross_cell)
            .is_none());
        assert!(mapper.intermediate_slots().is_empty());
        assert!(cross_coverage
            .coverage(TraversalCellId {
                spatial_index: 725,
                cell_index: cross_cell as usize,
            })
            .is_none());

        let mut loop_worklists = SpatialCellWorklists::new_for_original_query();
        let mut loop_pool = CellWorkNodePool::new_for_original_query().unwrap();
        let mut loop_coverage = VisitedCellCoverage::default();
        let mut loop_global = tree
            .seed_ordinary_camera_global_query(
                &bytes,
                None,
                point,
                &[[0.0, 0.0, 0.0, 1.0]; 6],
                [0.6; 3],
                ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                &mut loop_worklists,
                &mut loop_pool,
                &mut loop_coverage,
                &portal,
            )
            .unwrap()
            .unwrap();
        let camera_seed = loop_global
            .seeded_records
            .iter()
            .find(|seed| seed.spatial_node_index == 903 && seed.selection.cell_id == 1346)
            .unwrap();
        loop_worklists
            .activate_pending_spatial(camera_seed.pending_slot as usize)
            .unwrap();
        loop_global.branch_states[camera_seed.pending_slot as usize] = 0;
        let mut loop_mapper = SpatialCellSlotMapper::default();
        let processed = process_next_active_direct_cell(
            &cells,
            &bytes,
            record,
            903,
            direction,
            &vec![u32::MAX; tree.node_count().div_ceil(32)],
            Some(&gates),
            viewpoint,
            None,
            stored_matrix,
            false,
            &setup.active_planes,
            &rows,
            &mut loop_mapper,
            &mut loop_global.branch_states,
            &mut loop_worklists,
            &mut loop_pool,
            &mut loop_coverage,
            |_, _, _| Ok(()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(processed, camera_seed.work_item);
        assert_eq!(loop_pool.recycled_count(), 1);
        assert!(loop_coverage
            .coverage(TraversalCellId {
                spatial_index: 725,
                cell_index: cross_cell as usize,
            })
            .is_none());
        let mut progressed = 1;
        for _ in 0..500 {
            match process_next_active_direct_cell(
                &cells,
                &bytes,
                record,
                903,
                direction,
                &vec![u32::MAX; tree.node_count().div_ceil(32)],
                Some(&gates),
                viewpoint,
                None,
                stored_matrix,
                false,
                &setup.active_planes,
                &rows,
                &mut loop_mapper,
                &mut loop_global.branch_states,
                &mut loop_worklists,
                &mut loop_pool,
                &mut loop_coverage,
                |_, _, _| Ok(()),
            )
            .unwrap()
            {
                Some(_) => progressed += 1,
                None => break,
            }
        }
        assert_eq!(progressed, 1);
        assert!(loop_worklists.active_worklist().is_empty());
        let mut loop_output =
            VisibleObjectIndexOutput::new(object_ids(&bytes).unwrap().len(), 32768);
        let flushed = loop_pool
            .flush_recycled(
                loop_worklists.cell_states_mut(),
                &mut loop_coverage,
                |item, owner, state| {
                    assert_eq!(owner.spatial_index, 903);
                    let portal = state.coverage(owner).unwrap();
                    let query = CellObjectVisibilityQuery {
                        record_query_bounds: setup.bounds,
                        query_planes: &planes,
                        active_planes: &setup.active_planes,
                        projection_rows: stored_matrix,
                        camera_translation_imprecision: 0.6,
                        object_distance_point: viewpoint,
                        object_distance_scale_squared: [1.0; 4],
                        minimum_area_metric: 25.0 / (1920.0 * 1080.0),
                        viewport_tile_bounds: portal.rect,
                        portal,
                        depth_grid: None,
                    };
                    cells.append_visible_object_candidates(
                        &bytes,
                        item.cell_index as usize,
                        &query,
                        &mut loop_output,
                    )
                },
            )
            .unwrap();
        assert_eq!(flushed, 1);
        assert_eq!(loop_output.indices(), &[28686]);
    }

    #[test]
    #[ignore = "requires local original-game EOTW Tome fixture"]
    fn ordinary_direct_query_returns_authored_eotw_roof_objects() {
        let bytes = std::fs::read("artifacts/eotw_p_lowchaos.tome")
            .expect("missing local EOTW Tome fixture");
        let tree = SpatialTree::parse(&bytes).unwrap();
        let point = [-43.0, 138.0, 48.0];
        let pose = [
            [1.0, 0.0, 0.0, point[0]],
            [0.0, 1.0, 0.0, point[1]],
            [0.0, 0.0, 1.0, point[2]],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let rows = original_render_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0).unwrap();
        let gates = initial_gate_states(&bytes).unwrap();
        let input = OrdinaryDirectQueryInput {
            camera_point: point,
            render_rows: rows,
            cropped_rows: rows,
            initial_tile_rect: [0, 0, 8, 16],
            camera_imprecision: [0.6; 3],
            global_distance_threshold: ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            object_imprecision: 0.6,
            minimum_area_metric: 25.0 / (1920.0 * 1080.0),
            winding_flip: false,
            gate_states: Some(&gates),
            output_capacity: 32768,
        };
        let mut diagnostics = OrdinaryQueryDiagnostics::default();
        let output = ordinary_direct_tome_visible_objects_with_diagnostics(
            &tree,
            &bytes,
            &input,
            &mut diagnostics,
        )
        .unwrap()
        .unwrap();
        assert_eq!(output.indices(), &[28684]);
        assert!(output.area_metrics().iter().all(|area| area.is_some()));
        assert_eq!(diagnostics.stage, "complete");
        assert!(diagnostics.root_accepted);
        assert_eq!(diagnostics.accepted_objects, 1);
        assert!(diagnostics.distance_rejected_objects >= 2);
        assert!(diagnostics.cells_processed > 0);

        // Force a late object-pass error, rather than a setup failure, so
        // the last cell and traversal progress must survive the discarded output.
        let mut corrupt = bytes.clone();
        let bounds_at = word(&bytes, 0x44).unwrap() as usize + 28684 * 0x18;
        corrupt[bounds_at..bounds_at + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        let error = ordinary_direct_tome_visible_objects_with_diagnostics(
            &tree,
            &corrupt,
            &input,
            &mut diagnostics,
        )
        .unwrap_err();
        assert!(error.contains("invalid Tome object bounds"));
        assert_eq!(diagnostics.stage, "object_pass");
        assert!(diagnostics.current_spatial_record.is_some());
        assert!(diagnostics.current_object_cell.is_some());
        assert!(diagnostics.object_passes > 0);
        assert_eq!(diagnostics.accepted_objects, 0);
    }

    #[test]
    fn grows_and_merges_original_portal_tile_coverage() {
        assert_eq!(portal_tile_rect([9, 5, 24, 12]).unwrap(), [1, 1, 3, 3]);
        assert!(portal_tile_rect([9, 5, 9, 12]).is_err());
        let mut destination = PortalTileMask::new([2, 3, 4, 5]).unwrap();
        destination.bits = vec![1, 2, 4, 8];
        destination.grow_to_include([1, 4, 5, 6]).unwrap();
        assert_eq!(destination.rect, [1, 3, 5, 6]);
        assert_eq!(destination.bits, [0, 1, 2, 0, 0, 4, 8, 0, 0, 0, 0, 0]);
        let mut incoming = PortalTileMask::new([2, 4, 4, 6]).unwrap();
        incoming.bits = vec![4, 8, 16, 32];
        assert!(destination.merge_new_coverage(&incoming).unwrap());
        assert_eq!(destination.bits, [0, 1, 2, 0, 0, 4, 8, 0, 0, 16, 32, 0]);
        assert!(!destination.merge_new_coverage(&incoming).unwrap());
        assert!(destination
            .merge_new_coverage(&PortalTileMask::new([0, 0, 1, 1]).unwrap())
            .is_err());
    }

    #[test]
    fn initial_camera_cell_mask_covers_the_viewport_tile_rectangle() {
        let mask = PortalTileMask::full([3, 5, 6, 7]).unwrap();
        assert_eq!(mask.bits, vec![u32::MAX; 6]);
        assert!(mask.intersects_pixels([24, 20, 48, 28]).unwrap());
        assert!(PortalTileMask::full([4, 5, 4, 7]).is_err());
    }

    #[test]
    fn portal_face_vectors_follow_original_query_context_cofactors_and_winding() {
        let matrix = [
            [1.0, 2.0, 3.0, 4.0],
            [5.0, 6.0, 7.0, 8.0],
            [9.0, 10.0, 11.0, 12.0],
            [13.0, 14.0, 15.0, 16.0],
        ];
        let normal = portal_face_vectors_from_query_matrix(matrix, false).unwrap();
        let flipped = portal_face_vectors_from_query_matrix(matrix, true).unwrap();

        // The first cofactor triple is [-8, 12, -4], the second [16, -24,
        // 8], and the third [-8, 12, -4]. FUN_140a81500 emits the three
        // cyclic axis pairs in face order and swaps each pair by +0x4b4.
        assert_eq!(
            normal[0],
            [
                [8.0, 16.0, -8.0, -16.0],
                [-12.0, -24.0, 12.0, 24.0],
                [4.0, 8.0, -4.0, -8.0]
            ]
        );
        assert_eq!(
            normal[1],
            [
                [16.0, 8.0, -16.0, -8.0],
                [-24.0, -12.0, 24.0, 12.0],
                [8.0, 4.0, -8.0, -4.0]
            ]
        );
        assert_eq!(
            normal[2],
            [
                [8.0, -8.0, -8.0, 8.0],
                [-12.0, 12.0, 12.0, -12.0],
                [4.0, -4.0, -4.0, 4.0]
            ]
        );
        assert_eq!(
            normal[3],
            [
                [-8.0, 8.0, 8.0, -8.0],
                [12.0, -12.0, -12.0, 12.0],
                [-4.0, 4.0, 4.0, -4.0]
            ]
        );
        assert_eq!(
            normal[4],
            [
                [-16.0, -8.0, 16.0, 8.0],
                [24.0, 12.0, -24.0, -12.0],
                [-8.0, -4.0, 8.0, 4.0]
            ]
        );
        assert_eq!(
            normal[5],
            [
                [-8.0, -16.0, 8.0, 16.0],
                [12.0, 24.0, -12.0, -24.0],
                [-4.0, -8.0, 4.0, 8.0]
            ]
        );
        for pair in 0..3 {
            assert_eq!(flipped[pair * 2], normal[pair * 2 + 1]);
            assert_eq!(flipped[pair * 2 + 1], normal[pair * 2]);
        }
        assert!(portal_face_vectors_from_query_matrix([[f32::NAN; 4]; 4], false).is_err());
    }

    #[test]
    fn portal_corner_transform_uses_original_query_matrix_component_order() {
        let matrix = [
            [1.0, 2.0, 3.0, 4.0],
            [5.0, 6.0, 7.0, 8.0],
            [9.0, 10.0, 11.0, 12.0],
            [13.0, 14.0, 15.0, 16.0],
        ];
        assert_eq!(
            transform_portal_corner_by_query_matrix([2.0, 3.0, 4.0], matrix).unwrap(),
            [66.0, 76.0, 86.0, 96.0]
        );
        assert!(
            transform_portal_corner_by_query_matrix([f32::INFINITY, 0.0, 0.0], matrix).is_err()
        );
    }

    #[test]
    fn caller_builds_raster_corners_in_original_face_cycle() {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let bounds = [[1.0, 2.0, 3.0], [5.0, 7.0, 11.0]];
        let (normal_face, face_vectors) =
            portal_raster_inputs_from_bounds(bounds, 0, identity, false).unwrap();
        assert_eq!(
            normal_face,
            [
                [1.0, 1.0, 1.0, 1.0],
                [2.0, 7.0, 7.0, 2.0],
                [1.0, 1.0, 1.0, 1.0],
                [4.0, 0.0, 0.0, 0.0],
            ]
        );
        assert_eq!(
            face_vectors,
            portal_face_vectors_from_query_matrix(identity, false).unwrap()[0]
        );

        let (reversed_face, _) =
            portal_raster_inputs_from_bounds(bounds, 1, identity, false).unwrap();
        assert_eq!(
            reversed_face,
            [
                [1.0, 1.0, 1.0, 1.0],
                [2.0, 2.0, 7.0, 7.0],
                [1.0, 1.0, 1.0, 1.0],
                [4.0, 0.0, 0.0, 0.0],
            ]
        );
        assert_eq!(normal_face[2], [1.0; 4]);
        assert!(portal_raster_inputs_from_bounds(bounds, 6, identity, false).is_err());
    }

    #[test]
    fn real_street_portal_matches_original_raster() {
        let source = PortalTileMask::full([0, 0, 8, 16]).unwrap();
        let mut target = PortalTileMask::new([0, 0, 8, 16]).unwrap();
        let projected = [
            [2.0440674; 4],
            [-4.0475955, -4.0475955, 7.0755935, 7.0755935],
            [4.0101776, -0.48988342, -0.48988342, 4.0101776],
            [-0.5615573, 0.0, 0.0, 0.0],
        ];
        let face = [
            [0.0, 0.0, -0.0, -0.0],
            [1.1231146, -0.0, -1.1231146, 0.0],
            [0.0, 2.379513, -0.0, -2.379513],
        ];
        let changed = target
            .rasterize_projected_box(&source, [0, 0, 49, 64], projected, face)
            .unwrap();
        assert!(!changed);
        assert!(target.bits.iter().all(|&bits| bits == 0));
    }

    #[test]
    fn local_link_face_selection_preserves_direction_and_no_portal_branch() {
        let link = [1 << 29, 50, (40 << 16) | 60, (40 << 16) | 60];
        let volume = |center| LinkQueryDirection::Volume {
            center,
            tolerance: [2, 0, 0, 0],
        };
        assert_eq!(
            admitted_link_portal_face(&[], 0, link, volume([20, 50, 50, 0]), [0.0; 3], None)
                .unwrap(),
            Some(1)
        );
        assert_eq!(
            admitted_link_portal_face(&[], 0, link, volume([50, 50, 50, 0]), [0.0; 3], None)
                .unwrap(),
            None
        );
        assert_eq!(
            admitted_link_portal_face(&[], 0, link, volume([50, 25, 50, 0]), [0.0; 3], None)
                .unwrap(),
            Some(3)
        );
        assert_eq!(
            admitted_link_portal_face(&[], 0, link, volume([50, 75, 50, 0]), [0.0; 3], None)
                .unwrap(),
            Some(2)
        );
        assert_eq!(
            admitted_link_portal_face(
                &[],
                0,
                link,
                LinkQueryDirection::Point {
                    axis_sides: [false; 4]
                },
                [0.0; 3],
                None
            )
            .unwrap(),
            Some(1)
        );
    }

    #[test]
    fn local_no_portal_transition_reuses_incoming_tile_extent() {
        let incoming = PortalTileMask::full([2, 3, 4, 5]).unwrap();
        let (pixels, path) = projected_link_portal_transition(
            &[],
            0,
            [0; 4],
            None,
            [[0.0; 4]; 4],
            false,
            &[],
            [0, 0, 64, 64],
            &incoming,
        )
        .unwrap()
        .unwrap();
        assert_eq!(pixels, [16, 12, 32, 20]);
        assert_eq!(path, PortalCoveragePath::Unclipped);
    }

    #[test]
    fn direct_local_link_reaches_unvisited_cell_through_original_work_pool() {
        let link = [
            (1 << 29) | 4,
            (1 << 16) | 50,
            (40 << 16) | 60,
            (40 << 16) | 60,
        ];
        let direction = LinkQueryDirection::Volume {
            center: [50, 50, 50, 0],
            tolerance: [0; 4],
        };
        let destination = TraversalCellId {
            spatial_index: 4,
            cell_index: 1,
        };
        let incoming = PortalTileMask::full([0, 0, 1, 1]).unwrap();
        let mut worklists = SpatialCellWorklists::new(2);
        let mut pool = CellWorkNodePool::new(3).unwrap();
        let mut coverage = VisitedCellCoverage::default();
        assert!(traverse_direct_unvisited_link_from_pool(
            &[],
            0,
            link,
            direction,
            None,
            [0.0; 3],
            None,
            [[0.0; 4]; 4],
            false,
            &[],
            [0, 0, 8, 4],
            &incoming,
            4,
            destination,
            0,
            &mut worklists,
            &mut pool,
            &mut coverage,
            |_, _, _| Ok(())
        )
        .unwrap());
        assert_eq!(worklists.cell_states()[1], CellTraversalState::QUEUED | 1);
        assert_eq!(pool.free_count(), 1);
        assert_eq!(coverage.coverage(destination).unwrap(), &incoming);
        assert_eq!(worklists.active_worklist().entries().count(), 1);
    }

    #[test]
    fn new_portal_bits_reactivate_a_processed_cell_from_recycled_chain() {
        let destination = TraversalCellId {
            spatial_index: 4,
            cell_index: 1,
        };
        let mut worklists = SpatialCellWorklists::new(2);
        let mut pool = CellWorkNodePool::new(3).unwrap();
        let mut coverage = VisitedCellCoverage::default();
        let mut first = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        first.bits[0] = 1;
        worklists
            .propagate_and_schedule_unvisited_cell_from_pool(
                &mut pool,
                &mut coverage,
                destination,
                0,
                true,
                [0, 0, 8, 4],
                &first,
                PortalCoveragePath::Unclipped,
                |_, _, _| Ok(()),
            )
            .unwrap();
        let item = worklists.active.entries.pop_front().unwrap();
        pool.release_processed(item, worklists.cell_states_mut())
            .unwrap();
        assert_eq!(pool.recycled_count(), 1);
        assert!(!worklists
            .reactivate_processed_active_cell(
                &mut pool,
                &mut coverage,
                destination,
                item,
                [0, 0, 8, 4],
                &first,
                PortalCoveragePath::Unclipped,
            )
            .unwrap());
        assert_eq!(pool.recycled_count(), 1);
        let mut second = first.clone();
        second.bits[0] = 2;
        let link = [
            (1 << 29) | 4,
            (1 << 16) | 50,
            (40 << 16) | 60,
            (40 << 16) | 60,
        ];
        assert!(traverse_same_spatial_direct_link_from_pool(
            &[],
            0,
            link,
            LinkQueryDirection::Volume {
                center: [50, 50, 50, 0],
                tolerance: [0; 4],
            },
            None,
            [0.0; 3],
            None,
            [[0.0; 4]; 4],
            false,
            &[],
            [0, 0, 8, 4],
            &second,
            4,
            &mut worklists,
            &mut pool,
            &mut coverage,
            |_, _, _| Ok(())
        )
        .unwrap());
        assert_eq!(pool.recycled_count(), 0);
        assert_eq!(
            worklists.cell_states()[1],
            item.entry_id | CellTraversalState::QUEUED
        );
        assert_eq!(
            worklists.active_worklist().entries().collect::<Vec<_>>(),
            [item]
        );
        assert_eq!(coverage.coverage(destination).unwrap().bits, vec![3]);
    }

    #[test]
    fn visited_cell_coverage_requeues_only_for_new_bits_per_spatial_cell() {
        let cell = TraversalCellId {
            spatial_index: 903,
            cell_index: 71,
        };
        let mut visited = VisitedCellCoverage::default();
        let mut first = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        first.bits[0] = 1;
        assert!(visited.merge_incoming(cell, &first).unwrap());
        assert!(!visited.merge_incoming(cell, &first).unwrap());

        let mut next_portal = PortalTileMask::new([1, 0, 2, 1]).unwrap();
        next_portal.bits[0] = 2;
        assert!(visited.merge_incoming(cell, &next_portal).unwrap());
        let coverage = visited.coverage(cell).unwrap();
        assert_eq!(coverage.rect, [0, 0, 2, 1]);
        assert_eq!(coverage.bits, [1, 2]);

        let other_spatial_record = TraversalCellId {
            spatial_index: 569,
            cell_index: 71,
        };
        assert!(visited
            .merge_incoming(other_spatial_record, &next_portal)
            .unwrap());
    }

    #[test]
    fn portal_propagation_uses_direct_merge_or_projected_clip_branch() {
        let cell = TraversalCellId {
            spatial_index: 12,
            cell_index: 3,
        };
        let mut incoming = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        incoming.bits[0] = u32::MAX;
        let mut visited = VisitedCellCoverage::default();

        assert!(visited
            .propagate_portal(cell, [0, 0, 4, 4], &incoming, PortalCoveragePath::Unclipped)
            .unwrap());
        assert!(!visited
            .propagate_portal(cell, [0, 0, 4, 4], &incoming, PortalCoveragePath::Unclipped)
            .unwrap());
        assert_eq!(visited.coverage(cell).unwrap().bits, [u32::MAX]);

        let projected = [
            [-0.9, -0.9, 0.9, 0.9],
            [-0.9, 0.9, 0.9, -0.9],
            [1.0; 4],
            [0.0; 4],
        ];
        let clipped_cell = TraversalCellId {
            spatial_index: 12,
            cell_index: 4,
        };
        assert!(visited
            .propagate_portal(
                clipped_cell,
                [0, 0, 4, 4],
                &incoming,
                PortalCoveragePath::Clipped {
                    projected,
                    face: [[0.0; 4]; 3],
                },
            )
            .unwrap());
        assert_eq!(visited.coverage(clipped_cell).unwrap().bits, [0x0000_8000]);
        assert!(!visited
            .propagate_portal(
                clipped_cell,
                [0, 0, 4, 4],
                &incoming,
                PortalCoveragePath::Clipped {
                    projected,
                    face: [[0.0; 4]; 3],
                },
            )
            .unwrap());
    }

    #[test]
    fn cell_traversal_state_matches_original_queued_and_processed_flags() {
        let queued = CellTraversalState::queued(0x123).unwrap();
        assert_eq!(queued.raw(), 0x8123);
        assert_eq!(queued.entry(), 0x123);
        assert!(queued.is_pending());

        let processed = CellTraversalState::processed(0x123).unwrap();
        assert_eq!(processed.raw(), 0x4123);
        assert_eq!(processed.entry(), 0x123);
        assert!(!processed.is_pending());
        assert!(!CellTraversalState::from_raw(0xc123).is_pending());
        assert!(CellTraversalState::queued(0x4000).is_err());
    }

    #[test]
    fn maps_target_spatial_bit_row_to_query_cell_slots() {
        // FUN_140a84f10 starts the row at target_spatial_index * bit_count. For
        // index 1 and three bits, this reads source bits 3..5 (1,0,1).
        let mapping = map_spatial_cell_slots(1, 3, 1, &[0b0010_1000], &[0, 0, 0, 0]).unwrap();
        assert_eq!(mapping.slots, vec![1, 1, 0]);
        assert_eq!(mapping.next_slot, 0);

        // The executable advances each packed-word read by the descriptor's
        // word stride, so this reads words 0 and 2 rather than contiguous 0/1.
        let mapping = map_spatial_cell_slots(0, 33, 2, &[1, 0xffff_ffff, 0], &[0; 40]).unwrap();
        assert_eq!(mapping.slots.len(), 33);
        assert_eq!(mapping.slots.first(), Some(&1));
        assert_eq!(mapping.next_slot, 1);
        assert_eq!(mapping.slots.last(), Some(&1));
    }

    #[test]
    fn target_spatial_slot_cache_reuses_final_slot_and_marks_only_intermediates() {
        let mut mapper = SpatialCellSlotMapper::default();
        let mut states = [0u16; 4];
        assert_eq!(mapper.resolve(1, 3, 1, &[0b0010_1000], &states).unwrap(), 0);
        assert_eq!(mapper.intermediate_slots(), &[1, 1]);

        // Same-cell cache hit returns the stored final slot without reading
        // the now deliberately unavailable bit row.
        assert_eq!(mapper.resolve(1, 3, 1, &[], &states).unwrap(), 0);
        mapper.mark_intermediate_processed(&mut states).unwrap();
        assert_eq!(states, [0, 0x4000, 0, 0]);

        // Requesting another target spatial record rebuilds the one-row cache.
        assert_eq!(mapper.resolve(0, 0, 1, &[], &states).unwrap(), 1);
        assert!(mapper.intermediate_slots().is_empty());
    }

    #[test]
    fn target_spatial_slot_mapping_rejects_incomplete_tables() {
        assert!(map_spatial_cell_slots(0, 1, 0, &[0], &[0; 2]).is_err());
        assert!(map_spatial_cell_slots(1, 32, 1, &[0], &[0; 2]).is_err());
        assert!(map_spatial_cell_slots(0, 3, 1, &[1], &[0, 1]).is_err());
    }

    #[test]
    fn parses_original_spatial_slot_rows_from_tome_header() {
        let mut bytes = vec![0; 0x104];
        bytes[0x90..0x94].copy_from_slice(&3u32.to_le_bytes());
        bytes[0x94..0x98].copy_from_slice(&3u32.to_le_bytes());
        bytes[0x98..0x9c].copy_from_slice(&0x100u32.to_le_bytes());
        bytes[0x100..0x104].copy_from_slice(&0b0010_1000u32.to_le_bytes());

        let rows = SpatialCellSlotRows::parse(&bytes).unwrap();
        assert_eq!(rows.spatial_record_count(), 3);
        assert_eq!(rows.bit_count(), 3);
        let mut mapper = SpatialCellSlotMapper::default();
        let states = [0u16; 4];
        assert_eq!(rows.resolve(&mut mapper, 1, &states).unwrap(), 0);
        assert_eq!(mapper.intermediate_slots(), &[1, 1]);
        assert!(rows.resolve(&mut mapper, 3, &states).is_err());
    }

    #[test]
    fn tests_each_original_portal_pixel_bit_against_object_rectangle() {
        let mut mask = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        for x in 0..8 {
            for y in 0..4 {
                let bit = if x < 4 {
                    y * 4 + x
                } else {
                    16 + y * 4 + (x - 4)
                };
                mask.bits[0] = 1 << bit;
                assert!(
                    mask.intersects_pixels([x, y, x + 1, y + 1]).unwrap(),
                    "bit {bit} at ({x}, {y})"
                );
                if x > 0 {
                    assert!(!mask.intersects_pixels([0, y, x, y + 1]).unwrap());
                }
            }
        }
        mask.bits[0] = 0;
        assert!(!mask.intersects_pixels([0, 0, 8, 4]).unwrap());
        assert!(mask.intersects_pixels([0, 0, 9, 4]).is_err());
        let mut offset = PortalTileMask::new([1, 2, 3, 4]).unwrap();
        offset.bits[3] = 1 << (16 + 2 * 4 + 1);
        assert!(offset.intersects_pixels([21, 14, 22, 15]).unwrap());
        assert!(!offset.intersects_pixels([8, 8, 21, 14]).unwrap());
    }

    #[test]
    fn rasterizes_original_small_and_medium_portal_tile_branches() {
        let mut destination = PortalTileMask::new([0, 0, 2, 3]).unwrap();
        let mut incoming = PortalTileMask::new([0, 0, 2, 3]).unwrap();
        incoming.bits.fill(u32::MAX);
        let projected = [[0.0; 4]; 4];
        let face = [[0.0; 4]; 3];
        assert!(destination
            .rasterize_projected_box(&incoming, [0, 0, 4, 4], projected, face)
            .unwrap());
        assert_eq!(destination.bits[0], 0x0000_ffff);
        assert!(!destination
            .rasterize_projected_box(&incoming, [0, 0, 4, 4], projected, face)
            .unwrap());
        destination.bits[0] = 0x0000_ff00;
        incoming.bits[0] = 0x5555_5555;
        assert!(destination
            .rasterize_projected_box(&incoming, [0, 0, 4, 4], projected, face)
            .unwrap());
        assert_eq!(destination.bits[0], 0x0000_ff55);
        assert!(destination
            .rasterize_projected_box(&incoming, [0, 0, 16, 12], projected, face)
            .unwrap());
        incoming.bits[0] = u32::MAX;
        let mut outlined = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        let projected = [
            [-0.9, -0.9, 0.9, 0.9],
            [-0.9, 0.9, 0.9, -0.9],
            [1.0; 4],
            [0.0; 4],
        ];
        assert!(outlined
            .rasterize_projected_box(&incoming, [0, 0, 4, 4], projected, face)
            .unwrap());
        assert_eq!(outlined.bits[0], 0x0000_8000);
    }

    fn check_original_raster_samples(cases: &str, rect: [u8; 4]) {
        let word_count = (rect[2] - rect[0]) as usize * (rect[3] - rect[1]) as usize;
        for (case, row) in cases.lines().enumerate() {
            let values = row.split_whitespace().collect::<Vec<_>>();
            assert_eq!(values.len(), 33 + word_count, "case {case}");
            let pixels = std::array::from_fn(|axis| values[axis].parse::<i32>().unwrap());
            let projected = std::array::from_fn(|vector| {
                std::array::from_fn(|lane| values[4 + vector * 4 + lane].parse::<f32>().unwrap())
            });
            let face = std::array::from_fn(|vector| {
                std::array::from_fn(|lane| values[20 + vector * 4 + lane].parse::<f32>().unwrap())
            });
            let expected_changed = values[32].parse::<u8>().unwrap() != 0;
            let expected = (0..word_count)
                .map(|index| values[33 + index].parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            let mut destination = PortalTileMask::new(rect).unwrap();
            let mut incoming = PortalTileMask::new(rect).unwrap();
            incoming.bits.fill(u32::MAX);
            let changed = destination
                .rasterize_projected_box(&incoming, pixels, projected, face)
                .unwrap();
            assert_eq!(changed, expected_changed, "case {case} changed");
            assert_eq!(destination.bits, expected, "case {case} mask");
        }
    }

    #[test]
    fn small_and_medium_portal_raster_matches_original_executable_samples() {
        check_original_raster_samples(
            include_str!("../analysis/original_portal_small_cases.txt"),
            [0, 0, 3, 3],
        );
        check_original_raster_samples(
            include_str!("../analysis/original_portal_strip_cases.txt"),
            [0, 0, 6, 11],
        );
        check_original_raster_samples(
            include_str!("../analysis/original_portal_medium_cases.txt"),
            [0, 0, 4, 6],
        );
    }

    #[test]
    fn large_portal_raster_matches_original_executable_samples() {
        check_original_raster_samples(
            include_str!("../analysis/original_portal_large_cases.txt"),
            [0, 0, 7, 13],
        );
        let rect = [0, 0, 8, 16];
        let word_count = 8 * 16;
        for (case, row) in include_str!("../analysis/original_portal_large_varied_cases.txt")
            .lines()
            .enumerate()
        {
            let values = row.split_whitespace().collect::<Vec<_>>();
            assert_eq!(values.len(), 33 + word_count * 3, "case {case}");
            let pixels = std::array::from_fn(|axis| values[axis].parse::<i32>().unwrap());
            let projected = std::array::from_fn(|vector| {
                std::array::from_fn(|lane| values[4 + vector * 4 + lane].parse::<f32>().unwrap())
            });
            let face = std::array::from_fn(|vector| {
                std::array::from_fn(|lane| values[20 + vector * 4 + lane].parse::<f32>().unwrap())
            });
            let mut incoming = PortalTileMask::new(rect).unwrap();
            let mut destination = PortalTileMask::new(rect).unwrap();
            for index in 0..word_count {
                incoming.bits[index] = values[32 + index].parse().unwrap();
                destination.bits[index] = values[32 + word_count + index].parse().unwrap();
            }
            let changed = destination
                .rasterize_projected_box(&incoming, pixels, projected, face)
                .unwrap();
            let expected_changed = values[32 + word_count * 2].parse::<u8>().unwrap() != 0;
            assert_eq!(changed, expected_changed, "case {case} changed");
            let expected = (0..word_count)
                .map(|index| values[33 + word_count * 2 + index].parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(destination.bits, expected, "case {case} mask");
        }
    }

    #[test]
    fn depth_grid_matches_original_executable_samples() {
        for (case, row) in include_str!("../analysis/original_depth_grid_cases.txt")
            .lines()
            .enumerate()
        {
            let values = row.split_whitespace().collect::<Vec<_>>();
            let x0 = values[0].parse::<i32>().unwrap();
            let y0 = values[1].parse::<i32>().unwrap();
            let x1 = values[2].parse::<i32>().unwrap();
            let y1 = values[3].parse::<i32>().unwrap();
            let threshold = values[4].parse::<f32>().unwrap();
            let count = values[5].parse::<usize>().unwrap();
            assert_eq!(values.len(), 7 + count * 3, "case {case}");
            let mut depths = vec![0.0f32; 64 * 64];
            for point in 0..count {
                let at = 6 + point * 3;
                let x = values[at].parse::<usize>().unwrap();
                let y = values[at + 1].parse::<usize>().unwrap();
                let tile = (y >> 2) * 16 + (x >> 2);
                depths[tile * 16 + ((y & 3) << 2) + (x & 3)] =
                    values[at + 2].parse::<f32>().unwrap();
            }
            let expected = values[6 + count * 3].parse::<u8>().unwrap() != 0;
            let actual =
                depth_grid_intersects_pixels(&depths, [x0, y0], [x1, y1], threshold).unwrap();
            assert_eq!(actual, expected, "case {case}");
        }
    }

    #[test]
    fn combined_mask_depth_matches_original_executable_samples() {
        for (case, row) in include_str!("../analysis/original_mask_depth_cases.txt")
            .lines()
            .enumerate()
        {
            let values = row
                .split_whitespace()
                .map(|number| number.parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            let pixels = [
                values[0] as i32,
                values[1] as i32,
                values[2] as i32,
                values[3] as i32,
            ];
            let threshold = f32::from_bits(values[4]);
            let mask_count = values[5] as usize;
            let mut mask = PortalTileMask::new([0, 0, 8, 16]).unwrap();
            for point in 0..mask_count {
                let x = values[6 + point * 2] as usize;
                let y = values[7 + point * 2] as usize;
                let index = (y / 4) * 8 + x / 8;
                let bit = ((x / 4) & 1) * 16 + (y & 3) * 4 + (x & 3);
                mask.bits[index] |= 1 << bit;
            }
            let depth_count_at = 6 + mask_count * 2;
            let depth_count = values[depth_count_at] as usize;
            let mut depths = vec![0.0f32; 64 * 64];
            for point in 0..depth_count {
                let at = depth_count_at + 1 + point * 3;
                let x = values[at] as usize;
                let y = values[at + 1] as usize;
                let slot = ((y / 4) * 16 + x / 4) * 16 + (y & 3) * 4 + (x & 3);
                depths[slot] = f32::from_bits(values[at + 2]);
            }
            let expected_at = depth_count_at + 1 + depth_count * 3;
            assert_eq!(values.len(), expected_at + 1, "case {case}");
            let actual = mask
                .intersects_depth_pixels(&depths, pixels, threshold)
                .unwrap();
            assert_eq!(actual, values[expected_at] != 0, "case {case}");
        }
    }

    #[test]
    fn depth_mask_raster_matches_original_executable_samples() {
        for (case, row) in include_str!("../analysis/original_depth_raster_cases.txt")
            .lines()
            .enumerate()
        {
            let numbers = row
                .split_whitespace()
                .map(|number| number.parse::<u64>().unwrap())
                .collect::<Vec<_>>();
            let rect = [
                numbers[0] as u8,
                numbers[1] as u8,
                numbers[2] as u8,
                numbers[3] as u8,
            ];
            let value = f32::from_bits(numbers[4] as u32);
            let previous = f32::from_bits(numbers[5] as u32);
            let count = numbers[6] as usize;
            let bits = numbers[7..7 + count]
                .iter()
                .map(|&word| word as u32)
                .collect::<Vec<_>>();
            let points = numbers[7 + count] as usize;
            assert_eq!(numbers.len(), 10 + count + points * 2, "case {case}");
            let mut depths = vec![0.0f32; 64 * 64];
            for point in 0..points {
                let at = 8 + count + point * 2;
                depths[numbers[at] as usize] = f32::from_bits(numbers[at + 1] as u32);
            }
            let maximum = rasterize_depth_mask(&mut depths, rect, &bits, value, previous).unwrap();
            assert_eq!(
                maximum.to_bits(),
                numbers[8 + count + points * 2] as u32,
                "case {case} maximum"
            );
            let digest = depths.iter().fold(0xcbf29ce484222325u64, |hash, depth| {
                (hash ^ depth.to_bits() as u64).wrapping_mul(0x100000001b3)
            });
            assert_eq!(digest, numbers[9 + count + points * 2], "case {case} grid");
        }
    }

    #[test]
    fn decodes_original_grouped_tome_object_ids() {
        let mut data = vec![0u8; 0x90];
        data[0x40..0x44].copy_from_slice(&3u32.to_le_bytes());
        data[0x4c..0x50].copy_from_slice(&0x60u32.to_le_bytes());
        data[0x50..0x54].copy_from_slice(&0x70u32.to_le_bytes());
        for (i, offset) in [0u32, 2, 2, 3].iter().enumerate() {
            data[0x60 + i * 4..0x64 + i * 4].copy_from_slice(&offset.to_le_bytes());
        }
        for (i, id) in [11u32, 12, 99].iter().enumerate() {
            data[0x70 + i * 4..0x74 + i * 4].copy_from_slice(&id.to_le_bytes());
        }
        assert_eq!(object_ids_for_index(&data, 0).unwrap(), [11, 12]);
        assert!(object_ids_for_index(&data, 1).unwrap().is_empty());
        assert_eq!(object_ids_for_index(&data, 2).unwrap(), [99]);
        assert!(object_ids_for_index(&data, 3).is_err());
        data[0x6c..0x70].copy_from_slice(&1u32.to_le_bytes());
        assert!(object_ids_for_index(&data, 2).is_err());
    }

    #[test]
    fn reconstructs_original_four_corner_order_from_query_planes() {
        let planes = [
            [0.0, 0.0, 1.0, -1.0],
            [0.0, 0.0, 1.0, -2.0],
            [1.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, -1.0],
        ];
        assert_eq!(
            first_depth_quad(planes).unwrap(),
            [
                [-1.0, -1.0, 1.0],
                [-1.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, -1.0, 1.0],
            ]
        );
        let mut degenerate = planes;
        degenerate[4] = degenerate[2];
        assert!(first_depth_quad(degenerate).is_err());
    }

    #[test]
    fn builds_original_render_view_reverse_depth_projection() {
        let finite =
            original_render_perspective_rows([-1.0, 1.0], [-1.0, 1.0], 1.0, 100.0).unwrap();
        assert_eq!(finite[0], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(finite[1], [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(finite[3], [0.0, 0.0, -1.0, 0.0]);
        let project_depth = |z: f32| {
            let clip_z = finite[2][2] * z + finite[2][3];
            clip_z / -z
        };
        assert!((project_depth(-1.0) - 1.0).abs() < 1e-6);
        assert!(project_depth(-100.0).abs() < 1e-6);

        let infinite =
            original_render_perspective_rows([-1.0, 1.0], [-1.0, 1.0], 1.0, 0.0).unwrap();
        assert_eq!(infinite[2], [0.0, 0.0, -1.0, 0.0]);
        assert!(original_render_perspective_rows([0.0, 0.0], [-1.0, 1.0], 1.0, 100.0).is_err());
    }

    #[test]
    fn world_visibility_query_replaces_depth_and_flips_portal_winding() {
        let pose = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let reverse =
            original_render_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0).unwrap();
        let visibility =
            original_world_visibility_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0)
                .unwrap();
        assert_eq!(visibility[0], reverse[0]);
        assert_eq!(visibility[1], reverse[1]);
        assert_eq!(visibility[3], reverse[3]);
        assert!((visibility[2][2] + 7000.0 / 6999.99).abs() < 1e-6);
        assert!((visibility[2][3] + 70.0 / 6999.99).abs() < 1e-6);
        assert!(!query_matrix_winding_flip(reverse));
        assert!(query_matrix_winding_flip(visibility));
    }

    #[test]
    fn derives_original_render_fov_from_reference_fov_and_pixel_aspect() {
        let (wide_horizontal, wide_vertical) =
            original_render_fov_radians(80.0, 1920, 1080, 1.0).unwrap();
        assert!((wide_horizontal.to_degrees() - 80.0).abs() < 1e-4);
        let expected_vertical = 2.0 * ((80.0_f32.to_radians() * 0.5).tan() * 0.5625).atan();
        assert!((wide_vertical - expected_vertical).abs() < 1e-6);

        let (narrow_horizontal, narrow_vertical) =
            original_render_fov_radians(80.0, 4, 3, 1.0).unwrap();
        assert!(narrow_horizontal < wide_horizontal);
        assert!((narrow_vertical - wide_vertical).abs() < 1e-6);

        assert!(original_render_fov_radians(80.0, 0, 1080, 1.0).is_err());
        assert!(original_render_fov_radians(80.0, 1920, 1080, 0.0).is_err());
        assert!(original_render_fov_radians(180.0, 1920, 1080, 1.0).is_err());
    }

    #[test]
    fn assembles_query_planes_from_original_matrix_rows() {
        let rows = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, -1.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let planes = query_planes_from_rows(rows, rows);
        assert_eq!(
            planes,
            [
                [0.0, 0.0, 1.0, -1.0],
                [0.0, 0.0, -1.0, 2.0],
                [-1.0, 0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0, 1.0],
                [0.0, -1.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
            ]
        );
        assert_eq!(
            first_depth_quad(planes).unwrap(),
            [
                [1.0, 1.0, 1.0],
                [1.0, -1.0, 1.0],
                [-1.0, -1.0, 1.0],
                [-1.0, 1.0, 1.0],
            ]
        );
    }

    #[test]
    fn depth_enabled_aabb_matches_installed_executable() {
        let fixture = include_str!("../analysis/native_depth_aabb_cases.txt");
        assert_eq!(fixture.lines().count(), 9000);
        for (line_number, line) in fixture.lines().enumerate() {
            let words: Vec<u32> = line.split_whitespace().map(|word| word.parse().unwrap()).collect();
            assert_eq!(words.len(), 64);
            let viewport = std::array::from_fn(|axis| words[3 + axis] as i32);
            let render_rows = std::array::from_fn(|row| {
                std::array::from_fn(|column| f32::from_bits(words[10 + row * 4 + column]))
            });
            let grid = std::array::from_fn(|i| match words[0] {
                0 => 0.0,
                1 => f32::from_bits(0x3e4c_cccd),
                2 => 1.0,
                3 => (i * 7 % 101) as f32 / 100.0,
                4 => if i % 67 == 0 { 1.0 } else { 0.0 },
                _ => panic!("unknown native fixture grid pattern"),
            });
            let state = RenderQueryState {
                render_rows,
                cropped_rows: crop_query_rows(render_rows, viewport).unwrap(),
                camera_point: [0.0; 3],
                camera_imprecision: std::array::from_fn(|axis| f32::from_bits(words[7 + axis])),
                winding_flip: false,
                viewport,
                maximum_depth: 0.0,
                depth_grid: Some(Box::new(grid)),
            };
            let bounds = std::array::from_fn(|side| {
                std::array::from_fn(|axis| f32::from_bits(words[26 + side * 3 + axis]))
            });
            let query = RenderAabbQuery::from_state(&state).unwrap();
            let native_planes: [[f32; 4]; 6] = std::array::from_fn(|plane| {
                std::array::from_fn(|axis| f32::from_bits(words[32 + plane * 4 + axis]))
            });
            let native_bounds: [[f32; 4]; 2] = std::array::from_fn(|side| {
                std::array::from_fn(|axis| f32::from_bits(words[56 + side * 4 + axis]))
            });
            assert_eq!(query.planes, native_planes, "depth fixture {line_number}: query planes");
            assert_eq!(query.query_bounds, native_bounds, "depth fixture {line_number}: query bounds");
            let (visible, area) = query.visibility_and_area(bounds).unwrap();
            assert_eq!(visible, words[1] != 0, "depth fixture {line_number}: {bounds:?}");
            assert_eq!(area.to_bits(), words[2], "depth fixture {line_number}: area {area}");
            assert_eq!(query.visible(bounds).unwrap(), visible);
        }
    }

    #[test]
    fn no_depth_light_aabb_area_matches_installed_instructions() {
        let mut cases = 0;
        for (line_number, line) in include_str!("../analysis/native_light_aabb_cases.txt")
            .lines()
            .enumerate()
        {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            assert_eq!(fields.len(), 77, "native case {}", line_number + 1);
            let f = |index: usize| fields[index].parse::<f32>().unwrap();
            let initialized = fields[3] == "1";
            let expected_visible = fields[11] == "1";
            let expected_area = fields[12].parse::<u32>().unwrap();
            if initialized {
                let padding = f(4);
                let expanded = [
                    [f(5) - padding, f(6) - padding, f(7) - padding, 1.0],
                    [f(8) + padding, f(9) + padding, f(10) + padding, 1.0],
                ];
                let query_bounds =
                    std::array::from_fn(|side| std::array::from_fn(|axis| f(13 + side * 4 + axis)));
                let planes = (0..6)
                    .map(|plane| std::array::from_fn(|axis| f(21 + plane * 4 + axis)))
                    .collect::<Vec<_>>();
                let rows =
                    std::array::from_fn(|row| std::array::from_fn(|axis| f(45 + row * 4 + axis)));
                let (visible, area) =
                    query_aabb_visibility_without_depth(query_bounds, &planes, expanded, rows)
                        .unwrap();
                assert_eq!(
                    visible,
                    expected_visible,
                    "native visibility case {}",
                    line_number + 1
                );
                assert_eq!(
                    area.to_bits(),
                    expected_area,
                    "native area case {}: {area}",
                    line_number + 1
                );
                let render_rows =
                    std::array::from_fn(|row| std::array::from_fn(|axis| f(61 + row * 4 + axis)));
                assert_eq!(
                    rows,
                    std::array::from_fn(|column| std::array::from_fn(
                        |row| render_rows[row][column]
                    ))
                );
                let state = RenderQueryState {
                    render_rows,
                    cropped_rows: crop_query_rows(
                        render_rows,
                        render_query_job_tile(
                            fields[1].parse().unwrap(),
                            fields[2].parse().unwrap(),
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                    camera_point: [0.0; 3],
                    camera_imprecision: [padding; 3],
                    winding_flip: false,
                    viewport: [0, 0, 64, 64],
                    maximum_depth: 0.0,
                    depth_grid: None,
                };
                let bounds = [[f(5), f(6), f(7)], [f(8), f(9), f(10)]];
                let prepared = NoDepthAabbQuery::from_state(&state).unwrap();
                assert_eq!(prepared.visible(bounds).unwrap(), expected_visible);
                let (prepared_visible, prepared_area) =
                    prepared.visibility_and_area(bounds).unwrap();
                assert_eq!(prepared_visible, expected_visible);
                assert_eq!(prepared_area.to_bits(), expected_area);
                let (visible, area) =
                    render_query_aabb_visibility_without_depth(Some(&state), bounds).unwrap();
                assert_eq!(
                    visible,
                    expected_visible,
                    "typed native visibility case {}",
                    line_number + 1
                );
                assert_eq!(
                    area.unwrap().to_bits(),
                    expected_area,
                    "typed native area case {}",
                    line_number + 1
                );
                assert_eq!(
                    render_query_aabb_visible_without_depth(Some(&state), bounds).unwrap(),
                    expected_visible
                );
            } else {
                assert!(expected_visible);
                assert_eq!(expected_area, 123.0f32.to_bits());
                assert_eq!(
                    render_query_aabb_visibility_without_depth(None, [[0.0; 3]; 2]).unwrap(),
                    (true, None)
                );
            }
            cases += 1;
        }
        assert_eq!(cases, 4800);
    }

    #[test]
    #[ignore = "manual CPU measurement of descriptor setup lifetime"]
    fn prepared_spatial_query_cpu_measurement() {
        use std::{hint::black_box, time::Instant};
        let line = include_str!("../analysis/native_light_aabb_cases.txt")
            .lines()
            .find(|line| line.starts_with("1 0 1 1 0.0 "))
            .unwrap();
        let fields: Vec<_> = line.split_whitespace().collect();
        let f = |i: usize| fields[i].parse::<f32>().unwrap();
        let rows = std::array::from_fn(|r| std::array::from_fn(|a| f(61 + r * 4 + a)));
        let state = RenderQueryState {
            render_rows: rows,
            cropped_rows: rows,
            camera_point: [0.0; 3],
            camera_imprecision: [0.0; 3],
            winding_flip: false,
            viewport: [0, 0, 64, 64],
            maximum_depth: 0.0,
            depth_grid: None,
        };
        let bounds = [[f(5), f(6), f(7)], [f(8), f(9), f(10)]];
        let count = 100_000;
        let start = Instant::now();
        let mut repeated = 0;
        for _ in 0..count {
            repeated += usize::from(
                render_query_aabb_visible_without_depth(Some(black_box(&state)), black_box(bounds))
                    .unwrap(),
            );
        }
        let repeated_time = start.elapsed();
        let start = Instant::now();
        let query = NoDepthAabbQuery::from_state(black_box(&state)).unwrap();
        let mut prepared = 0;
        for _ in 0..count {
            prepared += usize::from(query.visible(black_box(bounds)).unwrap());
        }
        let prepared_time = start.elapsed();
        assert_eq!(repeated, prepared);
        eprintln!("spatial CPU synthetic {count} boxes: per-object setup={repeated_time:?}, once-per-frame setup={prepared_time:?}; identical bools; not FPS");
    }

    #[test]
    fn query_corners_and_bounds_match_installed_instructions() {
        let mut checked = 0;
        for line in include_str!("../analysis/native_query_corner_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let f = |i: usize| fields[i].parse::<f32>().unwrap();
            let finite = fields[1] == "1";
            assert_eq!(fields.len(), if finite { 58 } else { 34 });
            let planes = std::array::from_fn(|p| std::array::from_fn(|axis| f(2 + p * 4 + axis)));
            let expected: [[f32; 4]; 2] =
                std::array::from_fn(|side| std::array::from_fn(|axis| f(26 + side * 4 + axis)));
            let bounds = query_bounds_from_planes(&planes, [f(0); 3]).unwrap();
            assert_eq!(
                bounds.map(|row| row.map(f32::to_bits)),
                expected.map(|row| row.map(f32::to_bits)),
                "native bounds case {checked}"
            );
            if finite {
                for index in 0..8 {
                    let expected: [f32; 3] = std::array::from_fn(|axis| f(34 + index * 3 + axis));
                    assert_eq!(
                        query_frustum_corner(&planes, index)
                            .unwrap()
                            .map(f32::to_bits),
                        expected.map(f32::to_bits),
                        "native corner case {checked}/{index}"
                    );
                }
            }
            checked += 1;
        }
        assert_eq!(checked, 60);
        assert!(query_frustum_corner(&[[0.0; 4]; 6], 8).is_err());
        assert!(query_bounds_from_planes(&[[0.0; 4]; 6], [f32::NAN; 3]).is_err());
    }

    #[test]
    fn light_area_threshold_uses_the_original_64_numerator() {
        assert_eq!(
            minimum_light_projected_area_metric(1920, 1080)
                .unwrap()
                .to_bits(),
            (64.0f32 / 2_073_600.0).to_bits()
        );
        assert!(minimum_light_projected_area_metric(0, 1080).is_err());
        assert!(minimum_light_projected_area_metric(1920, i32::MAX).is_err());
    }

    #[test]
    fn selects_active_query_planes_using_original_sign_masks() {
        let planes = [
            [1.0, 0.0, 0.0, -2.0],
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, -1.0],
            [0.0, 0.0, 1.0, -3.0],
        ];
        let bounds = [[0.0, 0.0, 0.0, 1.0], [1.0, 2.0, 4.0, 1.0]];
        assert_eq!(active_query_planes(&planes, bounds), [0, 1, 2, 3]);
    }

    #[test]
    fn active_query_planes_match_original_executable_cases() {
        let mut cases = 0;
        for (line_number, line) in include_str!("../analysis/original_active_plane_cases.txt")
            .lines()
            .enumerate()
        {
            let values = line
                .split_whitespace()
                .map(|value| value.parse::<f32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(values.len(), 33, "fixture row {}", line_number + 1);
            let bounds = [
                values[0..4].try_into().unwrap(),
                values[4..8].try_into().unwrap(),
            ];
            let planes = values[8..32]
                .chunks_exact(4)
                .map(|plane| plane.try_into().unwrap())
                .collect::<Vec<[f32; 4]>>();
            let expected_mask = values[32] as u32;
            let actual_mask = active_query_planes(&planes, bounds)
                .into_iter()
                .fold(0, |mask, index| mask | (1 << index));
            assert_eq!(
                actual_mask,
                expected_mask,
                "fixture row {}",
                line_number + 1
            );
            cases += 1;
        }
        assert_eq!(cases, 256);
    }

    #[test]
    fn crops_query_matrix_on_original_sixty_four_unit_grid() {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        assert_eq!(crop_query_rows(identity, [0, 0, 64, 64]).unwrap(), identity);
        let left_half = crop_query_rows(identity, [0, 0, 32, 64]).unwrap();
        assert_eq!(
            left_half,
            [
                [2.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        );
        let planes = query_planes_from_rows(identity, left_half);
        assert_eq!(planes[2], [-2.0, 0.0, 0.0, 0.0]);
        assert_eq!(planes[3], [2.0, 0.0, 0.0, 2.0]);
        assert!(crop_query_rows(identity, [0, 0, 0, 64]).is_err());
    }

    #[test]
    fn render_job_partition_and_crop_match_installed_executable() {
        let cases = include_str!("../analysis/original_query_job_cases.txt");
        let identity = std::array::from_fn(|row| {
            std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
        });
        let mut checked = 0;
        for (case, line) in cases.lines().enumerate() {
            let values: Vec<f32> = line
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            assert_eq!(values.len(), 22);
            let tile = render_query_job_tile(values[1] as u32, values[0] as u32).unwrap();
            assert_eq!(
                tile,
                std::array::from_fn(|axis| values[2 + axis] as i32),
                "case {case}"
            );
            let cropped = crop_query_rows(identity, tile).unwrap();
            let planes = query_planes_from_rows(identity, cropped);
            for (actual, expected) in planes[2..].iter().flatten().zip(&values[6..]) {
                assert!(
                    (actual - expected).abs() <= 1e-6,
                    "case {case}: {actual} != {expected}"
                );
            }
            checked += 1;
        }
        assert_eq!(checked, 136);
    }

    #[test]
    fn render_query_state_merge_matches_installed_executable() {
        fn grid(source: bool) -> Box<[f32; 4096]> {
            Box::new(std::array::from_fn(|i| {
                if i % 29 == 0 {
                    f32::from_bits(if source { 0x7fc01234 } else { 0x7fc05678 })
                } else if i % 17 == 0 {
                    f32::from_bits(if source { 0 } else { 0x80000000 })
                } else {
                    ((i * if source { 17 } else { 13 }) % 257) as f32 / 64.0 - 2.0
                }
            }))
        }
        fn hash_grid(grid: &[f32; 4096]) -> u64 {
            grid.iter().fold(0xcbf29ce484222325, |hash, value| {
                value
                    .to_bits()
                    .to_le_bytes()
                    .iter()
                    .fold(hash, |hash, byte| {
                        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
                    })
            })
        }
        let identity = std::array::from_fn(|row| {
            std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
        });
        let modes = [
            (false, true, false, false),
            (false, true, false, true),
            (true, false, true, true),
            (true, true, true, true),
            (true, true, true, false),
            (true, true, false, true),
            (true, true, true, true),
            (true, true, true, true),
            (false, false, false, false),
        ];
        let mut checked = 0;
        for (case, line) in include_str!("../analysis/original_query_state_cases.txt")
            .lines()
            .enumerate()
        {
            let values: Vec<u64> = line
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            let count = values[0] as u32;
            let job = values[1] as u32;
            let mode = values[2] as usize;
            let (destination_valid, source_valid, destination_grid, source_grid) = modes[mode];
            let make = |source: bool, has_grid: bool| {
                let viewport = render_query_job_tile(
                    if source { job } else { 0 },
                    if source { count } else { 4 },
                )
                .unwrap();
                let rows = if source && job % 2 != 0 {
                    [
                        [2.0, 0.0, 0.0, 0.0],
                        [0.0, 3.0, 0.0, 0.0],
                        [0.0, 0.0, 4.0, 0.0],
                        [0.0, 0.0, 0.0, 1.0],
                    ]
                } else {
                    identity
                };
                RenderQueryState {
                    render_rows: rows,
                    cropped_rows: crop_query_rows(rows, viewport).unwrap(),
                    camera_point: if source {
                        [1.0, 2.0, 3.0]
                    } else {
                        [-1.0, -2.0, -3.0]
                    },
                    camera_imprecision: [0.0; 3],
                    winding_flip: false,
                    viewport,
                    maximum_depth: if mode == 6 && !source {
                        f32::from_bits(0x7fc12345)
                    } else if mode == 7 {
                        f32::from_bits(if source { 0 } else { 0x80000000 })
                    } else if source {
                        0.75
                    } else {
                        0.25
                    },
                    depth_grid: has_grid.then(|| grid(source)),
                }
            };
            let mut destination = destination_valid.then(|| make(false, destination_grid));
            let source = source_valid.then(|| make(true, source_grid));
            merge_render_query_state(&mut destination, source.as_ref()).unwrap();
            assert_eq!(destination.is_some(), values[3] != 0, "case {case}");
            if let Some(destination) = destination {
                assert_eq!(
                    destination.viewport,
                    std::array::from_fn(|i| values[4 + i] as i32),
                    "case {case}"
                );
                assert_eq!(
                    u64::from(destination.maximum_depth.to_bits()),
                    values[8],
                    "case {case}"
                );
                assert_eq!(
                    destination.camera_point.map(|v| u64::from(v.to_bits())),
                    [values[9], values[10], values[11]],
                    "case {case}"
                );
                assert_eq!(
                    destination.depth_grid.is_some(),
                    values[12] != 0,
                    "case {case}"
                );
                if let Some(grid) = &destination.depth_grid {
                    assert_eq!(hash_grid(grid), values[13], "entire grid case {case}");
                    if !destination_valid {
                        assert_ne!(
                            grid.as_ptr(),
                            source
                                .as_ref()
                                .unwrap()
                                .depth_grid
                                .as_ref()
                                .unwrap()
                                .as_ptr()
                        );
                    }
                }
                let planes =
                    query_planes_from_rows(destination.render_rows, destination.cropped_rows);
                for (actual, expected) in planes[2..].iter().flatten().zip(&values[14..]) {
                    assert!(
                        (actual - f32::from_bits(*expected as u32)).abs() <= 1e-6,
                        "plane case {case}"
                    );
                }
            }
            checked += 1;
        }
        assert_eq!(checked, 342);
    }

    #[test]
    fn builds_original_row_major_query_matrix_from_camera_pose() {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let pose = [
            [0.0, -1.0, 0.0, 10.0],
            [1.0, 0.0, 0.0, 20.0],
            [0.0, 0.0, 1.0, 30.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        assert_eq!(
            render_query_base_rows(identity, pose),
            [
                [0.0, 1.0, 0.0, -20.0],
                [-1.0, 0.0, 0.0, 10.0],
                [0.0, 0.0, 1.0, -30.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        );
    }

    #[test]
    fn composes_game_default_projection_with_original_camera_pose() {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let matrix =
            original_render_query_matrix(80.0, 1920, 1080, 1.0, identity, 0.01, 7000.0).unwrap();
        assert!(matrix[0][0] > 0.0);
        assert!(matrix[1][1] > matrix[0][0]);
        assert_eq!(matrix[3], [0.0, 0.0, -1.0, 0.0]);
        assert!(
            original_render_query_matrix(80.0, 1920, 1080, 1.0, identity, 0.0, 7000.0).is_err()
        );
    }

    #[test]
    fn partitions_render_query_jobs_on_original_grid() {
        assert_eq!(render_query_job_tile(0, 1).unwrap(), [0, 0, 64, 64]);
        assert_eq!(render_query_job_tile(0, 2).unwrap(), [0, 0, 32, 64]);
        assert_eq!(render_query_job_tile(1, 2).unwrap(), [32, 0, 64, 64]);
        assert_eq!(render_query_job_tile(1, 3).unwrap(), [32, 0, 48, 64]);
        for count in 1..=16 {
            let mut covered = [[false; 64]; 64];
            for job in 0..count {
                let [left, top, right, bottom] = render_query_job_tile(job, count).unwrap();
                assert!(0 <= left && left < right && right <= 64);
                assert!(0 <= top && top < bottom && bottom <= 64);
                for y in top..bottom {
                    for x in left..right {
                        let cell = &mut covered[y as usize][x as usize];
                        assert!(!*cell, "job tiles overlap for count {count}");
                        *cell = true;
                    }
                }
            }
            assert!(
                covered.iter().flatten().all(|&cell| cell),
                "job tiles leave a gap for count {count}"
            );
        }
        assert!(render_query_job_tile(0, 0).is_err());
        assert!(render_query_job_tile(16, 16).is_err());
    }

    #[test]
    fn original_render_job_settings_and_inputs() {
        assert_eq!(ORIGINAL_DEFAULT_QUERY_JOB_COUNT, 12);
        assert_eq!(ORIGINAL_DEFAULT_SCHEDULER_THREADING_MODE, 2);
        assert_eq!(
            original_world_query_job_count(ORIGINAL_DEFAULT_QUERY_JOB_COUNT, 2).unwrap(),
            12
        );
        assert_eq!(original_world_query_job_count(1, 2).unwrap(), 1);
        assert_eq!(original_world_query_job_count(32, 2).unwrap(), 16);
        assert_eq!(original_world_query_job_count(16, 0).unwrap(), 1);
        assert_eq!(original_world_query_job_count(4, 1).unwrap(), 4);
        assert!(original_world_query_job_count(0, 2).is_err());
        assert!(original_world_query_job_count(33, 2).is_err());
        assert!(original_world_query_job_count(1, 3).is_err());
        let rows = [
            [2.0, 0.0, 0.0, 3.0],
            [0.0, 4.0, 0.0, 5.0],
            [0.0, 0.0, 6.0, 7.0],
            [0.0, 0.0, -1.0, 0.0],
        ];
        let input = OrdinaryDirectQueryInput {
            camera_point: [1.0, 2.0, 3.0],
            render_rows: rows,
            cropped_rows: rows,
            initial_tile_rect: [0, 0, 8, 16],
            camera_imprecision: [0.6; 3],
            object_imprecision: 0.6,
            minimum_area_metric: 0.1,
            global_distance_threshold: ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            winding_flip: false,
            gate_states: None,
            output_capacity: ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
        };
        for count in 1..=16 {
            for job in 0..count {
                let tile = render_query_job_tile(job, count).unwrap();
                let split = input.for_render_job(job, count).unwrap();
                assert_eq!(split.render_rows, rows);
                assert_eq!(split.cropped_rows, crop_query_rows(rows, tile).unwrap());
                assert_eq!(split.initial_tile_rect, query_viewport_tile_bounds(tile));
                assert_eq!(split.minimum_area_metric, input.minimum_area_metric);
                assert_eq!(split.camera_point, input.camera_point);
                assert_eq!(split.output_capacity, 65536);
            }
        }
    }

    #[test]
    fn render_job_merge_keeps_order_metrics_and_clears_failed_jobs() {
        let mut first = VisibleObjectIndexOutput::new(4, 4);
        first.push_unique_with_area_metric(1, Some(0.25)).unwrap();
        let mut second = VisibleObjectIndexOutput::new(4, 4);
        second.push_unique_with_area_metric(1, Some(0.5)).unwrap();
        second.push_unique_with_area_metric(2, Some(0.75)).unwrap();
        first.cells = Some(VisibleCellIndexOutput::new(4, 4));
        first.cells.as_mut().unwrap().push_unique(3).unwrap();
        second.cells = Some(VisibleCellIndexOutput::new(4, 4));
        second.cells.as_mut().unwrap().push_unique(3).unwrap();
        second.cells.as_mut().unwrap().push_unique(0).unwrap();
        let make_job =
            |job_index, result: Result<Option<VisibleObjectIndexOutput>, String>| RenderQueryJob {
                diagnostics: RenderQueryJobDiagnostics {
                    job_index,
                    viewport: None,
                    elapsed_ns: 0,
                    progress: OrdinaryQueryDiagnostics::default(),
                    error: result.as_ref().err().cloned(),
                },
                state: None,
                result,
            };
        let mut jobs = vec![make_job(0, Ok(Some(first))), make_job(1, Ok(Some(second)))];
        let rows = std::array::from_fn(|row| {
            std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
        });
        for (index, job) in jobs.iter_mut().enumerate() {
            let viewport = render_query_job_tile(index as u32, 2).unwrap();
            job.state = Some(RenderQueryState {
                render_rows: rows,
                cropped_rows: crop_query_rows(rows, viewport).unwrap(),
                camera_point: [0.0; 3],
                camera_imprecision: [0.6; 3],
                winding_flip: false,
                viewport,
                maximum_depth: 0.0,
                depth_grid: None,
            });
        }
        let output = merge_render_query_outputs(&jobs).unwrap().unwrap();
        assert_eq!(output.state.as_ref().unwrap().viewport, [0, 0, 64, 64]);
        assert_eq!(output.state.as_ref().unwrap().cropped_rows, rows);
        assert_eq!(output.indices(), &[1, 1, 2]);
        assert_eq!(output.area_metrics(), &[Some(0.25), Some(0.5), Some(0.75)]);
        assert_eq!(output.cell_indices(), &[3, 3, 0]);
        jobs[1].result = Err("late object-pass failure".into());
        let output = merge_render_query_outputs(&jobs).unwrap().unwrap();
        assert_eq!(output.indices(), &[1]);
        assert_eq!(output.area_metrics(), &[Some(0.25)]);
        assert_eq!(output.cell_indices(), &[3]);
        assert_eq!(output.state.as_ref().unwrap().viewport, [0, 0, 64, 64]);
        jobs[0].result = Err("first worker also failed".into());
        assert!(merge_render_query_outputs(&jobs)
            .unwrap()
            .unwrap()
            .indices()
            .is_empty());
        jobs[0].result = Ok(None);
        jobs[1].result = Ok(None);
        let empty = merge_render_query_outputs(&jobs).unwrap().unwrap();
        assert!(empty.indices().is_empty());
        assert!(empty.area_metrics().is_empty());
        assert_eq!(empty.state.as_ref().unwrap().viewport, [0, 0, 64, 64]);
        jobs[0].state = None;
        jobs[1].state = None;
        assert!(merge_render_query_outputs(&jobs).unwrap().is_none());
        jobs.swap(0, 1);
        assert!(merge_render_query_outputs(&jobs).is_err());
    }

    #[test]
    fn render_job_merge_grows_beyond_per_worker_query_capacity() {
        let capacity = ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY;
        let jobs: [_; 2] = std::array::from_fn(|job| {
            let mut output = VisibleObjectIndexOutput::new(capacity, capacity);
            for index in 0..capacity {
                output
                    .push_unique_with_area_metric(index as u32, Some(job as f32 + 0.25))
                    .unwrap();
            }
            RenderQueryJob {
                diagnostics: RenderQueryJobDiagnostics {
                    job_index: job as u32,
                    viewport: None,
                    elapsed_ns: 0,
                    progress: OrdinaryQueryDiagnostics::default(),
                    error: None,
                },
                state: None,
                result: Ok(Some(output)),
            }
        });
        let output = merge_render_query_outputs(&jobs).unwrap().unwrap();
        assert_eq!(output.indices().len(), 2 * capacity);
        assert_eq!(output.area_metrics().len(), output.indices().len());
        assert_eq!(&output.indices()[..capacity], &output.indices()[capacity..]);
        for job in 0..2 {
            let entries = job * capacity..(job + 1) * capacity;
            assert!(output.area_metrics()[entries]
                .iter()
                .all(|&area| area == Some(job as f32 + 0.25)));
        }
    }

    #[test]
    #[ignore = "requires local original-game EOTW Tome fixture"]
    fn cached_direct_query_matches_uncached_installed_tome() {
        let bytes = std::fs::read("artifacts/eotw_p_lowchaos.tome")
            .expect("missing local EOTW Tome fixture");
        let tree = SpatialTree::parse(&bytes).unwrap();
        let data = DirectTomeQueryData::parse(&bytes, &tree).unwrap();
        let gates = initial_gate_states(&bytes).unwrap();
        assert!(data.decoded_record_count() > 0);
        for point in [
            [-43.0, 138.0, 48.0],
            [-36.249935, 107.33754, 22.55],
            [100_000.0, 0.0, 0.0],
        ] {
            for yaw in [0.0f32, 0.5, 90.0, 180.0, 270.0] {
                let (sin, cos) = yaw.to_radians().sin_cos();
                let pose = [
                    [cos, 0.0, sin, point[0]],
                    [0.0, 1.0, 0.0, point[1]],
                    [-sin, 0.0, cos, point[2]],
                    [0.0, 0.0, 0.0, 1.0],
                ];
                let rows = original_world_visibility_query_matrix(
                    80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0,
                )
                .unwrap();
                let input = OrdinaryDirectQueryInput {
                    camera_point: point,
                    render_rows: rows,
                    cropped_rows: rows,
                    initial_tile_rect: [0, 0, 8, 16],
                    camera_imprecision: [0.6; 3],
                    object_imprecision: 0.6,
                    minimum_area_metric: 25.0 / (1920.0 * 1080.0),
                    global_distance_threshold: ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                    winding_flip: query_matrix_winding_flip(rows),
                    gate_states: Some(&gates),
                    output_capacity: ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
                };
                let mut reference_progress = OrdinaryQueryDiagnostics::default();
                let reference = ordinary_direct_tome_visible_objects_with_diagnostics(
                    &tree,
                    &bytes,
                    &input,
                    &mut reference_progress,
                );
                let cached = ordinary_direct_tome_render_job(&tree, &bytes, &data, &input, 0, 1);
                assert_eq!(cached.diagnostics.error, reference.as_ref().err().cloned());
                if point[0] == 100_000.0 {
                    assert_eq!(
                        cached.diagnostics.error.as_deref(),
                        Some("Tome query input outside root bounds (error 5)")
                    );
                }
                match (reference, cached.result) {
                    (Ok(Some(reference)), Ok(Some(cached))) => {
                        assert_eq!(cached.indices(), reference.indices());
                        assert_eq!(cached.area_metrics(), reference.area_metrics());
                        assert_eq!(cached.count(), reference.count());
                    }
                    (Ok(None), Ok(None)) => {}
                    (Err(reference), Err(cached)) => assert_eq!(reference, cached),
                    _ => panic!("cached Tome result differs at {point:?}, yaw {yaw}"),
                }
                assert_eq!(
                    cached.diagnostics.progress.cells_processed,
                    reference_progress.cells_processed
                );
            }
        }
    }

    #[test]
    fn tests_quad_edge_segments_against_spatial_boxes() {
        let bounds = [[-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]];
        assert!(segment_intersects_aabb(
            bounds,
            [[-2.0, 0.0, 0.0], [2.0, 0.0, 0.0]]
        ));
        assert!(segment_intersects_aabb(
            bounds,
            [[-2.0, 1.0, 0.0], [2.0, 1.0, 0.0]]
        ));
        assert!(!segment_intersects_aabb(
            bounds,
            [[-2.0, 2.0, 0.0], [2.0, 2.0, 0.0]]
        ));
        assert!(!segment_intersects_aabb(
            bounds,
            [[-2.0, 2.0, 2.0], [2.0, 2.0, 2.0]]
        ));
    }

    #[test]
    fn tests_original_four_corner_volume_against_spatial_box() {
        let bounds = [[-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]];
        let quad = |extent: f32, z: f32| {
            [
                [-extent, -extent, z],
                [-extent, extent, z],
                [extent, extent, z],
                [extent, -extent, z],
            ]
        };
        assert!(quad_intersects_aabb(bounds, quad(0.5, 0.0)));
        assert!(quad_intersects_aabb(bounds, quad(2.0, 0.0)));
        assert!(quad_intersects_aabb(bounds, quad(2.0, 1.0)));
        assert!(quad_intersects_aabb(
            bounds,
            [
                [-2.0, -0.5, 0.0],
                [-2.0, 0.5, 0.0],
                [2.0, 0.5, 0.0],
                [2.0, -0.5, 0.0],
            ]
        ));
        assert!(!quad_intersects_aabb(bounds, quad(2.0, 2.0)));
        assert!(!quad_intersects_aabb(
            bounds,
            [
                [2.0, -2.0, 0.0],
                [2.0, 2.0, 0.0],
                [3.0, 2.0, 0.0],
                [3.0, -2.0, 0.0],
            ]
        ));
    }

    #[test]
    fn reads_link_gate_indices_from_header_table() {
        assert_eq!(direct_spatial_link_target([17, 0, 0, 0]), Some(17));
        assert_eq!(
            direct_spatial_link_target([(1 << 27) | 17, 0, 0, 0]),
            Some(17)
        );
        assert_eq!(direct_spatial_link_target([(1 << 28) | 17, 0, 0, 0]), None);
        assert_eq!(
            packed_link_geometry([(2 << 30) | (1 << 29) | 17, 0x1234, 0x2000_8000, 0x4000_a000]),
            Some(PackedLinkGeometry {
                destination: LinkDestination::Spatial(17),
                axis: 2,
                positive_side: true,
                plane: 0x1234,
                spans: [[0x2000, 0x8000], [0x4000, 0xa000]],
            })
        );
        assert_eq!(
            packed_link_geometry([(1 << 28) | (2 << 29) | 17, 0x1234, 0x2000_8000, 0x4000_a000])
                .unwrap()
                .destination,
            LinkDestination::BoundaryFace(2)
        );
        assert_eq!(packed_link_geometry([(1 << 27) | 17, 0, 0, 0]), None);
        let facing = packed_link_geometry([(1 << 29) | 17, 50, 0, 0]).unwrap();
        assert!(packed_link_faces_volume(
            facing,
            [52, 0, 0, 0],
            [2, 0, 0, 0]
        ));
        assert!(!packed_link_faces_volume(
            facing,
            [53, 0, 0, 0],
            [2, 0, 0, 0]
        ));
        assert!(packed_link_faces_point(facing, [true, false, false, false]));
        assert!(!packed_link_faces_point(
            facing,
            [false, false, false, false]
        ));
        let opposite = packed_link_geometry([17, 50, 0, 0]).unwrap();
        assert!(packed_link_faces_volume(
            opposite,
            [48, 0, 0, 0],
            [2, 0, 0, 0]
        ));
        assert!(!packed_link_faces_volume(
            opposite,
            [47, 0, 0, 0],
            [2, 0, 0, 0]
        ));
        let mut data = vec![0u8; 0x120];
        data[0x68..0x6c].copy_from_slice(&4u32.to_le_bytes());
        data[0x30..0x34].copy_from_slice(&1.0f32.to_le_bytes());
        data[0x70..0x74].copy_from_slice(&0xc0u32.to_le_bytes());
        data[0x74..0x78].copy_from_slice(&3u32.to_le_bytes());
        data[0x78..0x7c].copy_from_slice(&0x100u32.to_le_bytes());
        for (index, value) in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0].iter().enumerate() {
            data[0xcc + index * 4..0xd0 + index * 4].copy_from_slice(&value.to_le_bytes());
        }
        assert_eq!(
            gated_link_bounds(&data, [(1 << 27) | 17, 0, 0, 1 << 12]).unwrap(),
            Some([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]])
        );
        assert_eq!(
            gated_link_query_bounds(&data, 0, [(1 << 27) | 17, 0, 0, 1 << 12]).unwrap(),
            Some([[0.0, 1.0, 2.0], [5.0, 6.0, 7.0]])
        );
        assert_eq!(gated_link_bounds(&data, [17, 0, 0, 1 << 12]).unwrap(), None);
        for (index, gate) in [0u32, 2, 3].iter().enumerate() {
            data[0x100 + index * 4..0x104 + index * 4].copy_from_slice(&gate.to_le_bytes());
        }
        assert_eq!(
            gate_indices_for_link(&data, [1 << 27, 0, (1 << 12) | 2, 0]).unwrap(),
            vec![2, 3]
        );
        let link = [1 << 27, 0, (1 << 12) | 2, 0];
        let mut states = initial_gate_states(&data).unwrap();
        assert_eq!(states, vec![u32::MAX]);
        assert!(gate_link_allows(&data, link, Some(&states)).unwrap());
        set_gate_state(&data, &mut states, 3, false).unwrap();
        assert!(!gate_link_allows(&data, link, Some(&states)).unwrap());
        set_gate_state(&data, &mut states, 3, true).unwrap();
        assert!(gate_link_allows(&data, link, Some(&states)).unwrap());
        assert!(gate_link_allows(&data, link, None).unwrap());
        assert!(!gate_link_allows(&data, link, Some(&[1 << 2])).unwrap());
        assert!(gate_link_allows(&data, link, Some(&[(1 << 2) | (1 << 3)])).unwrap());
        assert!(gate_indices_for_link(&data, [1 << 27, 0, (2 << 12) | 2, 0]).is_err());
    }

    #[test]
    fn traversal_link_precheck_selects_original_direction_and_gate_branches() {
        let packed = [(1 << 29) | 17, 50, 0, 0];
        assert_eq!(
            traversal_link_precheck(
                &[],
                packed,
                LinkQueryDirection::Volume {
                    center: [52, 0, 0, 0],
                    tolerance: [2, 0, 0, 0],
                },
                None,
            )
            .unwrap(),
            TraversalLinkPrecheck::PackedDirection { faces_query: true }
        );
        assert_eq!(
            traversal_link_precheck(
                &[],
                packed,
                LinkQueryDirection::Point {
                    axis_sides: [true, false, false, false],
                },
                None,
            )
            .unwrap(),
            TraversalLinkPrecheck::PackedDirection { faces_query: true }
        );

        let mut bytes = vec![0u8; 0x104];
        bytes[0x68..0x6c].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x74..0x78].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x78..0x7c].copy_from_slice(&0x100u32.to_le_bytes());
        bytes[0x100..0x104].copy_from_slice(&0u32.to_le_bytes());
        let gated = [1 << 27, 0, 1, 0];
        assert_eq!(
            traversal_link_precheck(
                &bytes,
                gated,
                LinkQueryDirection::Volume {
                    center: [0; 4],
                    tolerance: [0; 4],
                },
                Some(&[0]),
            )
            .unwrap(),
            TraversalLinkPrecheck::Gates { open: false }
        );
        assert!(traversal_link_precheck(
            &bytes,
            gated,
            LinkQueryDirection::Point {
                axis_sides: [false; 4],
            },
            Some(&[1]),
        )
        .unwrap()
        .allows_traversal());
    }

    #[test]
    fn ready_same_spatial_cell_target_matches_original_link_loop_condition() {
        let volume = LinkQueryDirection::Volume {
            center: [53, 0, 0, 0],
            tolerance: [2, 0, 0, 0],
        };
        let positive = [(1 << 29) | 17, 50 | (71 << 16), 0, 0];
        assert_eq!(
            ready_same_spatial_cell_target(positive, 17, volume, 0x8005),
            Some(ReadyCellTarget {
                cell_index: 71,
                worklist_entry: 5,
            })
        );
        assert_eq!(
            ready_same_spatial_cell_target(positive, 17, volume, 0xc005),
            None
        );
        assert_eq!(
            ready_same_spatial_cell_target(positive, 17, volume, 0x4005),
            None
        );
        assert_eq!(
            ready_same_spatial_cell_target(positive, 18, volume, 0x8005),
            None
        );

        let facing = LinkQueryDirection::Point {
            axis_sides: [true, false, false, false],
        };
        assert_eq!(
            ready_same_spatial_cell_target(positive, 17, facing, 0x8005),
            None
        );
        assert_eq!(
            ready_same_spatial_cell_target(
                positive,
                17,
                LinkQueryDirection::Point {
                    axis_sides: [false, false, false, false],
                },
                0x8005
            ),
            Some(ReadyCellTarget {
                cell_index: 71,
                worklist_entry: 5,
            })
        );

        let gated = [(1 << 27) | (1 << 29) | 17, 0 | (71 << 16), 0, 0];
        assert_eq!(
            ready_same_spatial_cell_target(gated, 17, facing, 0x8005),
            Some(ReadyCellTarget {
                cell_index: 71,
                worklist_entry: 5,
            })
        );
        assert_eq!(
            ready_same_spatial_cell_target([(1 << 28) | 17, 71 << 16, 0, 0], 17, facing, 0x8005),
            None
        );
    }

    #[test]
    fn quantizes_direct_tome_link_query_with_record_padding() {
        let mut bytes = vec![0u8; 0x80];
        for (index, value) in [0.0f32, 0.0, 0.0, 65535.0, 65535.0, 65535.0]
            .iter()
            .enumerate()
        {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[0x30..0x34].copy_from_slice(&1.0f32.to_le_bytes());
        let (center, tolerance) =
            direct_link_query_quantization(&bytes, 0, [32768.0, 100.0, 0.0]).unwrap();
        assert_eq!(center, [32767, 99, 0, -1]);
        assert_eq!(tolerance, [2, 2, 2, 1]);
    }

    #[test]
    fn expands_spatial_record_query_bounds_by_authored_padding_and_context_offset() {
        let mut bytes = vec![0u8; 0x90];
        let record = 0x10usize;
        for (axis, value) in [0.0f32, 1.0, 2.0, 10.0, 11.0, 12.0].iter().enumerate() {
            let at = record + axis * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[record + 0x30..record + 0x34].copy_from_slice(&0.5f32.to_le_bytes());
        assert_eq!(
            spatial_record_query_bounds(&bytes, record as u32, [0.25, 0.5, 0.0]).unwrap(),
            [[-0.75, 0.0, 1.5, 1.0], [10.75, 12.0, 12.5, 1.0]]
        );
        assert!(spatial_record_query_bounds(&bytes, record as u32, [f32::NAN, 0.0, 0.0]).is_err());
    }

    #[test]
    fn assembles_record_local_query_bounds_quantization_and_active_planes() {
        let mut bytes = vec![0u8; 0x90];
        let record = 0x10usize;
        for (axis, value) in [0.0f32, 0.0, 0.0, 10.0, 10.0, 10.0].iter().enumerate() {
            let at = record + axis * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[record + 0x30..record + 0x34].copy_from_slice(&0.5f32.to_le_bytes());
        let planes = [
            [1.0, 0.0, 0.0, -11.0],
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, -9.0],
        ];
        let setup = setup_spatial_record_query(
            &bytes,
            record as u32,
            [5.0, 6.0, 7.0],
            [0.25, 0.5, 0.0],
            &planes,
        )
        .unwrap();
        assert_eq!(
            setup.bounds,
            [[-0.75, -1.0, -0.5, 1.0], [10.75, 11.0, 10.5, 1.0]]
        );
        assert_eq!(setup.link_center, [32767, 39320, 45874, -1]);
        assert_eq!(setup.active_planes, [0, 1, 2]);
        assert_eq!(
            setup.link_tolerance,
            direct_link_query_quantization(&bytes, record as u32, [5.0, 6.0, 7.0])
                .unwrap()
                .1
        );
    }

    #[test]
    fn expands_packed_portal_bounds_from_original_record() {
        let mut bytes = vec![0u8; 0x80];
        for (index, value) in [0.0f32, 0.0, 0.0, 65535.0, 65535.0, 65535.0]
            .iter()
            .enumerate()
        {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[0x30..0x34].copy_from_slice(&1.0f32.to_le_bytes());
        let geometry =
            packed_link_geometry([(2 << 30) | 17, 0x1234, 0x2000_8000, 0x4000_a000]).unwrap();
        assert_eq!(
            packed_link_query_bounds(&bytes, 0, geometry).unwrap(),
            [[8191.0, 16383.0, 4659.0], [32769.0, 40961.0, 4661.0]]
        );
    }

    #[test]
    fn clips_box_against_original_query_plane_with_rcpps() {
        let query = [[-10.0, -10.0, -10.0, 1.0], [10.0, 10.0, 10.0, 1.0]];
        let input = [[0.0, 0.0, 0.0, 1.0], [2.0, 2.0, 2.0, 1.0]];
        let mut planes = [[0.0; 4]; 3];
        planes[2] = [1.0, 0.0, 0.0, -1.0];
        assert_eq!(
            clip_box_to_active_query_planes(query, input, &planes, &[2]).unwrap(),
            [[1.000244140625, 0.0, 0.0, 1.0], [2.0, 2.0, 2.0, 1.0]]
        );
        planes[2] = [-1.0, 0.0, 0.0, 1.0];
        assert_eq!(
            clip_box_to_active_query_planes(query, input, &planes, &[2]).unwrap(),
            [[0.0, 0.0, 0.0, 1.0], [0.999755859375, 2.0, 2.0, 1.0]]
        );
        planes[2] = [1.0, 0.0, 0.0, -3.0];
        assert_eq!(
            clip_box_to_active_query_planes(query, input, &planes, &[2]).unwrap(),
            input
        );
    }

    #[test]
    fn box_clipping_matches_original_executable_cases() {
        for (case, line) in include_str!("../analysis/original_box_clip_cases.txt")
            .lines()
            .enumerate()
        {
            let values = line
                .split_whitespace()
                .map(|part| part.parse::<f32>().unwrap())
                .collect::<Vec<_>>();
            let count = values[0] as usize;
            assert_eq!(values.len(), 25 + count * 4, "case {case}");
            let box_at = |start: usize| -> [[f32; 4]; 2] {
                [
                    values[start..start + 4].try_into().unwrap(),
                    values[start + 4..start + 8].try_into().unwrap(),
                ]
            };
            let mut planes = vec![[0.0; 4]; 2];
            for offset in (17..17 + count * 4).step_by(4) {
                planes.push(values[offset..offset + 4].try_into().unwrap());
            }
            let actual = clip_box_to_active_query_planes(
                box_at(1),
                box_at(9),
                &planes,
                &(2..2 + count).collect::<Vec<_>>(),
            )
            .unwrap();
            let expected = box_at(17 + count * 4);
            for side in 0..2 {
                for axis in 0..4 {
                    assert!(
                        (actual[side][axis] - expected[side][axis]).abs() <= 0.005,
                        "case {case}, bound {side}, axis {axis}: {} != {}",
                        actual[side][axis],
                        expected[side][axis]
                    );
                }
            }
        }
    }

    #[test]
    fn std_floor_matches_installed_renderer_math() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_floor_cases.txt").lines() {
            let words: Vec<_> = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            assert_eq!(words.len(), 2);
            let value = f32::from_bits(words[0]);
            assert!(!value.is_nan());
            assert_eq!(value.floor().to_bits(), words[1], "input {:08x}", words[0]);
            checked += 1;
        }
        assert_eq!(checked, 9981);
    }

    #[test]
    fn scaled_render_viewport_matches_installed_executable_cases() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_viewport_scale_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 10);
            let rect = std::array::from_fn(|i| fields[i].parse().unwrap());
            let scale = std::array::from_fn(|i| fields[4 + i].parse().unwrap());
            let expected: [i32; 4] = std::array::from_fn(|i| fields[6 + i].parse().unwrap());
            assert_eq!(scaled_render_viewport(rect, scale).unwrap(), expected);
            checked += 1;
        }
        assert_eq!(checked, 37);
        assert_eq!(
            render_viewport_dimensions([0, 0, 1919, 1079]).unwrap(),
            [1920, 1080]
        );
        assert!(scaled_render_viewport([0, 0, 9, 9], [f32::NAN, 1.0]).is_err());
        assert!(scaled_render_viewport([0, 0, 9, 9], [f32::MAX, 1.0]).is_err());
        assert!(scaled_render_viewport([i32::MIN, 0, i32::MAX, 9], [1.0; 2]).is_err());
        assert!(render_viewport_dimensions([0, 0, -1, 9]).is_err());
    }

    #[test]
    fn spatial_query_and_bounds_match_installed_executable_cases() {
        let mut checked = [0; 2];
        for (case, line) in include_str!("../analysis/original_spatial_query_cases.txt")
            .lines()
            .enumerate()
        {
            let mut fields = line.split_whitespace();
            let kind = fields.next().unwrap();
            let values: Vec<u64> = fields.map(|v| v.parse().unwrap()).collect();
            let float = |i: usize| f32::from_bits(values[i] as u32);
            match kind {
                "spatial" => {
                    assert_eq!(values.len(), 29);
                    let rows = std::array::from_fn(|row| {
                        std::array::from_fn(|column| float(7 + row * 4 + column))
                    });
                    let viewport =
                        render_query_job_tile(values[1] as u32, values[0] as u32).unwrap();
                    let state = RenderQueryState {
                        render_rows: rows,
                        cropped_rows: crop_query_rows(rows, viewport).unwrap(),
                        camera_point: [0.0; 3],
                        camera_imprecision: std::array::from_fn(|i| float(4 + i)),
                        winding_flip: false,
                        viewport,
                        maximum_depth: 0.0,
                        depth_grid: None,
                    };
                    let bounds = std::array::from_fn(|side| {
                        std::array::from_fn(|axis| float(23 + side * 3 + axis))
                    });
                    assert_eq!(
                        render_query_aabb_visible_without_depth(
                            (values[2] != 0).then_some(&state),
                            bounds,
                        )
                        .unwrap(),
                        values[3] != 0,
                        "spatial case {case}"
                    );
                    checked[0] += 1;
                }
                "bounds" => {
                    let count = values[0] as usize;
                    assert_eq!(values.len(), 42 + count);
                    let box_at = |first| {
                        std::array::from_fn(|side| {
                            std::array::from_fn(|axis| float(first + side * 4 + axis))
                        })
                    };
                    let planes: [[f32; 4]; 6] = std::array::from_fn(|plane| {
                        std::array::from_fn(|axis| float(18 + plane * 4 + axis))
                    });
                    let active: Vec<usize> = values[42..].iter().map(|&v| v as usize).collect();
                    assert_eq!(
                        query_bounds_accepts_aabb(box_at(2), box_at(10), &planes, &active).unwrap(),
                        values[1] != 0,
                        "bounds case {case}"
                    );
                    checked[1] += 1;
                }
                _ => panic!("unknown original spatial fixture kind"),
            }
        }
        assert_eq!(checked, [576, 70]);
    }

    #[test]
    fn spatial_query_uses_native_depth_and_rejects_bad_plane_indices() {
        let rows = std::array::from_fn(|row| {
            std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
        });
        let mut state = RenderQueryState {
            render_rows: rows,
            cropped_rows: rows,
            camera_point: [0.0; 3],
            camera_imprecision: [0.0; 3],
            winding_flip: false,
            viewport: [0, 0, 64, 64],
            maximum_depth: 0.0,
            depth_grid: None,
        };
        let bounds = [[0.0; 3], [1.0; 3]];
        assert!(render_query_aabb_visible_without_depth(Some(&state), bounds).unwrap());
        state.camera_imprecision = [0.6; 3];
        state.depth_grid = Some(Box::new([0.0; 4096]));
        // Native nearest-Z <= 0 accepts, even with an empty depth grid.
        assert!(render_query_aabb_visible_without_depth(Some(&state), bounds).unwrap());
        state.camera_imprecision = [0.0; 3];
        let in_front = [[-0.125, -0.125, 0.25], [0.125, 0.125, 0.75]];
        assert!(!render_query_aabb_visible_without_depth(Some(&state), in_front).unwrap());
        state.depth_grid = Some(Box::new([1.0; 4096]));
        assert!(render_query_aabb_visible_without_depth(Some(&state), in_front).unwrap());
        assert!(query_bounds_accepts_aabb([[0.0; 4]; 2], [[0.0; 4]; 2], &[], &[0]).is_err());
        assert!(render_query_aabb_visible_without_depth(None, [[f32::NAN; 3]; 2]).unwrap());
    }

    #[test]
    fn projected_area_metric_uses_original_unit_square_clip() {
        assert_eq!(projected_object_area_metric([-1.0, -1.0, 1.0, 1.0]), 1.0);
        assert_eq!(projected_object_area_metric([0.0, 0.0, 1.0, 1.0]), 0.25);
        assert_eq!(projected_object_area_metric([-2.0, -2.0, 0.0, 0.0]), 0.25);
        assert_eq!(projected_object_area_metric([2.0, 2.0, 3.0, 3.0]), 0.0);
    }

    #[test]
    fn projected_area_cutoff_is_25_viewport_pixels() {
        let cutoff = minimum_projected_area_metric(1920, 1080).unwrap();
        assert!((cutoff * 1920.0 * 1080.0 - 25.0).abs() < 0.0001);
        assert!(minimum_projected_area_metric(0, 1080).is_err());
        assert!(minimum_projected_area_metric(1920, i32::MAX).is_err());
    }

    #[test]
    fn projected_visible_output_applies_cutoff_before_portal_and_deduplicates() {
        let mut portal = PortalTileMask::new([0, 0, 8, 16]).unwrap();
        portal.bits.fill(u32::MAX);
        let cutoff = minimum_projected_area_metric(64, 64).unwrap();
        let mut output = VisibleObjectIndexOutput::new(4, 4);

        let visible_metric = output
            .push_if_projected_object_visible(
                2,
                [0.0, 0.0, 0.2, 0.2],
                cutoff,
                [0, 0, 8, 16],
                &portal,
                None,
            )
            .unwrap()
            .unwrap();
        assert!((visible_metric - 0.01).abs() < 0.000_001);
        assert!(output.contains(2).unwrap());
        assert!(!output.contains(1).unwrap());
        assert_eq!(
            output
                .push_if_projected_object_visible(
                    1,
                    [0.0, 0.0, 0.05, 0.05],
                    cutoff,
                    [0, 0, 8, 16],
                    &portal,
                    None,
                )
                .unwrap(),
            None
        );
        assert_eq!(
            output
                .push_if_projected_object_visible(
                    2,
                    [f32::NAN; 4],
                    cutoff,
                    [0, 0, 8, 16],
                    &portal,
                    None,
                )
                .unwrap(),
            None
        );
        assert_eq!(output.indices(), &[2]);
        assert_eq!(output.area_metrics().len(), output.indices().len());
        assert!((output.area_metrics()[0].unwrap() - 0.01).abs() < 0.000_001);
        assert_eq!(output.count(), 1);
    }

    #[test]
    fn cell_candidate_pipeline_skips_accepted_and_distance_rejected_bounds() {
        let mut bytes = vec![0u8; 0xa0];
        bytes[0x40..0x44].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x44..0x48].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x54..0x58].copy_from_slice(&(1u32 | (1 << 5)).to_le_bytes());
        bytes[0x58..0x5c].copy_from_slice(&0x60u32.to_le_bytes());
        bytes[0x5c..0x60].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x60] = 0b10; // candidate index 0, run length 1
        for (index, value) in [-0.5f32, -0.5, 0.0, 0.5, 0.5, 0.0].into_iter().enumerate() {
            bytes[0x80 + index * 4..0x84 + index * 4].copy_from_slice(&value.to_le_bytes());
        }
        let cells = CellTree {
            bounds: Vec::new(),
            children: Vec::new(),
            axes: Vec::new(),
            planes: Vec::new(),
            leaf_ranks: Vec::new(),
            cell_codes: Vec::new(),
            cell_ids: vec![1],
            cell_visibility_counts: vec![0],
            direct_cell_ids_available: true,
            cell_bounds: vec![[[-1.0; 3], [1.0; 3]]],
            cell_link_ranges: vec![(0, 0)],
            cell_candidate_ranges: vec![(0, 1)],
            links_at: 0,
            candidates_at: 0x60,
            candidate_index_bits: 1,
            candidate_run_bits: 1,
            candidate_record_count: 1,
            object_count: 1,
            code_width: 1,
            portal_nodes_at: None,
            portal_planes_at: None,
        };
        let mut portal = PortalTileMask::new([0, 0, 8, 16]).unwrap();
        portal.bits.fill(u32::MAX);
        let query = CellObjectVisibilityQuery {
            record_query_bounds: [[-1.0, -1.0, -1.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
            query_planes: &[],
            active_planes: &[],
            projection_rows: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            camera_translation_imprecision: 0.0,
            object_distance_point: [0.0; 3],
            object_distance_scale_squared: [1.0; 4],
            minimum_area_metric: 0.0,
            viewport_tile_bounds: [0, 0, 8, 16],
            portal: &portal,
            depth_grid: None,
        };
        let mut output = VisibleObjectIndexOutput::new(1, 1);
        cells
            .append_visible_object_candidates(&bytes, 0, &query, &mut output)
            .unwrap();
        assert_eq!(output.indices(), &[0]);
        assert!((output.area_metrics()[0].unwrap() - 0.25).abs() < 1e-6);

        bytes[0x80..0x84].copy_from_slice(&f32::NAN.to_le_bytes());
        cells
            .append_visible_object_candidates(&bytes, 0, &query, &mut output)
            .unwrap();
        assert_eq!(output.count(), 1);

        // Distance rejection precedes the now-invalid geometry bounds, and
        // marks a tested bit without appending a visible object.
        bytes.resize(0xc0, 0);
        bytes[0x48..0x4c].copy_from_slice(&0xa0u32.to_le_bytes());
        let mut rejected = VisibleObjectIndexOutput::new(1, 1);
        cells
            .append_visible_object_candidates(&bytes, 0, &query, &mut rejected)
            .unwrap();
        assert!(rejected.indices().is_empty());
        assert!(rejected.area_metrics().is_empty());
        assert_eq!(rejected.count(), 0);
        assert!(rejected.contains(0).unwrap());
        assert_eq!(rejected.distance_rejected_objects, 1);

        // A later cell must skip even the distance-table read for this index.
        bytes[0x48..0x4c].copy_from_slice(&u32::MAX.to_le_bytes());
        cells
            .append_visible_object_candidates(&bytes, 0, &query, &mut rejected)
            .unwrap();
        assert_eq!(rejected.distance_rejected_objects, 1);
    }

    #[test]
    fn object_distance_matches_installed_instruction_cases() {
        let mut count = 0;
        for (case, line) in include_str!("../analysis/original_object_distance_cases.txt")
            .lines()
            .enumerate()
        {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|word| word.parse().unwrap())
                .collect();
            assert_eq!(words.len(), 17, "distance fixture {case}");
            let point = std::array::from_fn(|axis| f32::from_bits(words[axis]));
            let record = std::array::from_fn(|row| {
                std::array::from_fn(|axis| f32::from_bits(words[4 + row * 4 + axis]))
            });
            let scale = std::array::from_fn(|axis| f32::from_bits(words[12 + axis]));
            assert_eq!(
                object_distance_rejection_mask(record, point, scale),
                words[16],
                "native distance case {case}"
            );
            count += 1;
        }
        assert_eq!(count, 822);
    }

    #[test]
    fn optional_object_distance_table_is_indexed_and_preserves_nan() {
        let mut bytes = vec![0u8; 0xa0];
        bytes[0x40..0x44].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(object_distance_bounds(&bytes, 0).unwrap(), None);
        bytes[0x48..0x4c].copy_from_slice(&0x60u32.to_le_bytes());
        let records = [
            [-1.0f32, -2.0, -3.0, 4.0, 1.0, 2.0, 3.0, 9.0],
            [-5.0, -6.0, -7.0, 25.0, 5.0, 6.0, 7.0, f32::NAN],
        ];
        for (index, record) in records.into_iter().enumerate() {
            for (field, value) in record.into_iter().enumerate() {
                let at = 0x60 + index * 32 + field * 4;
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        assert_eq!(
            object_distance_bounds(&bytes, 0).unwrap(),
            Some([[-1.0, -2.0, -3.0, 4.0], [1.0, 2.0, 3.0, 9.0]])
        );
        let second = object_distance_bounds(&bytes, 1).unwrap().unwrap();
        assert_eq!(second[0], [-5.0, -6.0, -7.0, 25.0]);
        assert!(second[1][3].is_nan());
        assert!(object_distance_bounds(&bytes, 2)
            .unwrap_err()
            .contains("out of range"));
        bytes.truncate(0x9f);
        assert!(object_distance_bounds(&bytes, 1)
            .unwrap_err()
            .contains("truncated"));
    }

    #[test]
    #[ignore = "requires local original-game EOTW Tome fixture"]
    fn authored_eotw_distance_tiers_match_installed_predicate() {
        let bytes = std::fs::read("artifacts/eotw_p_lowchaos.tome")
            .expect("missing local EOTW Tome fixture");
        for (point, expected) in [
            ([-43.0, 138.0, 48.0], [0, 15, 15]),
            ([-43.0, 138.0, 100.0], [15, 15, 0]),
        ] {
            let masks: Vec<u32> = (28684..=28686)
                .map(|index| {
                    object_distance_rejection_mask(
                        object_distance_bounds(&bytes, index).unwrap().unwrap(),
                        point,
                        [1.0; 4],
                    )
                })
                .collect();
            assert_eq!(masks, expected);
        }
        let record = object_distance_bounds(&bytes, 28684).unwrap().unwrap();
        assert_eq!(record[0][3], 0.0);
        assert_eq!(record[1][3], 225.0);
        let boundary = [-43.0, 138.0, record[1][2] + 15.0];
        let masks: Vec<u32> = (28684..=28686)
            .map(|index| {
                object_distance_rejection_mask(
                    object_distance_bounds(&bytes, index).unwrap().unwrap(),
                    boundary,
                    [1.0; 4],
                )
            })
            .collect();
        assert_eq!(masks, [15, 0, 15]);
    }

    #[test]
    fn viewport_bounds_pack_to_original_query_tile_bytes() {
        assert_eq!(query_viewport_tile_bounds([10, 5, 101, 51]), [1, 1, 13, 13]);
        assert_eq!(
            query_viewport_tile_bounds([0, 0, 1920, 1080]),
            [0, 0, 240, 14]
        );
    }

    #[test]
    fn projected_object_rectangle_maps_to_pixels_and_intersects_query_tiles() {
        assert_eq!(
            projected_object_pixel_rect([-0.5, -0.5, 0.5, 0.5], [2, 3, 7, 11]).unwrap(),
            [16, 16, 48, 44]
        );
        assert_eq!(
            projected_object_pixel_rect([-2.0, -2.0, -1.5, -1.5], [0, 0, 8, 16]).unwrap(),
            [0, 0, 0, 0]
        );
        assert!(projected_object_pixel_rect([0.0, 0.0, 1.0, 1.0], [4, 8, 3, 9]).is_err());
    }

    #[test]
    fn object_pixel_rectangles_match_installed_conversion_instructions() {
        let mut count = 0;
        for (case, line) in include_str!("../analysis/original_object_pixel_cases.txt")
            .lines()
            .enumerate()
        {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 12);
            let projected =
                std::array::from_fn(|axis| f32::from_bits(fields[axis].parse::<u32>().unwrap()));
            let tiles = std::array::from_fn(|axis| fields[4 + axis].parse::<u8>().unwrap());
            let expected = std::array::from_fn(|axis| fields[8 + axis].parse::<i32>().unwrap());
            assert_eq!(
                projected_object_pixel_rect(projected, tiles).unwrap(),
                expected,
                "installed object pixel case {case}"
            );
            count += 1;
        }
        assert_eq!(count, 3000);
    }

    #[test]
    fn thin_visible_objects_keep_pixel_coverage_under_small_screen_shifts() {
        let portal = PortalTileMask::full([0, 0, 8, 16]).unwrap();
        let cutoff = minimum_projected_area_metric(1280, 720).unwrap();
        for step in 0..120 {
            let x = -0.4 + step as f32 * 0.006;
            let projected = [x, -0.25, x + 0.0005, 0.25];
            let mut output = VisibleObjectIndexOutput::new(1, 1);
            assert!(
                output
                    .push_if_projected_object_visible(
                        0,
                        projected,
                        cutoff,
                        [0, 0, 8, 16],
                        &portal,
                        None
                    )
                    .unwrap()
                    .is_some(),
                "thin object lost pixel coverage at step {step}"
            );
        }
    }

    #[test]
    fn projected_portal_rectangle_uses_authored_viewport_pixel_conversion() {
        assert_eq!(
            projected_portal_pixel_rect([-1.0, -1.0, 1.0, 1.0], [0, 0, 64, 64]).unwrap(),
            Some([0, 0, 64, 64])
        );
        assert_eq!(
            projected_portal_pixel_rect([-0.5, -0.5, 0.99, 0.99], [0, 0, 64, 64]).unwrap(),
            Some([16, 16, 64, 64])
        );
        assert_eq!(
            projected_portal_pixel_rect([-2.0, -2.0, -1.5, -1.5], [0, 0, 64, 64]).unwrap(),
            None
        );
        assert_eq!(
            projected_portal_pixel_rect([-2.0, -2.0, 2.0, 2.0], [8, 4, 40, 28]).unwrap(),
            Some([8, 4, 40, 28])
        );
        assert!(projected_portal_pixel_rect([f32::NAN, 0.0, 1.0, 1.0], [0, 0, 64, 64]).is_err());
        assert!(projected_portal_pixel_rect([0.0, 0.0, 1.0, 1.0], [4, 8, 3, 9]).is_err());
    }

    #[test]
    fn projected_portal_box_fallback_reduces_both_homogeneous_faces() {
        let corners = [
            [-1.0, -0.5, 0.0, 2.0],
            [1.0, -0.5, 0.0, 2.0],
            [1.0, 0.5, 0.0, 2.0],
            [-1.0, 0.5, 0.0, 2.0],
            [-3.0, -2.0, 0.0, 4.0],
            [3.0, -2.0, 0.0, 4.0],
            [3.0, 2.0, 0.0, 4.0],
            [-3.0, 2.0, 0.0, 4.0],
        ];
        let bounds = projected_portal_box_fallback_bounds(corners).unwrap();
        for (actual, expected) in bounds.into_iter().zip([-0.75, -0.5, 0.75, 0.5]) {
            assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
        }

        let zero_w = [
            [0.25, -0.25, 0.0, 0.0],
            [0.5, -0.5, 0.0, 1.0],
            [0.75, 0.5, 0.0, 1.0],
            [-0.5, 0.25, 0.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let bounds = projected_portal_box_fallback_bounds(zero_w).unwrap();
        for (actual, expected) in bounds.into_iter().zip([-0.5, -0.5, 0.75, 0.5]) {
            assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
        }
        let invalid = [[f32::NAN, 0.0, 0.0, 1.0]; 8];
        assert!(projected_portal_box_fallback_bounds(invalid).is_err());
    }

    #[test]
    fn projected_portal_box_uses_active_plane_crossing_clip_path() {
        let corners = [
            [-2.0, -0.5, 0.0, -1.0],
            [2.0, -0.5, 0.0, -1.0],
            [2.0, 0.5, 0.0, -1.0],
            [-2.0, 0.5, 0.0, -1.0],
            [-2.0, -0.5, 0.0, 1.0],
            [2.0, -0.5, 0.0, 1.0],
            [2.0, 0.5, 0.0, 1.0],
            [-2.0, 0.5, 0.0, 1.0],
        ];
        let (bounds, used_crossing_path) =
            projected_portal_box_bounds(corners, &[0]).unwrap().unwrap();
        assert!(used_crossing_path);
        for (actual, expected) in bounds.into_iter().zip([-1.0, -1.0, 1.0, 1.0]) {
            assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
        }
        assert_eq!(
            projected_portal_box_pixel_rect(corners, &[0], [0, 0, 64, 64]).unwrap(),
            (Some([0, 0, 64, 64]), true)
        );

        let partially_visible = [
            [0.0, 0.0, 0.0, -1.0],
            [2.0, 0.0, 0.0, -1.0],
            [2.0, 0.5, 0.0, -1.0],
            [0.0, 0.5, 0.0, -1.0],
            [0.0, 0.0, 0.0, 1.0],
            [2.0, 0.0, 0.0, 1.0],
            [2.0, 0.5, 0.0, 1.0],
            [0.0, 0.5, 0.0, 1.0],
        ];
        let bounds = projected_portal_box_crossing_bounds(partially_visible)
            .unwrap()
            .unwrap();
        for (actual, expected) in bounds.into_iter().zip([0.0, 0.0, 1.0, 1.0]) {
            assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
        }

        let outside = partially_visible.map(|mut corner| {
            corner[0] = 2.5;
            corner
        });
        assert_eq!(projected_portal_box_crossing_bounds(outside).unwrap(), None);
        assert_eq!(
            projected_portal_box_pixel_rect(outside, &[0], [0, 0, 64, 64]).unwrap(),
            (None, true)
        );

        let (_, used_crossing_path) = projected_portal_box_bounds(corners, &[1, 0])
            .unwrap()
            .unwrap();
        assert!(!used_crossing_path);

        let in_front = corners.map(|mut corner| {
            corner[3] = 2.0;
            corner
        });
        let (_, used_crossing_path) = projected_portal_box_bounds(in_front, &[0])
            .unwrap()
            .unwrap();
        assert!(!used_crossing_path);
    }

    #[test]
    fn object_bounds_expand_by_original_camera_translation_imprecision() {
        let bounds = [[-2.0, 3.0, 8.0], [5.0, 9.0, 14.0]];
        let expanded = expand_object_bounds_for_camera_imprecision(bounds, 0.6);
        for axis in 0..3 {
            assert!((expanded[0][axis] - (bounds[0][axis] - 0.6)).abs() < 1e-6);
            assert!((expanded[1][axis] - (bounds[1][axis] + 0.6)).abs() < 1e-6);
        }
        assert_eq!(
            expand_object_bounds_for_camera_imprecision(bounds, 0.0),
            bounds
        );
    }

    #[test]
    fn projects_all_object_box_corners_when_clip_w_is_positive() {
        let bounds = [[-2.0, -1.0, 2.0], [3.0, 4.0, 5.0]];
        let matrix = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        assert_eq!(
            project_object_bounds_clipped_w(bounds, matrix).unwrap(),
            Some([-2.0, -1.0, 3.0, 4.0])
        );

        let perspective = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
        ];
        assert_eq!(
            project_object_bounds_clipped_w(bounds, perspective).unwrap(),
            Some([-1.0, -0.5, 1.5, 2.0])
        );
    }

    #[test]
    fn clips_object_box_crossing_w_zero_and_clamps_unbounded_projection() {
        let matrix = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
        ];
        let rectangle =
            project_object_bounds_clipped_w([[-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]], matrix)
                .unwrap()
                .unwrap();
        assert_eq!(
            rectangle,
            [
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
                f32::INFINITY,
                f32::INFINITY
            ]
        );
        assert_eq!(
            projected_object_pixel_rect(rectangle, [0, 0, 8, 16]).unwrap(),
            [0, 0, 64, 64]
        );

        assert_eq!(
            project_object_bounds_clipped_w([[-1.0, -1.0, -2.0], [1.0, 1.0, -1.0]], matrix)
                .unwrap(),
            None
        );
    }

    #[test]
    fn w_zero_crossing_keeps_finite_axis_limit_when_its_numerator_is_zero() {
        let matrix = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
        ];
        let rectangle =
            project_object_bounds_clipped_w([[0.0, -1.0, -1.0], [0.0, 1.0, 1.0]], matrix)
                .unwrap()
                .unwrap();
        assert_eq!(rectangle[0], 0.0);
        assert_eq!(rectangle[2], 0.0);
        assert_eq!(rectangle[1], f32::NEG_INFINITY);
        assert_eq!(rectangle[3], f32::INFINITY);
    }

    #[test]
    fn visible_object_output_deduplicates_and_counts_beyond_buffer_capacity() {
        let mut output = VisibleObjectIndexOutput::new(70, 2);
        assert!(output.push_unique(31).unwrap());
        assert!(!output.push_unique(31).unwrap());
        assert!(output.push_unique(32).unwrap());
        assert!(output.push_unique(69).unwrap());
        assert!(!output.push_unique(69).unwrap());
        assert_eq!(output.indices(), [31, 32]);
        assert_eq!(output.count(), 3);
        assert!(output.push_unique(70).is_err());
    }

    #[test]
    fn projected_rectangle_output_selects_original_portal_or_depth_branch() {
        let pixels = [0, 0, 8, 4];
        let mut portal = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        portal.bits[0] = 1;
        let mut output = VisibleObjectIndexOutput::new(4, 4);

        assert!(output
            .push_if_rectangle_visible(1, &portal, pixels, None)
            .unwrap());
        assert!(!output
            .push_if_rectangle_visible(1, &portal, pixels, None)
            .unwrap());

        let mut depths = vec![0.0; 64 * 64];
        depths[0] = 0.75;
        assert!(!output
            .push_if_rectangle_visible(2, &portal, pixels, Some((&depths, 0.8)))
            .unwrap());
        assert!(output
            .push_if_rectangle_visible(2, &portal, pixels, Some((&depths, 0.75)))
            .unwrap());
        assert_eq!(output.indices(), [1, 2]);
        assert_eq!(output.count(), 2);
    }

    fn visible_cell_test_tree(first: u32, count: u32) -> CellTree {
        CellTree {
            bounds: Vec::new(),
            children: Vec::new(),
            axes: Vec::new(),
            planes: Vec::new(),
            leaf_ranks: Vec::new(),
            cell_codes: Vec::new(),
            cell_ids: vec![first],
            cell_visibility_counts: vec![count],
            direct_cell_ids_available: true,
            cell_bounds: Vec::new(),
            cell_link_ranges: Vec::new(),
            cell_candidate_ranges: Vec::new(),
            links_at: 0,
            candidates_at: 0,
            candidate_index_bits: 0,
            candidate_run_bits: 0,
            candidate_record_count: 0,
            object_count: 0,
            code_width: 0,
            portal_nodes_at: None,
            portal_planes_at: None,
        }
    }

    #[test]
    fn sphere_portal_geometry_matches_installed_instructions() {
        let mut checked = 0;
        for (case, line) in include_str!("../analysis/original_light_portal_cases.txt")
            .lines()
            .enumerate()
        {
            let values: Vec<u32> = line
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            let metrics = values[0];
            let link = values[7..11].try_into().unwrap();
            let sphere = std::array::from_fn(|i| f32::from_bits(values[11 + i]));
            let count = values[15] as usize;
            assert_eq!(values.len(), 16 + count + 3);
            let mut bytes = vec![0u8; 0x100 + count * 4];
            for (axis, &bits) in values[1..7].iter().enumerate() {
                bytes[0x14 + axis * 4..0x18 + axis * 4].copy_from_slice(&bits.to_le_bytes());
            }
            bytes[0x70..0x74].copy_from_slice(&0x100u32.to_le_bytes());
            bytes[0x74..0x78].copy_from_slice(&((count / 3) as u32).to_le_bytes());
            for (index, &bits) in values[16..16 + count].iter().enumerate() {
                bytes[0x100 + index * 4..0x104 + index * 4].copy_from_slice(&bits.to_le_bytes());
            }
            let mut inside = 1.25f32;
            let mut outside = 2.5f32;
            let accepted = sphere_intersects_portal(
                &bytes,
                link,
                sphere,
                (metrics & 1 != 0).then_some(&mut inside),
                (metrics & 2 != 0).then_some(&mut outside),
            )
            .unwrap();
            assert_eq!(
                [u32::from(accepted), inside.to_bits(), outside.to_bits()],
                <[u32; 3]>::try_from(&values[16 + count..]).unwrap(),
                "sphere portal case {case}: link={link:?}, sphere={sphere:?}, metrics={metrics}"
            );
            checked += 1;
        }
        assert_eq!(checked, 16704);
    }

    #[test]
    fn triangle_distance_matches_installed_instructions() {
        let mut checked = 0;
        for (case, line) in include_str!("../analysis/original_light_triangle_cases.txt")
            .lines()
            .enumerate()
        {
            let values: Vec<u32> = line
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            assert_eq!(values.len(), 13);
            let point = std::array::from_fn(|axis| f32::from_bits(values[axis]));
            let triangle = std::array::from_fn(|vertex| {
                std::array::from_fn(|axis| f32::from_bits(values[3 + vertex * 3 + axis]))
            });
            assert_eq!(
                point_triangle_distance_squared(point, triangle).to_bits(),
                values[12],
                "triangle case {case}: point={point:?}, triangle={triangle:?}"
            );
            checked += 1;
        }
        assert_eq!(checked, 4728);
    }

    #[test]
    fn authored_id_search_matches_installed_instructions() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_authored_id_search_cases.txt").lines() {
            let mut fields = line.split_whitespace();
            let target: u32 = fields.next().unwrap().parse().unwrap();
            let present = fields.next().unwrap() == "1";
            let count: usize = fields.next().unwrap().parse().unwrap();
            let ids: Vec<u32> = fields
                .by_ref()
                .take(count)
                .map(|v| v.parse().unwrap())
                .collect();
            let expected: i32 = fields.next().unwrap().parse().unwrap();
            assert!(fields.next().is_none());
            let input = if present { ids.as_slice() } else { &[] };
            assert_eq!(
                sorted_authored_id_index(input, target)
                    .map(|v| v as i32)
                    .unwrap_or(-1),
                expected,
                "target {target}, present={present}, ids={ids:?}"
            );
            checked += 1;
        }
        assert_eq!(checked, 334);
    }

    #[test]
    fn visible_cell_output_matches_installed_instructions() {
        let mut checked = 0;
        for (case, line) in include_str!("../analysis/original_visible_cell_cases.txt")
            .lines()
            .enumerate()
        {
            let values: Vec<u32> = line
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            let [index_bits, run_bits, domain, capacity, first, count, remap, base, requested, seed_count]: [u32; 10] = values[..10].try_into().unwrap();
            let mut at = 10;
            let seeds = &values[at..at + seed_count as usize];
            at += seed_count as usize;
            let rows = values[at] as usize;
            at += 1;
            let pairs = &values[at..at + rows * 2];
            at += rows * 2;
            let expected_count = values[at] as usize;
            let expected_length = values[at + 1] as usize;
            at += 2;
            let expected_indices = &values[at..at + expected_length];
            at += expected_length;
            let mask_length = values[at] as usize;
            at += 1;
            let expected_mask = &values[at..at + mask_length];
            assert_eq!(at + mask_length, values.len(), "case {case}");
            let width = (index_bits + run_bits) as usize;
            let mut bytes = vec![0u8; 0x100 + (rows * width).div_ceil(32) * 4 + 4];
            bytes[0x54..0x58].copy_from_slice(&(index_bits << 10 | run_bits << 15).to_le_bytes());
            bytes[0x60..0x64].copy_from_slice(&0x100u32.to_le_bytes());
            for (row, pair) in pairs.chunks_exact(2).enumerate() {
                let packed = pair[0] as u64 | ((pair[1] as u64) << index_bits);
                for bit in 0..width {
                    let target = row * width + bit;
                    bytes[0x100 + target / 8] |= (((packed >> bit) & 1) as u8) << (target % 8);
                }
            }
            let mut cells = visible_cell_test_tree(first, count);
            let mut output = VisibleCellIndexOutput::new(domain as usize, capacity as usize);
            let remap = (remap != 0).then_some(base);
            for &seed in seeds {
                cells.cell_ids[0] = seed;
                cells.cell_visibility_counts[0] = 0;
                cells
                    .append_visible_cell_indices(
                        &bytes,
                        0,
                        remap,
                        (requested != 0).then_some(&mut output),
                    )
                    .unwrap();
            }
            cells.cell_ids[0] = first;
            cells.cell_visibility_counts[0] = count;
            cells
                .append_visible_cell_indices(
                    &bytes,
                    0,
                    remap,
                    (requested != 0).then_some(&mut output),
                )
                .unwrap();
            assert_eq!(
                output.count(),
                expected_count,
                "native cell count case {case}"
            );
            assert_eq!(
                output.indices(),
                expected_indices,
                "native cell indices case {case}"
            );
            assert_eq!(output.seen, expected_mask, "native cell bitset case {case}");
            checked += 1;
        }
        assert_eq!(checked, 756);
    }

    #[test]
    fn point_cell_ids_match_installed_instructions() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_visible_cell_cases.point-id.txt").lines() {
            let values: Vec<u32> = line
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            let mut cells = visible_cell_test_tree(0, 0);
            cells.direct_cell_ids_available = values[0] >> 16 != 0;
            cells.cell_ids = values[2..4].to_vec();
            assert_eq!(cells.point_cell_id(values[1] as usize).unwrap(), values[4]);
            checked += 1;
        }
        assert_eq!(checked, 13);
    }

    #[test]
    fn visible_cell_mask_and_invalid_stream_boundaries() {
        assert_eq!(
            cell_visibility_mask(65, &[0, 31, 32, 64, 31]).unwrap(),
            [0x80000001, 1, 1]
        );
        assert!(cell_visibility_mask(0, &[]).unwrap().is_empty());
        assert!(cell_visibility_mask(65, &[65]).is_err());
        assert!(cell_visibility_mask(usize::MAX, &[]).is_err());
        let cells = visible_cell_test_tree(u32::MAX, 100);
        cells
            .append_visible_cell_indices(&[], usize::MAX, None, None)
            .unwrap();
        let mut output = VisibleCellIndexOutput::new(1, 0);
        assert!(cells
            .append_visible_cell_indices(&[], 0, None, Some(&mut output))
            .is_err());
        assert!(output.push_unique(1).is_err());
        output.push_unique(0).unwrap();
        assert_eq!(output.count(), 1);
        assert!(output.indices().is_empty());
        assert_eq!(cell_visibility_mask(1, output.indices()).unwrap(), [0]);
        let cells = visible_cell_test_tree(0, 1);
        let mut bytes = vec![0; 0x64];
        assert!(cells
            .append_visible_cell_indices(&bytes, 0, None, Some(&mut output))
            .unwrap_err()
            .contains("zero-width"));
        bytes[0x54..0x58].copy_from_slice(&(1u32 << 15).to_le_bytes());
        assert!(cells
            .append_visible_cell_indices(&bytes, 0, None, Some(&mut output))
            .unwrap_err()
            .contains("zero-length"));
    }

    #[test]
    fn object_candidate_batches_match_original_32_entry_limit_across_runs() {
        let first_run = (31u64 << 6) | 0;
        let second_run = (4u64 << 6) | 31;
        let packed = first_run | (second_run << 11);
        let bytes = packed.to_le_bytes();
        let cells = CellTree {
            bounds: Vec::new(),
            children: Vec::new(),
            axes: Vec::new(),
            planes: Vec::new(),
            leaf_ranks: Vec::new(),
            cell_codes: Vec::new(),
            cell_ids: Vec::new(),
            cell_visibility_counts: Vec::new(),
            direct_cell_ids_available: true,
            cell_bounds: Vec::new(),
            cell_link_ranges: Vec::new(),
            cell_candidate_ranges: vec![(0, 35)],
            links_at: 0,
            candidates_at: 0,
            candidate_index_bits: 6,
            candidate_run_bits: 5,
            candidate_record_count: 2,
            object_count: 64,
            code_width: 0,
            portal_nodes_at: None,
            portal_planes_at: None,
        };

        let mut batches = Vec::new();
        cells
            .for_each_object_candidate_batch(&bytes, 0, |batch| {
                batches.push(batch.to_vec());
                Ok(())
            })
            .unwrap();
        assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), [32, 3]);
        assert_eq!(
            batches.into_iter().flatten().collect::<Vec<_>>(),
            (0..35).collect::<Vec<_>>()
        );
    }

    #[test]
    fn resolves_direct_link_to_internal_record_cell_table() {
        let mut bytes = vec![0u8; 0x140];
        bytes[0x134..0x138].copy_from_slice(&3u32.to_le_bytes());
        let tree = SpatialTree {
            bounds: Vec::new(),
            children: Vec::new(),
            axes: Vec::new(),
            planes: Vec::new(),
            offsets: vec![0, 0x100],
            global_distance_scales: None,
        };
        let link = [1, 2 << 16, 0, 0];
        assert_eq!(
            tree.direct_link_destination(&bytes, link).unwrap(),
            Some(DirectLinkDestination {
                spatial_index: 1,
                record_offset: 0x100,
                cell_index: 2
            })
        );
        assert!(tree
            .direct_link_destination(&bytes, [1, 3 << 16, 0, 0])
            .is_err());
        assert_eq!(
            tree.direct_link_destination(&bytes, [1 << 28, 0, 0, 0])
                .unwrap(),
            None
        );
    }

    #[test]
    fn split_branch_stack_matches_original_child_insertions_and_order() {
        let tree = SpatialTree {
            bounds: Vec::new(),
            children: vec![Some([1, 2]), None, None],
            axes: vec![0, 3, 3],
            planes: vec![0.0; 3],
            offsets: vec![0; 3],
            global_distance_scales: None,
        };
        let mut bytes = [0x55; 0x28];
        bytes[0x0c..0x10].copy_from_slice(&0u32.to_le_bytes());
        bytes[0x1c] = 0;
        bytes[0x1d] = 0; // parent split axis x
        bytes[0x1e] = 0xff;
        let root = SpatialQueryBranchEntry::from_bytes(bytes);

        let mut high_first = SpatialQueryBranchStack::new(root);
        high_first
            .push_split(&tree, 3, 0x3f80_0000, false, 0x1111, 0x2222)
            .unwrap();
        assert_eq!(high_first.top(), 1);
        assert_eq!(high_first.entry(0).unwrap().spatial_node_index(), 1);
        assert_eq!(high_first.entry(0).unwrap().branch_mode(), 1);
        assert_eq!(high_first.entry(0).unwrap().query_state(), 0x1111);
        assert_eq!(high_first.entry(0).unwrap().bytes()[0x1e], 0xfd);
        assert_eq!(
            high_first.entry(0).unwrap().bytes()[0x10..0x14],
            0x3f80_0000u32.to_le_bytes()
        );
        assert_eq!(high_first.current().spatial_node_index(), 2);
        assert_eq!(high_first.current().branch_mode(), 2);
        assert_eq!(high_first.current().query_state(), 0x2222);
        assert_eq!(high_first.current().bytes()[0x1e], 0xfe);
        assert_eq!(high_first.current().bytes()[4..8], [0x55; 4]);

        let mut low_first = SpatialQueryBranchStack::new(root);
        low_first
            .push_split(&tree, 3, 0x3f80_0000, true, 0x1111, 0x2222)
            .unwrap();
        assert_eq!(low_first.current().spatial_node_index(), 1);
        assert_eq!(low_first.entry(0).unwrap().spatial_node_index(), 2);

        let mut one_child = SpatialQueryBranchStack::new(root);
        one_child
            .push_split(&tree, 1, 9, false, 0x3333, 0x4444)
            .unwrap();
        assert_eq!(one_child.top(), 0);
        assert_eq!(one_child.current().spatial_node_index(), 1);
        assert_eq!(one_child.current().query_state(), 0x3333);
    }

    #[test]
    fn spatial_branch_scheduler_selects_or_retires_branches_from_cell_states() {
        let tree = SpatialTree {
            bounds: Vec::new(),
            children: vec![Some([1, 2]), None, None],
            axes: vec![0, 3, 3],
            planes: vec![1.0; 3],
            offsets: vec![0; 3],
            global_distance_scales: None,
        };
        let mut bytes = [0u8; 0x28];
        bytes[0x0c..0x10].copy_from_slice(&0u32.to_le_bytes());
        bytes[0x1d] = 0;
        bytes[0x24..0x28].copy_from_slice(&0u32.to_le_bytes());

        let mut queued = SpatialQueryBranchStack::new(SpatialQueryBranchEntry::from_bytes(bytes));
        let candidate = queued
            .next_leaf(
                &tree,
                &[0x8001, 0x2001, 0x2002],
                &mut [0b111],
                &[1.0],
                0,
                1,
                SpatialBranchDirection::AxisValues([0.0; 3]),
            )
            .unwrap();
        assert_eq!(
            candidate,
            Some(SpatialQueryBranchCandidate {
                spatial_node_index: 1,
                cell_state_index: 1,
            })
        );

        let mut worklists = SpatialCellWorklists::new(3);
        let initial_cell = CellWorkItem {
            entry_id: 2,
            cell_index: 0,
        };
        worklists.prepend_pending_cell(1, initial_cell).unwrap();
        let activation = worklists
            .activate_spatial_candidate(candidate.unwrap())
            .unwrap();
        assert_eq!(
            activation,
            SpatialRecordActivation {
                spatial_node_index: 1,
                pending_slot: 1,
                activated_cells: 1,
            }
        );
        assert_eq!(
            worklists.active_worklist().entries().collect::<Vec<_>>(),
            [initial_cell]
        );
        assert_eq!(worklists.cell_states()[0], 0x8002);

        let mut empty = SpatialQueryBranchStack::new(SpatialQueryBranchEntry::from_bytes(bytes));
        let mut active_nodes = [0b111];
        assert_eq!(
            empty
                .next_leaf(
                    &tree,
                    &[0x8001, 0, 0],
                    &mut active_nodes,
                    &[1.0],
                    0,
                    1,
                    SpatialBranchDirection::AxisSides([true; 3]),
                )
                .unwrap(),
            None
        );
        assert!(!empty.is_active());
        assert_eq!(active_nodes, [0b001]);
    }

    #[test]
    fn ordinary_renderer_inverted_sentinel_selects_camera_point_volume() {
        let point = [0.0, 0.0, 0.0];
        let sentinel = [
            f32::MAX,
            f32::MAX,
            f32::MAX,
            -f32::MAX,
            -f32::MAX,
            -f32::MAX,
        ];
        assert_eq!(
            ordinary_spatial_query_volume(sentinel, point).unwrap(),
            OrdinarySpatialQueryVolume::CameraPoint(point)
        );
        assert_eq!(
            ordinary_spatial_query_volume([-2.0, -1.0, -1.0, 2.0, 1.0, 1.0], point).unwrap(),
            OrdinarySpatialQueryVolume::Bounds([[-2.0, -1.0, -1.0], [2.0, 1.0, 1.0]])
        );
        assert!(ordinary_spatial_query_volume(sentinel, [f32::NAN, 0.0, 0.0]).is_err());

        let tree = SpatialTree {
            bounds: vec![
                [[-2.0, -1.0, -1.0], [2.0, 1.0, 1.0]],
                [[-2.0, -1.0, -1.0], [0.0, 1.0, 1.0]],
                [[0.0, -1.0, -1.0], [2.0, 1.0, 1.0]],
            ],
            children: vec![Some([1, 2]), None, None],
            axes: vec![0, 3, 3],
            planes: vec![0.0; 3],
            offsets: vec![1, 1, 1],
            global_distance_scales: None,
        };
        let planes = [[0.0; 4]; 6];
        assert_eq!(
            tree.record_nodes_for_ordinary_query_prefilter(sentinel, point, &planes, [0.0; 3])
                .unwrap(),
            [0, 1, 2]
        );
        assert_eq!(
            tree.record_nodes_for_ordinary_query_prefilter(
                [-1.8, 0.0, 0.0, -0.2, 0.5, 0.5],
                point,
                &planes,
                [0.0; 3]
            )
            .unwrap(),
            Vec::<usize>::new()
        );
        assert_eq!(
            tree.record_nodes_for_ordinary_query_prefilter(
                [-3.0, -2.0, -2.0, 3.0, 2.0, 2.0],
                point,
                &planes,
                [0.0; 3]
            )
            .unwrap(),
            [0, 1, 2]
        );
    }

    #[test]
    fn original_query_root_gate_matches_all_three_volume_modes() {
        let tree = SpatialTree {
            bounds: vec![[[-2.0, -1.0, -1.0], [2.0, 1.0, 1.0]]],
            children: vec![None],
            axes: vec![3],
            planes: vec![0.0],
            offsets: vec![1],
            global_distance_scales: None,
        };
        assert!(original_query_accepts_tome_root(
            &tree,
            OriginalSpatialQueryVolume::Bounds([[-3.0; 3], [3.0; 3]])
        )
        .unwrap());
        assert!(!original_query_accepts_tome_root(
            &tree,
            OriginalSpatialQueryVolume::Bounds([[-1.0; 3], [1.0; 3]])
        )
        .unwrap());
        assert!(original_query_accepts_tome_root(
            &tree,
            OriginalSpatialQueryVolume::FourCorners([
                [0.0, 0.0, 0.0],
                [-1.0, 0.5, 0.0],
                [0.5, -0.5, 0.5],
                [1.0, 0.0, -1.0],
            ])
        )
        .unwrap());
        assert!(!original_query_accepts_tome_root(
            &tree,
            OriginalSpatialQueryVolume::FourCorners([
                [0.0, 0.0, 0.0],
                [-1.0, 0.5, 0.0],
                [0.5, -0.5, 0.5],
                [3.0, 0.0, 0.0],
            ])
        )
        .unwrap());
        assert!(original_query_accepts_tome_root(
            &tree,
            OriginalSpatialQueryVolume::Point([-2.0, 0.0, 1.0])
        )
        .unwrap());
        assert!(!original_query_accepts_tome_root(
            &tree,
            OriginalSpatialQueryVolume::Point([2.01, 0.0, 0.0])
        )
        .unwrap());
    }

    #[test]
    fn newly_reached_active_cell_is_prepended_and_marked_queued() {
        let mut worklists = SpatialCellWorklists::new(3);
        let item = CellWorkItem {
            entry_id: 2,
            cell_index: 1,
        };
        worklists.prepend_active_cell(item).unwrap();
        assert_eq!(
            worklists.active_worklist().entries().collect::<Vec<_>>(),
            [item]
        );
        assert_eq!(worklists.cell_states(), &[0, 0x8002, 0]);
        assert!(worklists.prepend_active_cell(item).is_err());
        assert!(worklists
            .prepend_active_cell(CellWorkItem {
                entry_id: 3,
                cell_index: 1,
            })
            .is_err());
    }

    #[test]
    fn work_node_pool_allocates_fresh_ids_then_flushes_recycled_head() {
        let mut pool = CellWorkNodePool::new(3).unwrap();
        let mut states = vec![0u16; 4];
        let mut coverage = VisitedCellCoverage::default();
        let first_owner = TraversalCellId {
            spatial_index: 5,
            cell_index: 0,
        };
        let second_owner = TraversalCellId {
            spatial_index: 8,
            cell_index: 1,
        };
        let first = pool
            .allocate(first_owner, &mut states, &mut coverage, |_, _, _| Ok(()))
            .unwrap()
            .unwrap();
        let second = pool
            .allocate(second_owner, &mut states, &mut coverage, |_, _, _| Ok(()))
            .unwrap()
            .unwrap();
        assert_eq!((first.entry_id, second.entry_id), (1, 2));
        assert_eq!(pool.free_count(), 0);
        assert!(pool
            .allocate(
                TraversalCellId {
                    spatial_index: 9,
                    cell_index: 2,
                },
                &mut states,
                &mut coverage,
                |_, _, _| Ok(())
            )
            .unwrap()
            .is_none());

        let mut mask = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        mask.bits[0] = 1;
        coverage.merge_incoming(first_owner, &mask).unwrap();
        coverage.merge_incoming(second_owner, &mask).unwrap();

        pool.release_processed(first, &mut states).unwrap();
        pool.release_processed(second, &mut states).unwrap();
        assert_eq!(pool.recycled_count(), 2);
        assert_eq!(states, [0x4001, 0x4002, 0, 0]);

        let mut flushed = Vec::new();
        let reused = pool
            .allocate(
                TraversalCellId {
                    spatial_index: 9,
                    cell_index: 2,
                },
                &mut states,
                &mut coverage,
                |old, owner, masks| {
                    assert_eq!(owner, second_owner);
                    assert!(masks.coverage(owner).is_some());
                    flushed.push(old);
                    Ok(())
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(flushed, [second]);
        assert_eq!(
            reused,
            CellWorkItem {
                entry_id: 2,
                cell_index: 2
            }
        );
        assert_eq!(pool.recycled_count(), 1);
        assert_eq!(states, [0x4001, 2, 0, 0]);
        assert!(coverage.coverage(second_owner).is_none());
        assert!(coverage.coverage(first_owner).is_some());
    }

    #[test]
    fn failed_recycled_cell_flush_keeps_pool_head_and_state() {
        let mut pool = CellWorkNodePool::new(2).unwrap();
        let mut states = vec![0u16; 2];
        let mut coverage = VisitedCellCoverage::default();
        let owner = TraversalCellId {
            spatial_index: 7,
            cell_index: 1,
        };
        let item = pool
            .allocate(owner, &mut states, &mut coverage, |_, _, _| Ok(()))
            .unwrap()
            .unwrap();
        pool.release_processed(item, &mut states).unwrap();
        let mut mask = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        mask.bits[0] = 1;
        coverage.merge_incoming(owner, &mask).unwrap();
        assert!(pool
            .allocate(
                TraversalCellId {
                    spatial_index: 7,
                    cell_index: 0,
                },
                &mut states,
                &mut coverage,
                |_, _, _| Err("candidate query failed".into())
            )
            .is_err());
        assert_eq!(pool.recycled_count(), 1);
        assert_eq!(states[1], 0x4001);
        assert!(coverage.coverage(owner).is_some());
    }

    #[test]
    fn record_end_flush_queries_recycled_cells_then_returns_them_to_free_head() {
        let mut pool = CellWorkNodePool::new(3).unwrap();
        let mut states = vec![0u16; 3];
        let mut coverage = VisitedCellCoverage::default();
        let owners = [
            TraversalCellId {
                spatial_index: 4,
                cell_index: 1,
            },
            TraversalCellId {
                spatial_index: 6,
                cell_index: 2,
            },
        ];
        let mut mask = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        mask.bits[0] = 1;
        let first = pool
            .allocate(owners[0], &mut states, &mut coverage, |_, _, _| Ok(()))
            .unwrap()
            .unwrap();
        let second = pool
            .allocate(owners[1], &mut states, &mut coverage, |_, _, _| Ok(()))
            .unwrap()
            .unwrap();
        coverage.merge_incoming(owners[0], &mask).unwrap();
        coverage.merge_incoming(owners[1], &mask).unwrap();
        pool.release_processed(first, &mut states).unwrap();
        pool.release_processed(second, &mut states).unwrap();

        let mut queried = Vec::new();
        assert_eq!(
            pool.flush_recycled(&mut states, &mut coverage, |item, owner, masks| {
                assert!(masks.coverage(owner).is_some());
                queried.push((item, owner));
                Ok(())
            })
            .unwrap(),
            2
        );
        assert_eq!(queried, [(second, owners[1]), (first, owners[0])]);
        assert_eq!(states, [0, 1, 2]);
        assert_eq!(pool.free_count(), 2);
        assert_eq!(pool.recycled_count(), 0);
        assert!(owners
            .iter()
            .all(|owner| coverage.coverage(*owner).is_none()));

        let reused = pool
            .allocate(
                TraversalCellId {
                    spatial_index: 9,
                    cell_index: 0,
                },
                &mut states,
                &mut coverage,
                |_, _, _| Ok(()),
            )
            .unwrap()
            .unwrap();
        assert_eq!(reused.entry_id, 1);
    }

    #[test]
    fn new_portal_coverage_schedules_unvisited_cells_by_spatial_record() {
        let mut worklists = SpatialCellWorklists::new(3);
        let mut coverage = VisitedCellCoverage::default();
        let mut incoming = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        incoming.bits[0] = 1;
        let active = CellWorkItem {
            entry_id: 2,
            cell_index: 1,
        };
        let active_id = TraversalCellId {
            spatial_index: 4,
            cell_index: 1,
        };

        assert!(worklists
            .propagate_and_schedule_unvisited_cell(
                &mut coverage,
                active_id,
                3,
                true,
                active,
                [0, 0, 4, 4],
                &incoming,
                PortalCoveragePath::Unclipped,
            )
            .unwrap());
        assert_eq!(
            worklists.active_worklist().entries().collect::<Vec<_>>(),
            [active]
        );
        assert_eq!(worklists.cell_states()[1], 0x8002);

        let pending = CellWorkItem {
            entry_id: 3,
            cell_index: 2,
        };
        let pending_id = TraversalCellId {
            spatial_index: 9,
            cell_index: 2,
        };
        assert!(worklists
            .propagate_and_schedule_unvisited_cell(
                &mut coverage,
                pending_id,
                7,
                false,
                pending,
                [0, 0, 4, 4],
                &incoming,
                PortalCoveragePath::Unclipped,
            )
            .unwrap());
        assert_eq!(worklists.pending_cell(7, 2), Some(pending));

        // New coverage for an already pending cell updates its mask without
        // allocating a second pending node.
        incoming.bits[0] = 2;
        assert!(worklists
            .propagate_and_schedule_unvisited_cell(
                &mut coverage,
                pending_id,
                7,
                false,
                pending,
                [0, 0, 4, 4],
                &incoming,
                PortalCoveragePath::Unclipped,
            )
            .unwrap());
        assert_eq!(worklists.pending_cell(7, 2), Some(pending));
    }

    #[test]
    fn pooled_portal_transition_returns_unused_node_before_active_enqueue() {
        let mut worklists = SpatialCellWorklists::new(2);
        let mut pool = CellWorkNodePool::new(2).unwrap();
        let mut coverage = VisitedCellCoverage::default();
        let destination = TraversalCellId {
            spatial_index: 12,
            cell_index: 1,
        };
        let mut no_bits = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        assert!(!worklists
            .propagate_and_schedule_unvisited_cell_from_pool(
                &mut pool,
                &mut coverage,
                destination,
                0,
                true,
                [0, 0, 4, 4],
                &no_bits,
                PortalCoveragePath::Unclipped,
                |_, _, _| Ok(()),
            )
            .unwrap());
        assert_eq!(pool.free_count(), 1);
        assert!(worklists.active_worklist().is_empty());
        assert_eq!(worklists.cell_states()[1], 0);
        assert!(coverage.coverage(destination).is_none());

        no_bits.bits[0] = 1;
        assert!(worklists
            .propagate_and_schedule_unvisited_cell_from_pool(
                &mut pool,
                &mut coverage,
                destination,
                0,
                true,
                [0, 0, 4, 4],
                &no_bits,
                PortalCoveragePath::Unclipped,
                |_, _, _| Ok(()),
            )
            .unwrap());
        assert_eq!(
            worklists.active_worklist().entries().collect::<Vec<_>>(),
            [CellWorkItem {
                entry_id: 1,
                cell_index: 1
            }]
        );
        assert_eq!(worklists.cell_states()[1], 0x8001);
    }

    #[test]
    fn pooled_cross_spatial_transition_reuses_existing_pending_node() {
        let mut worklists = SpatialCellWorklists::new(3);
        let mut pool = CellWorkNodePool::new(3).unwrap();
        let mut coverage = VisitedCellCoverage::default();
        let destination = TraversalCellId {
            spatial_index: 23,
            cell_index: 2,
        };
        // Numeric index 2 in the current record is a different local cell.
        worklists.cell_states_mut()[2] = 0x8002;
        let mut mask = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        mask.bits[0] = 1;
        assert!(worklists
            .propagate_and_schedule_unvisited_cell_from_pool(
                &mut pool,
                &mut coverage,
                destination,
                7,
                false,
                [0, 0, 4, 4],
                &mask,
                PortalCoveragePath::Unclipped,
                |_, _, _| Ok(()),
            )
            .unwrap());
        let first = worklists.pending_cell(7, 2).unwrap();
        assert_eq!(first.entry_id, 1);
        assert_eq!(pool.free_count(), 1);
        assert_eq!(worklists.cell_states()[2], 0x8002);

        mask.bits[0] = 2;
        assert!(worklists
            .propagate_and_schedule_unvisited_cell_from_pool(
                &mut pool,
                &mut coverage,
                destination,
                7,
                false,
                [0, 0, 4, 4],
                &mask,
                PortalCoveragePath::Unclipped,
                |_, _, _| Ok(()),
            )
            .unwrap());
        assert_eq!(worklists.pending_cell(7, 2), Some(first));
        assert_eq!(pool.free_count(), 1);
    }

    #[test]
    fn cross_spatial_portal_scheduling_resolves_original_pending_slot() {
        let mut worklists = SpatialCellWorklists::new(3);
        let mut pool = CellWorkNodePool::new(3).unwrap();
        let mut coverage = VisitedCellCoverage::default();
        let mut mapper = SpatialCellSlotMapper::default();
        let mut tome_bytes = vec![0; 0x104];
        tome_bytes[0x90..0x94].copy_from_slice(&9u32.to_le_bytes());
        tome_bytes[0x94..0x98].copy_from_slice(&2u32.to_le_bytes());
        tome_bytes[0x98..0x9c].copy_from_slice(&0x100u32.to_le_bytes());
        tome_bytes[0x100..0x104].copy_from_slice(&(1u32 << 2).to_le_bytes());
        let rows = SpatialCellSlotRows::parse(&tome_bytes).unwrap();
        let destination = TraversalCellId {
            spatial_index: 1,
            cell_index: 2,
        };
        let mut global_branch_states = vec![0u16; ORIGINAL_QUERY_CELL_STATE_COUNT];
        let mut incoming = PortalTileMask::new([0, 0, 1, 1]).unwrap();
        incoming.bits[0] = 1;

        // Target spatial record 1 selects bits 2..3. Its packed row has a
        // set first bit and then a clear bit; the scan resolves the pending
        // list head to slot 1 from the global branch states.
        assert!(worklists
            .propagate_and_schedule_cross_spatial_cell_from_pool(
                &mut mapper,
                &mut pool,
                &mut coverage,
                &rows,
                destination,
                &mut global_branch_states,
                [0, 0, 4, 4],
                &incoming,
                PortalCoveragePath::Unclipped,
                |_, _, _| Ok(()),
            )
            .unwrap());

        let pending = worklists.pending_cell(1, 2).unwrap();
        assert_eq!(worklists.cell_states()[2], 0);
        assert_eq!(worklists.activate_pending_spatial(1).unwrap(), 1);
        assert_eq!(
            worklists.active_worklist().entries().collect::<Vec<_>>(),
            [pending]
        );
        assert_eq!(worklists.cell_states()[2], pending.entry_id | 0x8000);
        assert_eq!(mapper.intermediate_slots(), &[1]);
        assert_eq!(
            global_branch_states[1],
            pending.entry_id | CellTraversalState::PROCESSED
        );
    }

    #[test]
    fn pending_cross_record_cell_uses_selected_record_on_activation() {
        let mut worklists = SpatialCellWorklists::new(4);
        let mut pool = CellWorkNodePool::new(4).unwrap();
        let mut coverage = VisitedCellCoverage::default();
        let target = TraversalCellId {
            spatial_index: 610,
            cell_index: 2,
        };
        let mask = PortalTileMask::full([0, 0, 1, 1]).unwrap();
        let node = worklists
            .seed_initial_camera_cell(&mut pool, &mut coverage, target, 5, &mask)
            .unwrap()
            .unwrap();
        assert_eq!(worklists.activate_pending_spatial(5).unwrap(), 1);
        worklists
            .bind_active_cells_to_spatial_record(4, &mut pool, &mut coverage)
            .unwrap();
        let selected = TraversalCellId {
            spatial_index: 4,
            cell_index: 2,
        };
        assert_eq!(pool.owners[node.entry_id as usize], Some(selected));
        assert!(coverage.coverage(target).is_none());
        assert_eq!(coverage.coverage(selected), Some(&mask));
    }

    #[test]
    fn selects_children_from_packed_codes_and_planes() {
        let mut data = vec![0u8; 0x120];
        data[0..4].copy_from_slice(&0xd600_0012u32.to_le_bytes());
        let size = data.len() as u32;
        data[8..12].copy_from_slice(&size.to_le_bytes());
        for (index, value) in [-2.0f32, -1.0, -1.0, 2.0, 1.0, 1.0].iter().enumerate() {
            data[0x14 + index * 4..0x18 + index * 4].copy_from_slice(&value.to_le_bytes());
        }
        data[0x2c..0x30].copy_from_slice(&96u32.to_le_bytes());
        data[0x30..0x34].copy_from_slice(&0x60u32.to_le_bytes());
        data[0x38..0x3c].copy_from_slice(&3u32.to_le_bytes());
        data[0x3c..0x40].copy_from_slice(&0x70u32.to_le_bytes());
        data[0xa0..0xa4].copy_from_slice(&0x80u32.to_le_bytes());
        data[0x60] = 0b0011_1100; // x split, then two leaves
        data[0x70..0x74].copy_from_slice(&0.0f32.to_le_bytes());
        let mut tree = SpatialTree::parse(&data).unwrap();
        let root_entry = tree.initial_query_branch_entry().unwrap();
        assert_eq!(root_entry.spatial_node_index(), 0);
        assert_eq!(root_entry.branch_mode(), 0);
        assert_eq!(root_entry.query_state(), 0);
        assert_eq!(
            &root_entry.bytes()[0..12],
            &[-2.0f32, -1.0, -1.0]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>()
        );
        assert_eq!(&root_entry.bytes()[12..16], &[0; 4]);
        assert_eq!(
            &root_entry.bytes()[16..28],
            &[2.0f32, 1.0, 1.0]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>()
        );
        assert_eq!(root_entry.bytes()[0x1c], 0); // root path length
        assert_eq!(root_entry.bytes()[0x1d], data[0x60] & 3);
        assert_eq!(&root_entry.bytes()[0x1e..0x20], &0x003fu16.to_le_bytes());
        let root_stack = SpatialQueryBranchStack::from_original_root(&tree).unwrap();
        assert_eq!(root_stack.top(), 0);
        assert_eq!(root_stack.current(), &root_entry);
        assert_eq!(tree.leaf_for_point([-1.0, 0.0, 0.0]), Some(1));
        assert_eq!(tree.leaf_for_point([0.0, 0.0, 0.0]), Some(2));
        assert_eq!(tree.leaf_for_point([3.0, 0.0, 0.0]), None);
        tree.offsets[1] = 1;
        tree.offsets[2] = 1;
        tree.offsets[0] = 1;
        assert_eq!(
            tree.record_leaves_intersecting_aabb([[-2.0, -1.0, -1.0], [-0.1, 1.0, 1.0]]),
            vec![1]
        );
        assert_eq!(
            tree.record_leaves_intersecting_aabb([[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]]),
            vec![1, 2]
        );
        assert_eq!(
            tree.record_leaves_containing_point([-1.0, 0.0, 0.0]),
            vec![1]
        );
        assert_eq!(
            tree.record_leaves_containing_point([0.0, 0.0, 0.0]),
            vec![1, 2]
        );
        assert!(tree
            .record_leaves_containing_point([f32::NAN, 0.0, 0.0])
            .is_empty());
        assert!(tree
            .record_leaves_intersecting_aabb([[3.0, 0.0, 0.0], [4.0, 1.0, 1.0]])
            .is_empty());
        let camera_planes = [
            [1.0, 0.0, 0.0, 2.0],
            [-1.0, 0.0, 0.0, -0.1],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, -1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, -1.0, 1.0],
        ];
        assert_eq!(
            tree.record_nodes_for_camera_query_prefilter([4.0, 0.0, 0.0], &camera_planes, [0.0; 3])
                .unwrap(),
            vec![0, 1]
        );
        assert_eq!(
            tree.record_nodes_for_camera_query_prefilter(
                [4.0, 0.0, 0.0],
                &camera_planes,
                [0.1, 0.0, 0.0]
            )
            .unwrap(),
            vec![0, 1, 2]
        );
        assert_eq!(
            tree.record_nodes_for_camera_query_prefilter(
                [-1.0, 0.0, 0.0],
                &camera_planes,
                [0.0; 3]
            )
            .unwrap(),
            vec![0, 1]
        );
        assert!(tree
            .record_nodes_for_camera_query_prefilter([4.0, 0.0, 0.0], &camera_planes[..5], [0.0; 3])
            .is_err());
        let left_quad = [
            [-1.0, -0.5, -0.5],
            [-1.0, 0.5, -0.5],
            [-1.0, 0.5, 0.5],
            [-1.0, -0.5, 0.5],
        ];
        assert_eq!(tree.record_leaves_intersecting_quad(left_quad), vec![1]);
        let base = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        assert_eq!(
            tree.record_leaves_for_quad_diagnostic(base, 0, 1).unwrap(),
            vec![1, 2]
        );
    }

    #[test]
    fn initial_camera_cell_seed_copies_full_mask_and_marks_pending_state() {
        let mut worklists = SpatialCellWorklists::new_for_original_query();
        let mut pool = CellWorkNodePool::new_for_original_query().unwrap();
        let mut coverage = VisitedCellCoverage::default();
        assert_eq!(
            worklists.cell_states().len(),
            ORIGINAL_RECORD_CELL_STATE_COUNT
        );
        assert_eq!(pool.free_count(), ORIGINAL_QUERY_WORK_NODE_COUNT - 1);
        let destination = TraversalCellId {
            spatial_index: 17,
            cell_index: 2,
        };
        let initial_mask = PortalTileMask::full([0, 0, 2, 1]).unwrap();
        let item = worklists
            .seed_initial_camera_cell(&mut pool, &mut coverage, destination, 1, &initial_mask)
            .unwrap()
            .unwrap();

        assert_eq!(
            item,
            CellWorkItem {
                entry_id: 1,
                cell_index: 2
            }
        );
        assert_eq!(worklists.pending_cell(1, 2), Some(item));
        assert_eq!(worklists.cell_states()[2], 0x8001);
        assert_eq!(coverage.coverage(destination).unwrap().bits, [u32::MAX; 2]);
        assert_eq!(
            worklists
                .seed_initial_camera_cell(&mut pool, &mut coverage, destination, 1, &initial_mask)
                .unwrap(),
            None
        );
        assert_eq!(pool.free_count(), ORIGINAL_QUERY_WORK_NODE_COUNT - 2);
    }

    #[test]
    fn parses_spatial_record_without_local_split_tree() {
        let mut data = vec![0u8; 0x180];
        data[0x54..0x58].copy_from_slice(&(1u32 | (1 << 5)).to_le_bytes());
        data[0x58..0x5c].copy_from_slice(&0x80u32.to_le_bytes());
        let base = 0x100;
        for (index, value) in [0.0f32, 0.0, 0.0, 2.0, 2.0, 2.0].iter().enumerate() {
            data[base + index * 4..base + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        data[base + 0x34..base + 0x38].copy_from_slice(&1u32.to_le_bytes());
        data[base + 0x38..base + 0x3c].copy_from_slice(&0x50u32.to_le_bytes());
        data[base + 0x3c..base + 0x40].copy_from_slice(&0x74u32.to_le_bytes());
        for (offset, packed) in [(0x18, 0u32), (0x1c, 0x0000_ffff), (0x20, 0xffff_ffff)] {
            data[base + 0x50 + offset..base + 0x54 + offset].copy_from_slice(&packed.to_le_bytes());
        }
        let cells = CellTree::parse(&data, base as u32).unwrap();
        assert_eq!(cells.node_count(), 0);
        assert_eq!(cells.cell_count(), 1);
        assert_eq!(cells.code_counts(), (0, 0, 0));
        assert!(cells
            .cell_for_point(&data, [1.0, 1.0, 1.0])
            .unwrap()
            .is_none());
        assert_eq!(
            cells.traversal_records(&data, 0).unwrap(),
            Vec::<[u32; 4]>::new()
        );
    }

    #[test]
    fn selects_direct_cell_code_from_local_tree() {
        let mut data = vec![0u8; 0x300];
        data[0x40..0x44].copy_from_slice(&16u32.to_le_bytes());
        data[0x54..0x58].copy_from_slice(&(4u32 | (3 << 5)).to_le_bytes());
        data[0x58..0x5c].copy_from_slice(&0x80u32.to_le_bytes());
        data[0x5c..0x60].copy_from_slice(&1u32.to_le_bytes());
        data[0x80] = 2 | (3 << 4); // object indices 2, 3, 4
        let base = 0x100;
        for (index, value) in [0.0f32, 0.0, 0.0, 2.0, 2.0, 2.0].iter().enumerate() {
            data[base + index * 4..base + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        data[base + 0x18..base + 0x1c].copy_from_slice(&(3u32 << 5 | 2).to_le_bytes());
        data[base + 0x1c..base + 0x20].copy_from_slice(&0x80u32.to_le_bytes());
        data[base + 0x20..base + 0x24].copy_from_slice(&0x90u32.to_le_bytes());
        data[base + 0x24..base + 0x28].copy_from_slice(&1u32.to_le_bytes());
        data[base + 0x28..base + 0x2c].copy_from_slice(&0xa0u32.to_le_bytes());
        data[base + 0x34..base + 0x38].copy_from_slice(&(2u32 | (1 << 16)).to_le_bytes());
        data[base + 0x38..base + 0x3c].copy_from_slice(&0xb0u32.to_le_bytes());
        data[base + 0x3c..base + 0x40].copy_from_slice(&0x120u32.to_le_bytes());
        data[base + 0x80] = 0b0011_1100;
        data[base + 0x90] = 0b0000_0100; // leaf codes 0, 1
        data[base + 0xa0..base + 0xa4].copy_from_slice(&1.0f32.to_le_bytes());
        data[base + 0xc0..base + 0xc4].copy_from_slice(&7u32.to_le_bytes());
        data[base + 0xe4..base + 0xe8].copy_from_slice(&9u32.to_le_bytes());
        data[base + 0xb4..base + 0xb8].copy_from_slice(&1u32.to_le_bytes());
        data[base + 0xbc..base + 0xc0].copy_from_slice(&3u32.to_le_bytes());
        for (offset, packed) in [(0x18, 0u32), (0x1c, 0x0000_ffff), (0x20, 0xffff_8000)] {
            data[base + 0xb0 + offset..base + 0xb4 + offset].copy_from_slice(&packed.to_le_bytes());
        }
        for (index, value) in [2u32, 3, 4, 5].iter().enumerate() {
            data[base + 0x120 + index * 4..base + 0x124 + index * 4]
                .copy_from_slice(&value.to_le_bytes());
        }
        let cells = CellTree::parse(&data, base as u32).unwrap();
        let low = cells
            .cell_for_point(&data, [0.5, 1.0, 1.0])
            .unwrap()
            .unwrap();
        let high = cells
            .cell_for_point(&data, [1.0, 1.0, 1.0])
            .unwrap()
            .unwrap();
        assert_eq!(
            (low.tree_node, low.leaf_rank, low.cell_index, low.cell_id),
            (1, 0, 0, 7)
        );
        let mut seeded_worklists = SpatialCellWorklists::new(2);
        let mut seed_pool = CellWorkNodePool::new(4).unwrap();
        let mut seed_coverage = VisitedCellCoverage::default();
        let initial_mask = PortalTileMask::full([0, 0, 2, 1]).unwrap();
        let selected_tree = SpatialTree {
            bounds: vec![[[0.0; 3], [2.0; 3]]; 2],
            children: vec![None; 2],
            axes: vec![3; 2],
            planes: vec![0.0; 2],
            offsets: vec![base as u32; 2],
            global_distance_scales: None,
        };
        let seeded = selected_tree
            .seed_initial_camera_cell_for_candidate(
                &data,
                [0.5, 1.0, 1.0],
                SpatialQueryBranchCandidate {
                    spatial_node_index: 0,
                    cell_state_index: 17,
                },
                &mut seeded_worklists,
                &mut seed_pool,
                &mut seed_coverage,
                &initial_mask,
            )
            .unwrap()
            .unwrap();
        assert_eq!(seeded.spatial_node_index, 0);
        assert_eq!(seeded.pending_slot, 17);
        assert_eq!(
            (seeded.selection.cell_index, seeded.selection.cell_id),
            (0, 7)
        );
        assert_eq!(
            seeded.work_item,
            CellWorkItem {
                entry_id: 1,
                cell_index: 0
            }
        );
        assert_eq!(seeded_worklists.cell_states()[0], 0x8001);
        assert_eq!(seeded_worklists.pending_cell(17, 0), Some(seeded.work_item));
        let second_seed = selected_tree
            .seed_initial_camera_cell_for_candidate(
                &data,
                [0.5, 1.0, 1.0],
                SpatialQueryBranchCandidate {
                    spatial_node_index: 1,
                    cell_state_index: 18,
                },
                &mut seeded_worklists,
                &mut seed_pool,
                &mut seed_coverage,
                &initial_mask,
            )
            .unwrap()
            .unwrap();
        assert_eq!(seeded_worklists.cell_states()[0], 0x8002);
        assert_eq!(seeded_worklists.pending_cell(17, 0), Some(seeded.work_item));
        assert_eq!(
            seeded_worklists.pending_cell(18, 0),
            Some(second_seed.work_item)
        );
        assert_eq!(
            (
                high.tree_node,
                high.leaf_rank,
                high.cell_index,
                high.cell_id
            ),
            (2, 1, 1, 9)
        );
        assert_eq!(low.link_count, 1);
        let decoded = cells.cell_bounds(0).unwrap();
        assert_eq!(decoded[0], [0.0, 0.0, 0.0]);
        assert_eq!(decoded[1][0], 2.0);
        assert_eq!(decoded[1][1], 2.0);
        assert!((decoded[1][2] - 1.0000153).abs() < 0.000001);
        data[base + 0x124..base + 0x128].copy_from_slice(&((1u32 << 16) | 3).to_le_bytes());
        assert_eq!(
            cells.traversal_records(&data, 0).unwrap(),
            vec![[2, (1 << 16) | 3, 4, 5]]
        );
        assert_eq!(
            cells
                .first_ready_same_spatial_cell_target(
                    &data,
                    2,
                    0,
                    LinkQueryDirection::Volume {
                        center: [0; 4],
                        tolerance: [0; 4],
                    },
                    &[0, 0x8003],
                )
                .unwrap(),
            Some(ReadyCellTarget {
                cell_index: 1,
                worklist_entry: 3,
            })
        );
        let mut spatial_lists = SpatialCellWorklists::new(2);
        spatial_lists
            .prepend_pending_cell(
                17,
                CellWorkItem {
                    entry_id: 2,
                    cell_index: 1,
                },
            )
            .unwrap();
        spatial_lists
            .prepend_pending_cell(
                17,
                CellWorkItem {
                    entry_id: 1,
                    cell_index: 0,
                },
            )
            .unwrap();
        assert_eq!(
            spatial_lists.pending_cell(17, 1),
            Some(CellWorkItem {
                entry_id: 2,
                cell_index: 1,
            })
        );
        assert_eq!(spatial_lists.activate_pending_spatial(17).unwrap(), 2);
        assert_eq!(
            spatial_lists
                .active_worklist()
                .entries()
                .collect::<Vec<_>>(),
            vec![
                CellWorkItem {
                    entry_id: 1,
                    cell_index: 0,
                },
                CellWorkItem {
                    entry_id: 2,
                    cell_index: 1,
                },
            ]
        );
        assert_eq!(spatial_lists.cell_states(), &[0x8001, 0x8002]);
        assert_eq!(
            spatial_lists
                .select_active_cell(
                    &cells,
                    &data,
                    2,
                    LinkQueryDirection::Volume {
                        center: [0; 4],
                        tolerance: [0; 4],
                    },
                )
                .unwrap(),
            Some(CellWorkItem {
                entry_id: 2,
                cell_index: 1,
            })
        );
        assert_eq!(
            spatial_lists
                .active_worklist()
                .entries()
                .collect::<Vec<_>>(),
            vec![CellWorkItem {
                entry_id: 1,
                cell_index: 0,
            }]
        );
        assert_eq!(spatial_lists.cell_states(), &[0x8001, 0x8002]);
        assert_eq!(cells.object_candidates(&data, 0).unwrap(), vec![2, 3, 4]);

        // The original scan ends on its first link to another spatial record,
        // even when a later link points back to this one.
        let mut ordered = data.clone();
        ordered[base + 0xb4..base + 0xb8].copy_from_slice(&2u32.to_le_bytes());
        ordered[base + 0x120..base + 0x124].copy_from_slice(&3u32.to_le_bytes());
        ordered[base + 0x130..base + 0x134].copy_from_slice(&2u32.to_le_bytes());
        ordered[base + 0x134..base + 0x138].copy_from_slice(&((1u32 << 16) | 3).to_le_bytes());
        let ordered_cells = CellTree::parse(&ordered, base as u32).unwrap();
        assert_eq!(
            ordered_cells
                .first_ready_same_spatial_cell_target(
                    &ordered,
                    2,
                    0,
                    LinkQueryDirection::Volume {
                        center: [0; 4],
                        tolerance: [0; 4],
                    },
                    &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0x8003],
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn resolves_portal_code_with_authored_plane_and_terminal_children() {
        let mut data = vec![0u8; 0x400];
        data[0x54..0x58].copy_from_slice(&1u32.to_le_bytes());
        let base = 0x100;
        for (index, value) in [0.0f32, 0.0, 0.0, 2.0, 2.0, 2.0].iter().enumerate() {
            data[base + index * 4..base + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        data[base + 0x18..base + 0x1c].copy_from_slice(&(1u32 << 5 | 2).to_le_bytes());
        for (field, offset) in [
            (0x1c, 0x80u32),
            (0x20, 0x90),
            (0x38, 0xb0),
            (0x40, 0x100),
            (0x48, 0x110),
        ] {
            data[base + field..base + field + 4].copy_from_slice(&offset.to_le_bytes());
        }
        data[base + 0x34..base + 0x38].copy_from_slice(&(2u32 | (1 << 16)).to_le_bytes());
        data[base + 0x80] = 3; // one local leaf
        data[base + 0x90] = 2; // high-bit portal code, node zero
        data[base + 0xc0..base + 0xc4].copy_from_slice(&7u32.to_le_bytes());
        data[base + 0xe4..base + 0xe8].copy_from_slice(&9u32.to_le_bytes());
        data[base + 0x100..base + 0x104].copy_from_slice(&0xc000_0000u32.to_le_bytes());
        data[base + 0x104..base + 0x108].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        for (index, value) in [1.0f32, 0.0, 0.0, -1.0].iter().enumerate() {
            data[base + 0x110 + index * 4..base + 0x114 + index * 4]
                .copy_from_slice(&value.to_le_bytes());
        }
        let cells = CellTree::parse(&data, base as u32).unwrap();
        assert_eq!(
            cells
                .cell_for_point(&data, [0.5, 0.5, 0.5])
                .unwrap()
                .unwrap()
                .cell_id,
            7
        );
        assert_eq!(
            cells
                .cell_for_point(&data, [1.5, 0.5, 0.5])
                .unwrap()
                .unwrap()
                .cell_id,
            9
        );
    }
}
