//! Trees.
use super::shadow_caster;
use super::{
    crop_query_rows, direct_spatial_link_target, expand_object_bounds_for_camera_imprecision,
    first_depth_quad, float, global_node_overlaps_query_planes, object_bounds,
    object_distance_bounds, object_distance_rejection_mask, ordinary_spatial_query_volume,
    original_query_accepts_tome_root, project_clipped_query_aabb, quad_intersects_aabb,
    quantized_cell_query_bounds, query_bounds_accepts_aabb, query_planes_from_rows,
    ready_same_spatial_cell_target, render_query_job_tile, word, write_u32,
    CellObjectVisibilityQuery, CellSelection, CellTraversalWorklist, CellTree, CellWorkItem,
    CellWorkNodePool, DirectLinkDestination, DirectTomeQueryData, InitialCameraCellSeed,
    LinkQueryDirection, OrdinaryGlobalCameraQuery, OrdinarySpatialQueryVolume,
    OriginalSpatialQueryVolume, PortalTileMask, ReadyCellTarget, SpatialCellWorklists,
    SpatialQueryBranchCandidate, SpatialQueryBranchStack, SpatialTree, TraversalCellId,
    VisibleCellIndexOutput, VisibleObjectIndexOutput, VisitedCellCoverage,
    ORIGINAL_OBJECT_QUERY_BATCH_SIZE, ORIGINAL_QUERY_CELL_STATE_COUNT,
};

pub(super) fn recorded_bounds(bytes: &[u8], at: usize) -> Result<[[f32; 3]; 2], String> {
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
            || global_distance_threshold.is_nan()
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
            cell_bounds_root: root,
            cell_table_at: cell_table,
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

    pub(super) fn cell_depth_bounds(
        &self,
        bytes: &[u8],
        index: usize,
        padding: f32,
        imprecision: [f32; 3],
    ) -> Result<[[f32; 4]; 2], String> {
        if index >= self.cell_count() {
            return Err("Tome cell-depth index exceeds cell table".into());
        }
        let at = self.cell_table_at + index * 0x24;
        let packed = [
            word(bytes, at + 0x18)?,
            word(bytes, at + 0x1c)?,
            word(bytes, at + 0x20)?,
        ];
        let quantized = [
            [
                (packed[0] >> 16) as u16,
                packed[0] as u16,
                (packed[1] >> 16) as u16,
            ],
            [packed[1] as u16, (packed[2] >> 16) as u16, packed[2] as u16],
        ];
        quantized_cell_query_bounds(self.cell_bounds_root, quantized, padding, imprecision)
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
                //a82710 rejects against active planes, then projects the
                //expanded authored box unchanged. Receiver-box clipping is
                //a separate path and would shrink valid object coverage.
                if !query_bounds_accepts_aabb(
                    query.record_query_bounds,
                    input_bounds,
                    query.query_planes,
                    query.active_planes,
                )? {
                    continue;
                }
                if let Some(near_offset) = query.point_occlusion_near_offset {
                    if !shadow_caster::native_point_occlusion_visible(
                        bytes,
                        object_index,
                        query.object_distance_point,
                        near_offset,
                    )? {
                        output.seen[object_index as usize / 32] |= 1 << (object_index % 32);
                        continue;
                    }
                }
                //a82710 uses the same reciprocal/corner mask projection as
                //a79460's no-grid branch, including boxes crossing W=0.
                let projected =
                    project_clipped_query_aabb(input_bounds, query.projection_rows, true);
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

    pub(super) fn for_each_traversal_record_while(
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
    pub(super) fn resolve_portal_code(
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
