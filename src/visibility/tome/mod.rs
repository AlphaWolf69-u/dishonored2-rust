//! Authored global spatial tree from the game's Umbra Tome v18.
//!
//! FUN_140a7b6c0 and FUN_140a89500 use the header's packed two-bit tree;
//! FUN_140a89660 chooses a branch using the recorded split plane, and
//! FUN_140a7a710 derives child indices from the rank of internal nodes.
//! The ordinary camera query combines global spatial selection, local cell
//! traversal, portal coverage, and per-cell object acceptance.

use std::collections::{HashMap, HashSet, VecDeque};

mod cell_worklist;
mod portal_raster;
pub use cell_worklist::CellTraversalWorklist;
#[path = "shadow_caster.rs"]
pub mod shadow_caster;
#[cfg(target_arch = "x86_64")]
mod simd_predicates;
#[path = "tome_sphere.rs"]
mod sphere;
pub use sphere::{
    SphereBatchResult, SphereCellGraph, SphereCellResult, SphereCellWorker,
    sphere_portal_gates_allow,
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
    cell_bounds_root: [[f32; 3]; 2],
    cell_table_at: usize,
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
    ///Some only when the native perspective baked-occlusion stage is enabled.
    pub point_occlusion_near_offset: Option<f32>,
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

    pub fn object_count(&self) -> usize {
        self.object_count
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

/// Number of Tome object indices in the header, shared by cached renderer
/// views so frame queries do not reread or reinterpret the header.
pub fn object_count(bytes: &[u8]) -> Result<usize, String> {
    Ok(word(bytes, 0x40)? as usize)
}

/// FUN_140a767e0/FUN_140a768d0 map one Tome object index to the authored
/// object IDs consumed by the world renderer. With header+0x4c present, it
/// indexes two offsets into the ID table at header+0x50; otherwise there is
/// one ID per Tome index. Empty groups are valid in the packed format.
pub fn object_ids_for_index(bytes: &[u8], index: u32) -> Result<Vec<u32>, String> {
    let mut result = Vec::new();
    append_object_ids_for_index(bytes, index, &mut result)?;
    Ok(result)
}

/// Append FUN_140a767e0/FUN_140a768d0's mapped IDs without allocating a
/// temporary per object. Cached renderer tables use this during load, then
/// reuse their flattened ID storage for every frame.
pub fn append_object_ids_for_index(
    bytes: &[u8],
    index: u32,
    output: &mut Vec<u32>,
) -> Result<(), String> {
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
    for i in start..end {
        output.push(word(bytes, ids_at + i * 4)?);
    }
    Ok(())
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
#[inline(always)]
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
#[inline(always)]
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
    #[cfg(target_arch = "x86_64")]
    return simd_predicates::object_distance_rejection_mask(record, point, scale_squared);
    #[cfg(not(target_arch = "x86_64"))]
    object_distance_rejection_mask_scalar(record, point, scale_squared)
}

#[cfg(any(test, not(target_arch = "x86_64")))]
fn object_distance_rejection_mask_scalar(
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

#[inline(always)]
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
    #[inline(always)]
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

    #[inline(always)]
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

    #[inline(always)]
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
    #[inline(always)]
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

/// FUN_140a82340 prepares scale and padding once; FUN_140a82710 then
/// decodes the six u16 cell bounds in this order before emitting query depth.
/// Do not expand the already decoded CellTree::cell_bounds: multiplying the
/// quantized value before adding the base/padding preserves native rounding.
pub fn quantized_cell_query_bounds(
    root: [[f32; 3]; 2],
    quantized: [[u16; 3]; 2],
    record_padding: f32,
    camera_imprecision: [f32; 3],
) -> Result<[[f32; 4]; 2], String> {
    if root
        .iter()
        .flatten()
        .chain(camera_imprecision.iter())
        .any(|value| !value.is_finite())
        || !record_padding.is_finite()
    {
        return Err("nonfinite Tome cell-depth bounds".into());
    }
    let mut bounds = [[0.0; 4]; 2];
    for axis in 0..3 {
        let scale = (root[1][axis] - root[0][axis]) * (1.0 / 65535.0);
        let margin = record_padding + camera_imprecision[axis];
        bounds[0][axis] = quantized[0][axis] as f32 * scale + (root[0][axis] - margin);
        // 140a83e31 adds base+margin before 140a83e54 adds q*scale.
        bounds[1][axis] = quantized[1][axis] as f32 * scale + (root[0][axis] + margin);
    }
    bounds[0][3] = 1.0;
    bounds[1][3] = 1.0;
    Ok(bounds)
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

/// FUN_140a77560/FUN_140a77c00 plane and maximum-distance-box fields.
/// The caster initializer FUN_140a72df0 uses the full viewport and zero
/// imprecision; ordinary camera jobs may provide cropped rows instead.
pub fn query_plane_geometry_from_rows(
    render_rows: [[f32; 4]; 4],
    cropped_rows: [[f32; 4]; 4],
    camera_imprecision: [f32; 3],
) -> Result<([[f32; 4]; 6], [[f32; 4]; 2]), String> {
    let mut planes = query_planes_from_rows(render_rows, cropped_rows);
    // Only the near plane is normalized by the original constructor.
    let [x, y, z, _] = planes[0];
    let reciprocal_length = 1.0 / (y * y + x * x + z * z).sqrt();
    for coefficient in &mut planes[0] {
        *coefficient *= reciprocal_length;
    }
    if planes
        .iter()
        .flatten()
        .chain(camera_imprecision.iter())
        .any(|v| !v.is_finite())
    {
        return Err("nonfinite Tome spatial-query box or plane".into());
    }
    Ok((
        planes,
        query_bounds_from_planes(&planes, camera_imprecision)?,
    ))
}

/// FUN_140a77b10 initializes the plane-query portion of an existing native
/// context. Preserve every byte that the original leaves untouched. In
/// particular, primary-query bounds are +/-f32::MAX in all four lanes, NOT
/// the finite intersection bounds of a receiver or cascade.
///
/// The three 22-plane regions end at 0x1e0/0x340/0x4a0 respectively. A larger
/// slice cannot represent this original layout and is rejected before writes.
/// Executable fixture: tests/fixtures/plane_initialization_cases.txt.
pub fn initialize_native_plane_query(
    context: &mut [u8; 0x590],
    planes: &[[f32; 4]],
) -> Result<(), String> {
    if planes.len() > 22 {
        return Err("native plane-query context supports at most 22 planes".into());
    }
    for (offset, value) in [(0x580, 0u32), (0x584, 1), (0x4ac, planes.len() as u32)] {
        context[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    context[0x4c0..0x4d0].fill(0);
    context[0x4d0] = 0;
    for (index, plane) in planes.iter().enumerate() {
        let at = 0x4d4 + index * 4;
        context[at..at + 4].copy_from_slice(&(index as u32).to_le_bytes());
        for (axis, &coefficient) in plane.iter().enumerate() {
            for base in [0x80, 0x1e0] {
                let at = base + index * 16 + axis * 4;
                context[at..at + 4].copy_from_slice(&coefficient.to_bits().to_le_bytes());
            }
            let at = 0x340 + index * 16 + axis * 4;
            let sign = if 0.0 < coefficient { u32::MAX } else { 0 };
            context[at..at + 4].copy_from_slice(&sign.to_le_bytes());
        }
    }
    for (base, value) in [(0x530, -f32::MAX), (0x540, f32::MAX)] {
        for axis in 0..4 {
            let at = base + axis * 4;
            context[at..at + 4].copy_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    Ok(())
}

/// FUN_140a76ac0 selects query planes that can intersect a box. The sign
/// masks at query+0x340 are generated from positive plane components by
/// FUN_140a77b10; the original SIMD bit-select uses the lower bound for those
/// lanes and the upper bound otherwise, yielding the minimum plane distance.
/// A retained plane is passed to later clipping; this alone does not establish
/// visibility.
pub fn active_query_planes(planes: &[[f32; 4]], bounds: [[f32; 4]; 2]) -> Vec<usize> {
    intersecting_query_plane_positions(planes, bounds, 0..planes.len())
}

/// Reuse caller-owned plane-index storage for the same FUN_140a76ac0 path.
pub fn active_query_planes_into(
    planes: &[[f32; 4]],
    bounds: [[f32; 4]; 2],
    output: &mut Vec<usize>,
) {
    fill_query_plane_positions(planes, bounds, 0..planes.len(), output);
}

/// FUN_140a76ac0 with param_3 != NULL tests the supplied plane indices,
/// but emits positions within that list (uVar11 & 0xff), not plane IDs.
/// Cell-depth clipping consumes these native slot values unchanged.
pub fn active_query_plane_positions(
    planes: &[[f32; 4]],
    bounds: [[f32; 4]; 2],
    candidates: &[usize],
) -> Result<Vec<usize>, String> {
    if candidates.iter().any(|&index| index >= planes.len()) {
        return Err("invalid Tome active-plane candidate index".into());
    }
    Ok(intersecting_query_plane_positions(
        planes,
        bounds,
        candidates.iter().copied(),
    ))
}

fn intersecting_query_plane_positions(
    planes: &[[f32; 4]],
    bounds: [[f32; 4]; 2],
    candidates: impl Iterator<Item = usize>,
) -> Vec<usize> {
    let mut output = Vec::new();
    fill_query_plane_positions(planes, bounds, candidates, &mut output);
    output
}

fn fill_query_plane_positions(
    planes: &[[f32; 4]],
    bounds: [[f32; 4]; 2],
    candidates: impl Iterator<Item = usize>,
    output: &mut Vec<usize>,
) {
    output.clear();
    for (slot, index) in candidates.enumerate() {
        let plane = planes[index];
        let products: [f32; 4] = std::array::from_fn(|axis| {
            let corner = if plane[axis] > 0.0 {
                bounds[0][axis]
            } else {
                bounds[1][axis]
            };
            corner * plane[axis]
        });
        // MULPS/SHUFPS/ADDPS adds x+y, then z, then w.
        let distance = ((products[0] + products[1]) + products[2]) + products[3];
        if distance <= 0.0 {
            output.push(slot & 0xff);
        }
    }
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
    let minimum = query_plane_minimum(planes, input, active, true, 0.0)?;
    Ok(!(minimum < 0.0
        || (0..4).any(|axis| {
            query_bounds[1][axis] < input[0][axis] || input[1][axis] < query_bounds[0][axis]
        })))
}

/// FUN_140a76f30: minimum-corner containment, with no query-bounds gate.
/// The caster query uses this to stop testing later receiver/cascade volumes
/// once a box is completely contained. This is NOT a depth-occlusion test.
/// DAT_141f69a00 initializes the original four-lane MINPS accumulator to 1.
pub fn query_planes_contain_aabb(
    planes: &[[f32; 4]],
    input: [[f32; 4]; 2],
    active: &[usize],
) -> Result<bool, String> {
    Ok(!(query_plane_minimum(planes, input, active, false, 1.0)? < 0.0))
}

/// Three-way receiver/cascade classification composed from the already
/// ported FUN_140a71b90 broad acceptance and FUN_140a76f30 minimum-corner
/// containment predicates. This classifies bounds against a caller-provided
/// verified receiver state; it does not construct the native receiver volume
/// or perform FUN_140a707e0's extrusion/distance setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeReceiverBoundsClass {
    Outside,
    Intersecting,
    Contained,
}

pub fn classify_native_receiver_bounds(
    receiver_bounds: [[f32; 4]; 2],
    object_bounds: [[f32; 4]; 2],
    planes: &[[f32; 4]],
    active: &[usize],
) -> Result<NativeReceiverBoundsClass, String> {
    if receiver_bounds
        .iter()
        .flatten()
        .chain(object_bounds.iter().flatten())
        .any(|value| !value.is_finite())
        || (0..4).any(|axis| {
            receiver_bounds[0][axis] > receiver_bounds[1][axis]
                || object_bounds[0][axis] > object_bounds[1][axis]
        })
    {
        return Err("invalid native receiver/object bounds".into());
    }
    if !query_bounds_accepts_aabb(receiver_bounds, object_bounds, planes, active)? {
        return Ok(NativeReceiverBoundsClass::Outside);
    }
    if query_planes_contain_aabb(planes, object_bounds, active)? {
        Ok(NativeReceiverBoundsClass::Contained)
    } else {
        Ok(NativeReceiverBoundsClass::Intersecting)
    }
}

/// Distance branches observed in FUN_140a73bf0. Only the explicitly verified
/// unbounded path with no per-node records is handled by the full-domain query.
/// A bounded descriptor or node+0x48 records must not be silently treated as
/// an AABB-only query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeShadowDistancePath {
    /// Caller verified param_4 is null/has no positive +0x0c radius and every
    /// traversed node has a zero +0x48 distance-record offset.
    UnboundedNoAuxiliaryRecords,
    BoundedOrAuxiliaryRecords,
    Unknown,
}

/// Parent/child IDs consumed directly as byte offsets into FUN_140330c10's
/// per-receiver caster mask. Keep these in the original traversal order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeShadowCasterSuppression {
    pub parent_id: u32,
    pub suppressed_child_id: u32,
}

/// State for the implemented original-backed AABB candidate query. Receiver
/// bounds/planes must already be built from FUN_140a707e0/FUN_140a88460;
/// caster flags and ordered suppression links must come from original-backed
/// state. The query scans the full cached Tome object domain, never ordinary
/// camera visibility output. It does not build FUN_140330c10's
/// +0x1840/+0x1850 instance bounds or apply native per-node candidate filters.
#[derive(Debug, Clone, Copy)]
pub struct NativeShadowCasterAabbQueryInput<'a> {
    pub receivers: &'a [shadow_caster::VerifiedShadowReceiverState<'a>],
    pub caster_enabled: &'a [bool],
    pub parent_suppression: &'a [NativeShadowCasterSuppression],
    pub distance_path: NativeShadowDistancePath,
}

/// Reusable full-Tome AABB candidate query. Object AABBs and authored IDs are
/// read from `ShadowCasterTome`'s load-time cache; bitsets, output rows, and
/// ID deduplication storage are allocated once here and retained across frames.
/// `query_candidates` is the broad AABB path. `query_directional_candidates`
/// additionally applies both native baked-direction table encodings. Neither
/// implements the bounded/per-node distance or receiver-depth branches, so
/// neither is yet the complete gatherCaster result.
#[derive(Debug)]
pub struct NativeShadowCasterAabbQueryScratch {
    object_count: usize,
    selected_by_receiver: Vec<Vec<u32>>,
    output_by_receiver: Vec<Vec<u32>>,
    active_ids: HashSet<u32>,
}

impl NativeShadowCasterAabbQueryScratch {
    pub fn new(
        tome: &shadow_caster::ShadowCasterTome,
        receiver_count: usize,
    ) -> Result<Self, String> {
        let object_count = tome.object_count();
        let word_count = object_count
            .checked_add(31)
            .ok_or("shadow caster mask size overflow")?
            / 32;
        let mut id_capacity = 0usize;
        for index in 0..object_count {
            let object = tome
                .object(index as u32)
                .ok_or("shadow caster Tome object index is absent")?;
            id_capacity = id_capacity
                .checked_add(object.object_ids.len())
                .ok_or("shadow caster ID count overflow")?;
        }
        Ok(Self {
            object_count,
            selected_by_receiver: (0..receiver_count).map(|_| vec![0; word_count]).collect(),
            output_by_receiver: (0..receiver_count)
                .map(|_| Vec::with_capacity(id_capacity))
                .collect(),
            active_ids: HashSet::with_capacity(id_capacity),
        })
    }

    /// Recompute per-receiver AABB candidate IDs by testing every cached Tome
    /// object against caller-provided receiver states. Containment stops later
    /// receiver tests as FUN_140a76f30 does; then ordered authored-ID parent
    /// suppression follows FUN_140330c10. This is not the complete gatherCaster
    /// result: FUN_140a73bf0 reads 24-byte candidate records at node+0x44 and,
    /// when `*(u32 *)(node+0x48) != 0` (`local_34c`), 32-byte distance records
    /// there, with a radius descriptor at param_4+0x0c. `FUN_140a7bbf0` obtains
    /// the work record through FUN_140a7bdb0 using `(ctx+0x40)+(*(i32 *)(ctx+0x4c)
    /// + index*0x20)`; its mapping to Tome candidate records remains unresolved.
    /// Trace that mapping or capture runtime values before enabling these filters.
    pub fn query_candidates<'a>(
        &'a mut self,
        tome: &shadow_caster::ShadowCasterTome,
        input: NativeShadowCasterAabbQueryInput<'_>,
    ) -> Result<&'a [Vec<u32>], String> {
        self.query_candidates_filtered(tome, input, |_| Ok(true))
    }

    /// Apply FUN_140a73bf0's 0x140a74a19/0x140a7538d direction gate after
    /// broad acceptance, before publishing any receiver mask. The low two
    /// node-header flag bits choose dense one-bit or palette/nibble tables;
    /// the palette accepts precisely 0x7f7f. The immutable bytes must be the
    /// same global Tome resource used to build `tome`. No table is copied or
    /// allocated in the frame loop. Distance/depth branches remain explicit
    /// blockers rather than being replaced by this gate.
    pub fn query_directional_candidates<'a>(
        &'a mut self,
        tome: &shadow_caster::ShadowCasterTome,
        bytes: &[u8],
        receiver_direction: [f32; 3],
        input: NativeShadowCasterAabbQueryInput<'_>,
    ) -> Result<&'a [Vec<u32>], String> {
        let direction = object_count(bytes).and_then(|count| {
            if count != self.object_count {
                return Err(
                    "shadow direction tables belong to a different Tome object domain".into(),
                );
            }
            shadow_caster::NativeCasterDirection::from_receiver_direction(receiver_direction)
        });
        let direction = match direction {
            Ok(direction) => direction,
            Err(error) => {
                self.reset_output();
                return Err(error);
            }
        };
        self.query_candidates_filtered(tome, input, |index| direction.allows_object(bytes, index))
    }

    fn reset_output(&mut self) {
        for mask in &mut self.selected_by_receiver {
            mask.fill(0);
        }
        for output in &mut self.output_by_receiver {
            output.clear();
        }
        self.active_ids.clear();
    }

    fn query_candidates_filtered<'a>(
        &'a mut self,
        tome: &shadow_caster::ShadowCasterTome,
        input: NativeShadowCasterAabbQueryInput<'_>,
        mut direction_allows: impl FnMut(u32) -> Result<bool, String>,
    ) -> Result<&'a [Vec<u32>], String> {
        self.reset_output();

        if input.distance_path != NativeShadowDistancePath::UnboundedNoAuxiliaryRecords {
            return Err("unsupported native shadow distance branch".into());
        }
        if tome.object_count() != self.object_count {
            return Err("shadow caster scratch belongs to a different Tome object domain".into());
        }
        if input.receivers.len() != self.selected_by_receiver.len() {
            return Err("shadow caster scratch receiver count mismatch".into());
        }
        if input.caster_enabled.len() != self.object_count {
            return Err("shadow caster flag table does not match Tome object count".into());
        }
        for receiver in input.receivers {
            if receiver
                .bounds
                .iter()
                .flatten()
                .any(|value| !value.is_finite())
                || (0..3).any(|axis| receiver.bounds[0][axis] > receiver.bounds[1][axis])
            {
                return Err("invalid native shadow receiver bounds".into());
            }
            if receiver
                .active_planes
                .iter()
                .any(|&index| index >= receiver.planes.len())
            {
                return Err("invalid native shadow receiver plane index".into());
            }
        }

        for object_index in 0..self.object_count {
            if !input.caster_enabled[object_index] {
                continue;
            }
            let object = tome
                .object(object_index as u32)
                .ok_or("shadow caster Tome object index is absent")?;
            let object_bounds = [
                [
                    object.bounds[0][0],
                    object.bounds[0][1],
                    object.bounds[0][2],
                    1.0,
                ],
                [
                    object.bounds[1][0],
                    object.bounds[1][1],
                    object.bounds[1][2],
                    1.0,
                ],
            ];
            let mut direction_checked = false;
            for (receiver_index, receiver) in input.receivers.iter().enumerate() {
                let receiver_bounds = [
                    [
                        receiver.bounds[0][0],
                        receiver.bounds[0][1],
                        receiver.bounds[0][2],
                        1.0,
                    ],
                    [
                        receiver.bounds[1][0],
                        receiver.bounds[1][1],
                        receiver.bounds[1][2],
                        1.0,
                    ],
                ];
                let class = classify_native_receiver_bounds(
                    receiver_bounds,
                    object_bounds,
                    receiver.planes,
                    receiver.active_planes,
                )?;
                if class != NativeReceiverBoundsClass::Outside && !direction_checked {
                    if !direction_allows(object_index as u32)? {
                        break;
                    }
                    direction_checked = true;
                }
                match class {
                    NativeReceiverBoundsClass::Outside => continue,
                    NativeReceiverBoundsClass::Intersecting => {
                        self.selected_by_receiver[receiver_index][object_index / 32] |=
                            1 << (object_index % 32);
                    }
                    NativeReceiverBoundsClass::Contained => {
                        self.selected_by_receiver[receiver_index][object_index / 32] |=
                            1 << (object_index % 32);
                        break;
                    }
                }
            }
        }

        for receiver_index in 0..self.selected_by_receiver.len() {
            let selected = &self.selected_by_receiver[receiver_index];
            let output = &mut self.output_by_receiver[receiver_index];
            for object_index in 0..self.object_count {
                if selected[object_index / 32] & (1 << (object_index % 32)) == 0 {
                    continue;
                }
                let object = tome
                    .object(object_index as u32)
                    .ok_or("shadow caster Tome object index is absent")?;
                for &id in object.object_ids {
                    if !self.active_ids.contains(&id) {
                        if self.active_ids.len() == self.active_ids.capacity()
                            || output.len() == output.capacity()
                        {
                            return Err("shadow caster scratch ID capacity mismatch".into());
                        }
                        self.active_ids.insert(id);
                        output.push(id);
                    }
                }
            }
            // Native parent suppression observes the mask after earlier rows;
            // do not turn this into a set of independent parent tests.
            for edge in input.parent_suppression {
                if self.active_ids.contains(&edge.parent_id) {
                    self.active_ids.remove(&edge.suppressed_child_id);
                }
            }
            output.retain(|id| self.active_ids.contains(id));
            self.active_ids.clear();
        }
        Ok(&self.output_by_receiver)
    }

    pub fn candidate_ids(&self, receiver_index: usize) -> Option<&[u32]> {
        self.output_by_receiver
            .get(receiver_index)
            .map(Vec::as_slice)
    }
}

// Shared native support-plane reduction for maximum-corner overlap and
// minimum-corner containment. Keep MINPS operand order for NaNs/zeroes.
fn query_plane_minimum(
    planes: &[[f32; 4]],
    input: [[f32; 4]; 2],
    active: &[usize],
    maximum_corner: bool,
    minimum: f32,
) -> Result<f32, String> {
    if active.iter().any(|&index| index >= planes.len()) {
        return Err("invalid Tome query-bounds plane index".into());
    }
    #[cfg(target_arch = "x86_64")]
    // SAFETY: all selected indices were checked above; x86_64 guarantees SSE.
    return Ok(unsafe {
        simd_predicates::plane_minimum(planes, input, active, maximum_corner, minimum)
    });
    #[cfg(not(target_arch = "x86_64"))]
    Ok(query_plane_minimum_scalar(
        planes,
        input,
        active,
        maximum_corner,
        minimum,
    ))
}

#[cfg(any(test, not(target_arch = "x86_64")))]
fn query_plane_minimum_scalar(
    planes: &[[f32; 4]],
    input: [[f32; 4]; 2],
    active: &[usize],
    maximum_corner: bool,
    mut minimum: f32,
) -> f32 {
    let distance = |index: usize| {
        let plane = planes[index];
        let terms: [f32; 4] = std::array::from_fn(|axis| {
            let upper = (plane[axis] > 0.0) == maximum_corner;
            input[usize::from(upper)][axis] * plane[axis]
        });
        ((terms[1] + terms[0]) + terms[2]) + terms[3]
    };
    let minps = |a: f32, b: f32| if a < b { a } else { b };
    let mut groups = active.chunks_exact(4);
    for group in &mut groups {
        let first = minps(distance(group[0]), distance(group[1]));
        let second = minps(distance(group[2]), distance(group[3]));
        minimum = minps(minimum, minps(first, second));
    }
    for &index in groups.remainder() {
        minimum = minps(minimum, distance(index));
    }
    minimum
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
        .skip_while(|&index| index < 2)
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

    /// FUN_140a85090 reserves one exterior node through 140a84e90 before
    /// seeding any cell, even when no output depth descriptor is requested.
    pub fn new_for_original_query() -> Result<Self, String> {
        let mut pool = Self::new(ORIGINAL_QUERY_WORK_NODE_COUNT)?;
        let exterior = pool.free_head;
        pool.free_head = pool.nodes[exterior as usize].next();
        pool.nodes[exterior as usize].set_next(0);
        Ok(pool)
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

/// Pending per-spatial cell lists and the active list initialized by
/// FUN_140a82340. The slot is the mapped list-head index produced for a
/// spatial record by the query; work-item entry IDs come from the game's
/// separate query-local node pool.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SpatialCellWorklists {
    pending_by_slot: HashMap<usize, VecDeque<CellWorkItem>>,
    pending_entries: Vec<u64>,
    active: CellTraversalWorklist,
    cell_states: Vec<u16>,
}

impl SpatialCellWorklists {
    fn pending_contains_entry(&self, id: u16) -> bool {
        self.pending_entries
            .get(id as usize / 64)
            .is_some_and(|word| word & (1u64 << (id % 64)) != 0)
    }

    fn set_pending_entry(&mut self, id: u16, present: bool) {
        let word = id as usize / 64;
        let mask = 1u64 << (id % 64);
        if present {
            if self.pending_entries.len() <= word {
                self.pending_entries.resize(word + 1, 0);
            }
            self.pending_entries[word] |= mask;
        } else {
            self.pending_entries[word] &= !mask;
            // Keep equality independent of earlier node-allocation history.
            while self.pending_entries.last() == Some(&0) {
                self.pending_entries.pop();
            }
        }
    }

    pub fn new(cell_state_count: usize) -> Self {
        Self {
            pending_by_slot: HashMap::new(),
            pending_entries: Vec::new(),
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
        if self.active.contains_entry(item.entry_id) || self.pending_contains_entry(item.entry_id) {
            return Err("duplicate Tome work-list entry ID".into());
        }
        self.pending_by_slot
            .entry(slot)
            .or_default()
            .push_front(item);
        self.set_pending_entry(item.entry_id, true);
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
        if self.active.contains_entry(item.entry_id)
            || self.active.contains_cell(item.cell_index)
            || self.pending_contains_entry(item.entry_id)
        {
            return Err("duplicate Tome active work-list entry or cell".into());
        }
        self.active.push_front(item);
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
            || self.active.contains_entry(item.entry_id)
        {
            return Err("processed Tome cell cannot be reactivated from this worklist".into());
        }
        if !coverage.propagate_portal(destination, pixels, incoming, path)? {
            return Ok(false);
        }
        pool.reactivate_processed(item, destination, &self.cell_states)?;
        self.active.push_front(item);
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
            self.set_pending_entry(item.entry_id, false);
            self.cell_states[item.cell_index as usize] = item.entry_id | 0x8000;
            self.active.push_back(item);
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
        for item in self.active.entries() {
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
        if let Some(destination) = self.masks.get_mut(&cell) {
            if tile_rect[0] >= destination.rect[0]
                && tile_rect[1] >= destination.rect[1]
                && tile_rect[2] <= destination.rect[2]
                && tile_rect[3] <= destination.rect[3]
            {
                // Both writers validate before changing any bits. With no
                // rectangle growth, update the existing allocation as the
                // original does; errors still leave coverage untouched.
                return match path {
                    PortalCoveragePath::Unclipped => destination.merge_new_coverage(incoming),
                    PortalCoveragePath::Clipped { projected, face } => {
                        destination.rasterize_projected_box(incoming, pixels, projected, face)
                    }
                };
            }
        }
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
#[inline(always)]
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
    projected_link_portal_transition_prepared(
        bytes,
        record_offset,
        link,
        face_code,
        &PortalQueryProjection::new(query_matrix, winding_flip),
        active_planes,
        viewport,
        incoming,
    )
}

fn projected_link_portal_transition_prepared(
    bytes: &[u8],
    record_offset: u32,
    link: [u32; 4],
    face_code: Option<u8>,
    projection: &PortalQueryProjection,
    active_planes: &[usize],
    viewport: [i32; 4],
    incoming: &PortalTileMask,
) -> Result<Option<([i32; 4], PortalCoveragePath)>, String> {
    //a86319 jumps directly to a867ab for sentinel-1, bypassing
    //corner construction and the paired depth predicate entirely.
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
    let (projected, face, box_corners) =
        portal_raster_and_clip_inputs_prepared(bounds, face_code, projection)?;
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
    projection: &PortalQueryProjection,
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
    let Some((pixels, path)) = projected_link_portal_transition_prepared(
        bytes,
        record_offset,
        link,
        face,
        projection,
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
    projection: &PortalQueryProjection,
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
            projection,
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
    let Some((pixels, path)) = projected_link_portal_transition_prepared(
        bytes,
        record_offset,
        link,
        face,
        projection,
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
    projection: &PortalQueryProjection,
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
    let Some((pixels, path)) = projected_link_portal_transition_prepared(
        bytes,
        record_offset,
        link,
        face,
        projection,
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
    projection: &PortalQueryProjection,
    active_planes: &[usize],
    rows: &SpatialCellSlotRows,
    mapper: &mut SpatialCellSlotMapper,
    global_branch_states: &mut [u16],
    worklists: &mut SpatialCellWorklists,
    pool: &mut CellWorkNodePool,
    coverage: &mut VisitedCellCoverage,
    mut exterior: Option<&mut PortalTileMask>,
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
            // FUN_140a85bd0's bit-28 branch merges coverage into +0x1c7e,
            // without scheduling a local cell. Finalization rasterizes it at1.
            if let Some(mask) = exterior.as_deref_mut() {
                if !traversal_link_precheck(bytes, link, direction, gate_states)?.allows_traversal()
                {
                    continue;
                }
                let face = admitted_link_portal_face(
                    bytes,
                    record_offset,
                    link,
                    direction,
                    query_point,
                    point_mode_face,
                )?;
                if let Some((pixels, path)) = projected_link_portal_transition_prepared(
                    bytes,
                    record_offset,
                    link,
                    face,
                    projection,
                    active_planes,
                    viewport,
                    &incoming,
                )? {
                    mask.grow_to_include(portal_tile_rect(pixels)?)?;
                    match path {
                        PortalCoveragePath::Unclipped => {
                            mask.merge_new_coverage(&incoming)?;
                        }
                        PortalCoveragePath::Clipped { projected, face } => {
                            mask.rasterize_projected_box(&incoming, pixels, projected, face)?;
                        }
                    }
                }
            }
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
                projection,
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
                projection,
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
#[derive(Debug, Default, Clone)]
pub struct OrdinaryQueryDiagnostics {
    /// Optional CPU timings: setup, global seed, record setup, objects, cell depth.
    pub cost_ns: [u128; 5],
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
    #[cfg(test)]
    pub processed_object_cells: Vec<[usize; 2]>,
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
    /// FUN_140351780's FOV/LOD scale, passed through the ordinary query's
    /// distance descriptor. Separate from the accurate-occlusion threshold.
    pub object_distance_scale: f32,
    pub winding_flip: bool,
    pub gate_states: Option<&'a [u32]>,
    pub output_capacity: usize,
}

///a81500 floors the configured accurate-occlusion threshold against the
///Tome's +0xc distance divided by min(radius,1). The local shadow query
///supplies MIN_POSITIVE radius, so this can overflow to infinity; retaining
///the ordinary camera's raw CVar changes its spatial-node traversal.
pub fn render_query_global_distance_threshold(
    bytes: &[u8],
    configured: f32,
    radius: f32,
) -> Result<f32, String> {
    let radius = if radius <= 0.0 { 1.0 } else { radius };
    let denominator = if !(radius <= 1.0) { 1.0 } else { radius };
    let floor = float(bytes, 0xc)? / denominator;
    Ok(if floor <= configured {
        configured
    } else {
        floor
    })
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

#[derive(Debug, Clone)]
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
    /// FUN_140a788c0 copies the requested descriptor and installs its inline
    /// grid at+0x590. FUN_140a82630 zeroes4096 floats before any traversal.
    /// This is output only: current cell-object tests still have no input grid.
    pub fn from_input(input: &OrdinaryDirectQueryInput<'_>, viewport: [i32; 4]) -> Self {
        Self {
            render_rows: input.render_rows,
            cropped_rows: input.cropped_rows,
            camera_point: input.camera_point,
            camera_imprecision: input.camera_imprecision,
            winding_flip: input.winding_flip,
            viewport,
            maximum_depth: 0.0,
            depth_grid: Some(Box::new([0.0; 4096])),
        }
    }
}

/// The spatial-instance caller does not consume the area; the descriptor
/// determines whether the native no-grid or depth-grid predicate is used.
pub fn render_query_aabb_visible_without_depth(
    state: Option<&RenderQueryState>,
    bounds: [[f32; 3]; 2],
) -> Result<bool, String> {
    render_query_aabb_without_depth(state, bounds, false).map(|(visible, _)| visible)
}

/// FUN_140a764c0 flag0 / FUN_140a79460. An invalid descriptor accepts
/// without touching the caller's area (None). A requested descriptor uses
/// its generated depth grid; descriptors without one keep the no-grid path.
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
        let (planes, query_bounds) = query_plane_geometry_from_rows(
            state.render_rows,
            state.cropped_rows,
            state.camera_imprecision,
        )?;
        Ok(Self {
            camera_imprecision: state.camera_imprecision,
            query_bounds,
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

    // Cell generation consumes only immutable descriptor math. Do not borrow
    // the output grid that the same worker is still building or clone16KiB.
    fn for_cell_depth(state: &RenderQueryState) -> Result<RenderAabbQuery<'static>, String> {
        let query = RenderAabbQuery::from_state(state)?;
        Ok(RenderAabbQuery {
            camera_imprecision: query.camera_imprecision,
            planes: query.planes,
            query_bounds: query.query_bounds,
            projection_columns: query.projection_columns,
            viewport: query.viewport,
            depth_grid: None,
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

    /// FUN_140a82710's cell-output branch, ending at 0x140a83f72.
    /// This is farthest Z/W, unlike the nearest-depth object predicate.
    /// Record-level planes are reduced against the padded cell before clipping.
    pub fn cell_depth_value(
        &self,
        bounds: [[f32; 4]; 2],
        record_active_planes: &[usize],
    ) -> Result<f32, String> {
        if bounds.iter().flatten().any(|value| !value.is_finite())
            || record_active_planes
                .iter()
                .any(|&index| index >= self.planes.len())
        {
            return Err("invalid Tome cell-depth query".into());
        }
        let active = active_query_plane_positions(&self.planes, bounds, record_active_planes)?;
        let clipped = if active.is_empty() {
            bounds
        } else {
            clip_box_to_active_query_planes(self.query_bounds, bounds, &self.planes, &active)?
        };
        let depth_row = self.projection_columns.map(|row| row[2]);
        let w_row = self.projection_columns.map(|row| row[3]);
        let corner: [f32; 4] = std::array::from_fn(|axis| {
            if depth_row[axis] > 0.0 {
                clipped[1][axis]
            } else {
                clipped[0][axis]
            }
        });
        let dot = |row: [f32; 4]| {
            ((row[1] * corner[1] + row[0] * corner[0]) + row[2] * corner[2]) + row[3] * corner[3]
        };
        let z = dot(depth_row);
        let mut depth = if z > 0.0 { z / dot(w_row) } else { 0.0 };
        if depth > 1.0 {
            depth = 1.0;
        }
        Ok(depth)
    }

    pub fn visibility_and_area(&self, bounds: [[f32; 3]; 2]) -> Result<(bool, f32), String> {
        let expanded = self.expanded_bounds(bounds)?;
        if let Some(grid) = self.depth_grid {
            query_aabb_visibility_with_depth(
                self.query_bounds,
                &self.planes,
                expanded,
                self.projection_columns,
                self.viewport,
                grid,
            )
        } else {
            query_aabb_visibility_without_depth(
                self.query_bounds,
                &self.planes,
                expanded,
                self.projection_columns,
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
    native_projected_corner_bounds(corners, crossing)
}

#[cfg(target_arch = "x86_64")]
fn native_projected_corner_bounds(corners: [[f32; 4]; 8], crossing: bool) -> [f32; 4] {
    use std::arch::x86_64::*;
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
                        if inside_high { projected } else { 1.0 }
                    } else {
                        -1.0
                    },
                    if inside_high {
                        if inside_low { projected } else { -1.0 }
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
    let nearest: [f32; 4] =
        std::array::from_fn(|axis| clipped[usize::from(!(rows[axis][2] > 0.0))][axis]);
    let dot = |component: usize| {
        ((nearest[1] * rows[1][component] + nearest[0] * rows[0][component])
            + nearest[2] * rows[2][component])
            + nearest[3] * rows[3][component]
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
        let bias = [
            32.0,
            32.0,
            f32::from_bits(0x4203_ffff),
            f32::from_bits(0x4203_ffff),
        ];
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
    _: [[f32; 4]; 2],
    _: &[[f32; 4]],
    _: [[f32; 4]; 2],
    _: [[f32; 4]; 4],
    _: [i32; 4],
    _: &[f32; 4096],
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
/// Requested descriptors receive a depth grid; collection traversal and an
/// optional input grid remain unsupported. Failure exposes progress, not partial indices.
pub fn ordinary_direct_tome_render_job(
    tree: &SpatialTree,
    bytes: &[u8],
    data: &DirectTomeQueryData,
    input: &OrdinaryDirectQueryInput<'_>,
    job: u32,
    count: u32,
) -> RenderQueryJob {
    ordinary_direct_tome_render_job_inner(tree, bytes, Some(data), input, job, count)
}

pub fn ordinary_direct_tome_render_job_uncached(
    tree: &SpatialTree,
    bytes: &[u8],
    input: &OrdinaryDirectQueryInput<'_>,
    job: u32,
    count: u32,
) -> RenderQueryJob {
    ordinary_direct_tome_render_job_inner(tree, bytes, None, input, job, count)
}

fn ordinary_direct_tome_render_job_inner(
    tree: &SpatialTree,
    bytes: &[u8],
    data: Option<&DirectTomeQueryData>,
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
        ordinary_direct_tome_visible_objects_inner(
            tree,
            bytes,
            &job_input,
            data,
            &mut diagnostics.progress,
            state.as_mut(),
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
#[derive(Default, Clone)]
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
    ordinary_direct_tome_visible_objects_inner(tree, bytes, input, None, diagnostics, None)
}

pub fn ordinary_direct_tome_visible_objects_cached_with_diagnostics(
    tree: &SpatialTree,
    bytes: &[u8],
    data: &DirectTomeQueryData,
    input: &OrdinaryDirectQueryInput<'_>,
    diagnostics: &mut OrdinaryQueryDiagnostics,
) -> Result<Option<VisibleObjectIndexOutput>, String> {
    ordinary_direct_tome_visible_objects_inner(tree, bytes, input, Some(data), diagnostics, None)
}

fn ordinary_direct_tome_visible_objects_inner(
    tree: &SpatialTree,
    bytes: &[u8],
    input: &OrdinaryDirectQueryInput<'_>,
    data: Option<&DirectTomeQueryData>,
    diagnostics: &mut OrdinaryQueryDiagnostics,
    mut output_state: Option<&mut RenderQueryState>,
) -> Result<Option<VisibleObjectIndexOutput>, String> {
    *diagnostics = OrdinaryQueryDiagnostics::default();
    let profile_cost = std::env::var_os("DH2_PROFILE_TOME_QUERY").is_some();
    let setup_started = profile_cost.then(std::time::Instant::now);
    diagnostics.stage = "setup";
    //a77560 normalizes the first depth plane before either the global
    //selection or cell-object clipping. Keep the same prepared planes used
    //by the descriptor; unnormalized arithmetic changes near-plane edges.
    let (planes, object_query_bounds) = query_plane_geometry_from_rows(
        input.render_rows,
        input.cropped_rows,
        input.camera_imprecision,
    )?;
    let stored_matrix = stored_query_matrix_from_render_rows(input.render_rows);
    let portal_projection = PortalQueryProjection::new(stored_matrix, input.winding_flip);
    let initial_mask = PortalTileMask::full(input.initial_tile_rect)?;
    let depth_query = output_state
        .as_deref()
        .filter(|state| state.depth_grid.is_some())
        .map(RenderAabbQuery::for_cell_depth)
        .transpose()?;
    let mut exterior_mask = if depth_query.is_some() {
        Some(PortalTileMask::new(input.initial_tile_rect)?)
    } else {
        None
    };
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
    if let Some(started) = setup_started {
        diagnostics.cost_ns[0] += started.elapsed().as_nanos();
    }
    let seed_started = profile_cost.then(std::time::Instant::now);
    diagnostics.stage = "global_seed";
    let Some(mut global) = tree.seed_ordinary_camera_global_query(
        bytes,
        data,
        input.camera_point,
        &planes,
        input.camera_imprecision,
        render_query_global_distance_threshold(
            bytes,
            input.global_distance_threshold,
            input.object_distance_scale,
        )?,
        &mut worklists,
        &mut pool,
        &mut coverage,
        &initial_mask,
    )?
    else {
        diagnostics.stage = "outside_tome_root";
        return Err("Tome query input outside root bounds (error 5)".into());
    };
    if let Some(started) = seed_started {
        diagnostics.cost_ns[1] += started.elapsed().as_nanos();
    }
    let setup_started = profile_cost.then(std::time::Instant::now);
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
    if let Some(started) = setup_started {
        diagnostics.cost_ns[0] += started.elapsed().as_nanos();
    }
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
        let record_started = profile_cost.then(std::time::Instant::now);
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
        let record_padding = float(bytes, record_offset as usize + 0x30)?;
        diagnostics.stage = "cell_traversal";
        let direction = LinkQueryDirection::Volume {
            center: setup.link_center,
            tolerance: setup.link_tolerance,
        };
        if let Some(started) = record_started {
            diagnostics.cost_ns[2] += started.elapsed().as_nanos();
        }
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
            #[cfg(test)]
            diagnostics
                .processed_object_cells
                .push([owner.spatial_index, item.cell_index as usize]);
            let query = CellObjectVisibilityQuery {
                //a82710 reads descriptor+530/+540, the finite frustum box.
                //The spatial-record box selects active planes only.
                record_query_bounds: object_query_bounds,
                query_planes: &planes,
                active_planes: &setup.active_planes,
                projection_rows: stored_matrix,
                camera_translation_imprecision: input.object_imprecision,
                object_distance_point: input.camera_point,
                object_distance_scale_squared: [if input.object_distance_scale <= 0.0 {
                    1.0
                } else {
                    input.object_distance_scale * input.object_distance_scale
                }; 4],
                minimum_area_metric: input.minimum_area_metric,
                viewport_tile_bounds: portal.rect,
                portal,
                depth_grid: None,
                //a81500 initializes+1c84 to0; a85090 only derives a
                //perspective near offset when query flags bit1 is set.
                //The ordinary37e1d0/a73820 caller passes flags0.
                point_occlusion_near_offset: Some(0.0),
            };
            let object_started = profile_cost.then(std::time::Instant::now);
            let result = cells.append_visible_object_candidates(
                bytes,
                item.cell_index as usize,
                &query,
                &mut output,
            );
            if let Some(started) = object_started {
                diagnostics.cost_ns[3] += started.elapsed().as_nanos();
            }
            let result = result.and_then(|()| {
                let depth_started = profile_cost.then(std::time::Instant::now);
                let result = (|| {
                    // 140a82710 emits cell depth after objects, before cells.
                    if let (Some(depth_query), Some(state)) =
                        (depth_query.as_ref(), output_state.as_deref_mut())
                    {
                        let bounds = cells.cell_depth_bounds(
                            bytes,
                            item.cell_index as usize,
                            record_padding,
                            input.camera_imprecision,
                        )?;
                        let value = depth_query.cell_depth_value(bounds, &setup.active_planes)?;
                        if let Some(grid) = state.depth_grid.as_deref_mut() {
                            state.maximum_depth = rasterize_depth_mask(
                                grid,
                                portal.rect,
                                &portal.bits,
                                value,
                                state.maximum_depth,
                            )?;
                        }
                    }
                    cells.append_visible_cell_indices(
                        bytes,
                        item.cell_index as usize,
                        None,
                        output.cells.as_mut(),
                    )
                })();
                if let Some(started) = depth_started {
                    diagnostics.cost_ns[4] += started.elapsed().as_nanos();
                }
                result
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
                &portal_projection,
                &setup.active_planes,
                rows,
                &mut mapper,
                &mut global.branch_states,
                &mut worklists,
                &mut pool,
                &mut coverage,
                exterior_mask.as_mut(),
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
    if let (Some(mask), Some(state)) = (exterior_mask.as_ref(), output_state.as_deref_mut()) {
        if let Some(grid) = state.depth_grid.as_deref_mut() {
            // 140a85bd0 flushes every delayed cell before the exterior mask.
            // Ordinary world jobs have no input grid to min-clamp against.
            if mask.bits.iter().any(|&word| word != 0) {
                state.maximum_depth =
                    rasterize_depth_mask(grid, mask.rect, &mask.bits, 1.0, state.maximum_depth)?;
            }
        }
    }
    diagnostics.stage = "complete";
    diagnostics.accepted_objects = output.count();
    diagnostics.distance_rejected_objects = output.distance_rejected_objects;
    Ok(Some(output))
}

/// FUN_140a85bd0 expands a packed portal rectangle into world coordinates
/// using the record's bounds and +0x30 padding. Direct Tome initialization
/// supplies zero additional offset. This is the geometry supplied to later
/// clipping, not a visibility decision by itself.
#[inline(always)]
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
#[inline(always)]
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

/// Original global Tome header bounds consumed by receiver initialization.
/// These include the full level, independently of the current visible cells.
pub fn global_bounds(bytes: &[u8]) -> Result<[[f32; 3]; 2], String> {
    recorded_bounds(bytes, 0x14)
}

mod trees;
use trees::recorded_bounds;
pub use trees::render_query_job_quad;

mod portal_projection;
use portal_projection::portal_raster_and_clip_inputs_prepared;
pub use portal_projection::{
    PortalQueryProjection, depth_grid_intersects_pixels, portal_box_depth_mask,
    portal_face_vectors_from_query_matrix, portal_raster_inputs_from_bounds, portal_tile_rect,
    projected_portal_box_bounds, projected_portal_box_crossing_bounds,
    projected_portal_box_fallback_bounds, projected_portal_box_pixel_rect,
    projected_portal_pixel_rect, rasterize_depth_mask, transform_portal_corner_by_query_matrix,
};

mod query_geometry;
#[cfg(test)]
use query_geometry::segment_intersects_aabb;
pub use query_geometry::{
    ORIGINAL_DEFAULT_QUERY_JOB_COUNT, ORIGINAL_DEFAULT_SCHEDULER_THREADING_MODE,
    ORIGINAL_VISIBLE_CELL_OUTPUT_CAPACITY, ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
    original_render_fov_radians, original_render_perspective_rows, original_render_query_matrix,
    original_render_vertical_fov_radians, original_world_query_job_count,
    original_world_visibility_query_matrix, point_triangle_distance_squared, quad_intersects_aabb,
    query_matrix_winding_flip, render_query_base_rows, render_query_job_tile,
    sphere_intersects_portal, stored_query_matrix_from_render_rows,
};

#[cfg(test)]
mod tests;
