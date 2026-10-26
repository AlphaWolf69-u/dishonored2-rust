//! Retained query geometry, with references local to one physics asset.
//! Decoding is separate from body admission, transforms and hit collection.
use super::{compound::CompoundShape, polytope::ConvexPolytope, shape::ShapeHeader, PhysWorld};
use std::collections::BTreeMap;

#[derive(Debug)]
pub enum QueryGeometry {
    Compound(CompoundShape),
    Polytope(ConvexPolytope),
}

#[derive(Debug)]
pub struct QueryShape {
    pub header: ShapeHeader,
    pub geometry: QueryGeometry,
}

#[derive(Debug)]
pub struct QueryShapes {
    root: usize,
    shapes: BTreeMap<usize, QueryShape>,
}

impl QueryShapes {
    pub fn root(&self) -> usize {
        self.root
    }

    pub fn get(&self, item: usize) -> Result<&QueryShape, String> {
        self.shapes
            .get(&item)
            .ok_or_else(|| format!("query shape item {item} was not retained"))
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &QueryShape)> {
        self.shapes.iter().map(|(&item, shape)| (item, shape))
    }

    /// Actual calcAabb dispatch: compounds transform their cached local bounds;
    /// convex shapes transform vertices and then include the convex radius.
    pub fn bounds(
        &self,
        item: usize,
        frame: crate::gameplay::shape_transform::ShapeMatrix,
    ) -> Result<[[f32; 4]; 2], String> {
        Ok(match &self.get(item)?.geometry {
            QueryGeometry::Compound(shape) => {
                crate::gameplay::shape_transform::transform_bounds(frame, shape.bounds)
            }
            QueryGeometry::Polytope(shape) => {
                crate::gameplay::convex_support::convex_bounds(&shape.vertices, frame, shape.radius)
            }
        })
    }
}

impl PhysWorld {
    /// Decode each reachable shape once. Keep the authored trees, instance
    /// indices, tags and connectivity; do not turn geometry into volume hits.
    /// Compound free-list slots without enabled bit64 are not shape references.
    pub fn query_shapes(&self, bytes: &[u8], root: usize) -> Result<QueryShapes, String> {
        let mut shapes = BTreeMap::new();
        let mut pending = vec![root];
        while let Some(index) = pending.pop() {
            if shapes.contains_key(&index) {
                continue;
            }
            let item = self.items.get(index).ok_or("query shape item missing")?;
            let kind = &self.types[item.type_index].name;
            let header = self.shape_header(bytes, index)?;
            let geometry = match kind.as_str() {
                "hknpCompoundShape" => {
                    let compound = self.compound_shape(bytes, index)?;
                    pending.extend(
                        compound
                            .instances
                            .iter()
                            .filter(|instance| instance.flags & 64 != 0)
                            .map(|instance| instance.shape),
                    );
                    QueryGeometry::Compound(compound)
                }
                "hknpConvexPolytopeShape" | "hknpBoxShape" => {
                    QueryGeometry::Polytope(self.convex_polytope(bytes, index)?)
                }
                _ => return Err(format!("query geometry for {kind} is not implemented")),
            };
            shapes.insert(index, QueryShape { header, geometry });
        }
        Ok(QueryShapes { root, shapes })
    }
}

#[cfg(test)]
#[path = "query_geometry/point_tests.rs"]
mod point_tests;
#[cfg(test)]
#[path = "query_geometry/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "query_geometry/world_tests.rs"]
mod world_tests;
