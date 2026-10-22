//! Closest-point handler registration/inheritance (cbb830/cc22c0) and filtered
//! pair selection (cbc1f0). The selected geometry kernels are separate owners.
use super::collision_filter::CollisionQueryFilter;

pub const SHAPE_TYPE_COUNT: usize = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ClosestPointHandler {
    NoContact,
    ConvexConvex,
    ConvexMesh,
    MeshConvex,
    ConvexExternalMesh,
    ExternalMeshConvex,
    ConvexCompound,
    CompoundConvex,
    ConvexHeightfield,
    HeightfieldConvex,
    ExternalMeshShape,
    ShapeExternalMesh,
    MeshMesh,
    HeightfieldHeightfield,
    HeightfieldMesh,
    MeshHeightfield,
    CompoundShape,
    ShapeCompound,
    CompoundCompound,
    ScaleQuery,
    ScaleTarget,
    UnwrapMaskedQuery,
    UnwrapMaskedTarget,
    LodQuery,
    LodTarget,
    UnwrapBoth,
    LodBoth,
    ScaleBoth,
}

#[derive(Clone, Copy, Debug)]
pub struct CollisionDispatchEntry {
    pub handler: ClosestPointHandler,
    pub depth: u8,
    pub registered_query: u8,
    pub registered_target: u8,
}

pub struct ClosestPointDispatch {
    entries: [CollisionDispatchEntry; SHAPE_TYPE_COUNT * SHAPE_TYPE_COUNT],
    strict: bool,
}

/// cbc4c0's dispatch inheritance, distinct from an entity class hierarchy.
pub fn shape_parent(shape: u8) -> u8 {
    match shape {
        1..=6 => 0,
        8..=10 => 7,
        12 => 11,
        14 => 13,
        17 => 16,
        _ => 26,
    }
}

impl ClosestPointDispatch {
    pub fn new(enabled_types: u32) -> Result<Self, String> {
        use ClosestPointHandler::*;
        let mut result = Self {
            entries: [CollisionDispatchEntry {
                handler: NoContact,
                depth: 100,
                registered_query: 26,
                registered_target: 26,
            }; SHAPE_TYPE_COUNT * SHAPE_TYPE_COUNT],
            strict: true,
        };
        for (a, b, handler) in [
            (0, 0, ConvexConvex),
            (0, 8, ConvexMesh),
            (8, 0, MeshConvex),
            (0, 9, ConvexExternalMesh),
            (9, 0, ExternalMeshConvex),
            (0, 12, ConvexCompound),
            (12, 0, CompoundConvex),
            (0, 14, ConvexHeightfield),
            (14, 0, HeightfieldConvex),
            (9, 7, ExternalMeshShape),
            (7, 9, ShapeExternalMesh),
            (9, 13, ExternalMeshShape),
            (13, 9, ShapeExternalMesh),
            (8, 8, MeshMesh),
            (14, 14, HeightfieldHeightfield),
            (14, 8, HeightfieldMesh),
            (8, 14, MeshHeightfield),
            (11, 7, CompoundShape),
            (7, 11, ShapeCompound),
            (11, 13, CompoundShape),
            (13, 11, ShapeCompound),
            (11, 11, CompoundCompound),
        ] {
            result.register(a, b, handler, enabled_types)?;
        }
        result.strict = false;
        for other in 0..SHAPE_TYPE_COUNT as u8 {
            for (a, b, handler) in [
                (15, other, ScaleQuery),
                (other, 15, ScaleTarget),
                (16, other, UnwrapMaskedQuery),
                (other, 16, UnwrapMaskedTarget),
                (18, other, LodQuery),
                (other, 18, LodTarget),
            ] {
                result.register(a, b, handler, enabled_types)?;
            }
        }
        for (a, b, handler) in [(16, 16, UnwrapBoth), (18, 18, LodBoth), (15, 15, ScaleBoth)] {
            result.register(a, b, handler, enabled_types)?;
        }
        result.strict = true;
        Ok(result)
    }

    fn register(
        &mut self,
        a: u8,
        b: u8,
        handler: ClosestPointHandler,
        enabled: u32,
    ) -> Result<(), String> {
        let mask = (1u32 << a) | (1u32 << b);
        if mask & enabled == mask {
            self.register_inherited(a, b, handler, enabled, 0)?;
        }
        Ok(())
    }

    fn register_inherited(
        &mut self,
        a: u8,
        b: u8,
        handler: ClosestPointHandler,
        enabled: u32,
        depth: u8,
    ) -> Result<(), String> {
        for child in 0..SHAPE_TYPE_COUNT as u8 {
            if shape_parent(child) == a {
                self.register_inherited(child, b, handler, enabled, depth + 1)?;
            }
            if shape_parent(child) == b {
                self.register_inherited(a, child, handler, enabled, depth + 1)?;
            }
        }
        let mask = (1u32 << a) | (1u32 << b);
        if mask & enabled == mask {
            let entry = &mut self.entries[a as usize * SHAPE_TYPE_COUNT + b as usize];
            if self.strict && entry.depth < depth {
                return Err(
                    "closest-point registration overrides a more specialized handler".into(),
                );
            }
            *entry = CollisionDispatchEntry {
                handler,
                depth,
                registered_query: a,
                registered_target: b,
            };
        }
        Ok(())
    }

    pub fn entry(&self, query: u8, target: u8) -> Result<&CollisionDispatchEntry, String> {
        if query as usize >= SHAPE_TYPE_COUNT || target as usize >= SHAPE_TYPE_COUNT {
            return Err("closest-point shape type out of bounds".into());
        }
        Ok(&self.entries[query as usize * SHAPE_TYPE_COUNT + target as usize])
    }

    /// NoContact is the original empty default handler or a rejected filter;
    /// it must not be substituted for an unimplemented selected kernel.
    pub fn select(
        &self,
        query: u8,
        target: u8,
        filter: Option<CollisionQueryFilter>,
        filter_info: [u32; 2],
        flipped: bool,
    ) -> Result<ClosestPointHandler, String> {
        if filter.is_some_and(|f| !f.accepts_pair(filter_info[0], filter_info[1], !flipped)) {
            return Ok(ClosestPointHandler::NoContact);
        }
        Ok(self.entry(query, target)?.handler)
    }
}

#[cfg(test)]
#[path = "collision_dispatch/tests.rs"]
mod tests;
