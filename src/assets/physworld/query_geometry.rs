//! Retained query geometry, with references local to one physics asset.
//! Decoding is separate from body admission, transforms and hit collection.
use super::{
    compound::CompoundShape,
    polytope::{ConvexPolytope, ConvexShape},
    shape::ShapeHeader,
    PhysWorld,
};
use std::collections::BTreeMap;
pub(crate) mod alias;

#[derive(Debug)]
pub enum QueryGeometry {
    Compound(CompoundShape),
    Polytope(ConvexPolytope),
    Convex(ConvexShape),
    CompressedMesh(super::compressed_mesh::CompressedMesh),
}
impl QueryGeometry {
    pub(crate) fn convex_radius(&self) -> Option<f32> {
        match self {
            Self::Convex(shape) => Some(shape.radius),
            Self::Polytope(shape) => Some(shape.radius),
            _ => None,
        }
    }
    pub(crate) fn convex_vertices(&self) -> Option<&[[f32; 4]]> {
        match self {
            Self::Convex(shape) => Some(&shape.vertices),
            Self::Polytope(shape) => Some(&shape.vertices),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct QueryShape {
    pub header: ShapeHeader,
    pub geometry: QueryGeometry,
}

pub struct QueryShapes {
    root: usize,
    shapes: BTreeMap<usize, QueryShape>,
    // Present only when the tree's resource identities refer to standalone
    // source/wrapper records. Geometry leases must not outlive those records.
    resources: Option<Box<ResourceLeases>>,
}

#[derive(Default)]
struct ResourceLeases {
    sources: Vec<std::sync::Arc<super::source::ShapeSource>>,
    aliases: Vec<std::sync::Arc<super::source::ShapeAlias>>,
}

impl std::fmt::Debug for QueryShapes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Retain the geometry-only diagnostic representation. Leases have no
        // effect on shape fingerprints, and do not print source byte buffers.
        f.debug_struct("QueryShapes")
            .field("root", &self.root)
            .field("shapes", &self.shapes)
            .finish()
    }
}

impl QueryShapes {
    pub(crate) fn retain_source(&mut self, source: std::sync::Arc<super::source::ShapeSource>) {
        self.resources
            .get_or_insert_with(Default::default)
            .sources
            .push(source);
    }

    pub(crate) fn retain_aliases(&mut self, aliases: &[std::sync::Arc<super::source::ShapeAlias>]) {
        if aliases.is_empty() {
            return;
        }
        self.resources
            .get_or_insert_with(Default::default)
            .aliases
            .extend_from_slice(aliases);
    }

    fn absorb_resources(&mut self, other: &mut Self) {
        let Some(mut resources) = other.resources.take() else {
            return;
        };
        match &mut self.resources {
            None => self.resources = Some(resources),
            Some(own) => {
                own.sources.append(&mut resources.sources);
                own.aliases.append(&mut resources.aliases);
            }
        }
    }

    pub(crate) fn set_root_resource_identity(&mut self, identity: u64) -> Result<(), String> {
        self.shapes
            .get_mut(&self.root)
            .ok_or("root query shape is missing")?
            .header
            .user_data = identity;
        Ok(())
    }

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

    /// A scaled HKX redirect (`standalone_hkx_alias`): the uniform factor
    /// applies to the base resource's vertices, plane distances, convex
    /// radius, compound child translations and bounds.
    pub fn scaled(mut self, factor: f32) -> Self {
        for shape in self.shapes.values_mut() {
            match &mut shape.geometry {
                QueryGeometry::Convex(convex) => {
                    for vertex in &mut convex.vertices {
                        for axis in 0..3 {
                            vertex[axis] *= factor;
                        }
                    }
                    convex.radius *= factor;
                    shape.header.radius = convex.radius;
                }
                QueryGeometry::Polytope(polytope) => {
                    for vertex in &mut polytope.vertices {
                        for axis in 0..3 {
                            vertex[axis] *= factor;
                        }
                    }
                    for plane in &mut polytope.planes {
                        plane[3] *= factor;
                    }
                    polytope.radius *= factor;
                }
                QueryGeometry::Compound(compound) => {
                    for instance in &mut compound.instances {
                        for axis in 0..3 {
                            instance.transform[3][axis] *= factor;
                        }
                    }
                    for bound in &mut compound.bounds {
                        for axis in 0..3 {
                            bound[axis] *= factor;
                        }
                    }
                }
                QueryGeometry::CompressedMesh(mesh) => mesh.scale *= factor,
            }
        }
        self
    }

    /// Actual calcAabb dispatch: compounds transform their cached local bounds;
    /// convex shapes transform vertices and then include the convex radius.
    pub fn bounds(
        &self,
        item: usize,
        frame: crate::gameplay::shape_transform::ShapeMatrix,
    ) -> Result<[[f32; 4]; 2], String> {
        Ok(match &self.get(item)?.geometry {
            QueryGeometry::Convex(shape) => {
                crate::gameplay::convex_support::convex_bounds(&shape.vertices, frame, shape.radius)
            }
            QueryGeometry::Compound(shape) => {
                crate::gameplay::shape_transform::transform_bounds(frame, shape.bounds)
            }
            QueryGeometry::Polytope(shape) => {
                crate::gameplay::convex_support::convex_bounds(&shape.vertices, frame, shape.radius)
            }
            QueryGeometry::CompressedMesh(shape) => {
                crate::gameplay::shape_transform::transform_bounds(frame, shape.bounds())
            }
        })
    }
}

impl PhysWorld {
    /// Decode each reachable shape once. Keep the authored trees, instance
    /// indices, tags and connectivity; do not turn geometry into volume hits.
    /// Compound free-list slots without enabled bit64 are not shape references.
    pub fn query_shapes(&self, bytes: &[u8], root: usize) -> Result<QueryShapes, String> {
        self.query_shapes_inner(bytes, root, None)
    }

    pub(crate) fn query_shapes_retained(
        &self,
        source: &std::sync::Arc<super::source::ShapeSource>,
        root: usize,
    ) -> Result<QueryShapes, String> {
        self.query_shapes_inner(&source.bytes, root, Some(source))
    }

    fn query_shapes_inner(
        &self,
        bytes: &[u8],
        root: usize,
        source: Option<&std::sync::Arc<super::source::ShapeSource>>,
    ) -> Result<QueryShapes, String> {
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
                "hknpConvexShape" => QueryGeometry::Convex(self.convex_shape(bytes, index)?),
                "hknpCompressedMeshShape" => {
                    QueryGeometry::CompressedMesh(super::compressed_mesh::CompressedMesh::new(
                        source
                            .ok_or("compressed mesh queries require their source lease")?
                            .clone(),
                        index,
                    )?)
                }
                _ => return Err(format!("query geometry for {kind} is not implemented")),
            };
            shapes.insert(index, QueryShape { header, geometry });
        }
        Ok(QueryShapes {
            root,
            shapes,
            resources: None,
        })
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
