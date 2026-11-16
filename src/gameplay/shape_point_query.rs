//! Zero-distance all-hit shape queries used by camera environment contacts.
//! The world owns candidate order, body admission and the caller's hit packets.
use super::{
    closest_point_query::{prepare_transforms, PointSphere},
    collision_dispatch::{ClosestPointDispatch, ClosestPointHandler},
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    compound_query::{
        instance::{prepare_instance, CompoundChildGeometry, CompoundShapeState, ShapeKey},
        prepare::{prepare, CompoundQueryTransform},
        CompoundQuery, CompoundQueryVisitor, CompoundTree,
    },
    convex_support::{
        distance::{query_point, ConvexClosestPoint},
        ConvexSupportInput, ConvexSupportScale, ConvexSupportWorkspace,
    },
    polytope_query::zero_distance_point_geometry,
    shape_tags::{decode_shape_tag, ShapeTagSource},
    shape_transform::{transform_bounds, transform_point, ScaledShapeTransform, ShapeMatrix},
};
use crate::assets::physworld::query_geometry::{QueryGeometry, QueryShapes};

#[path = "shape_point_query/mesh.rs"]
mod mesh;

const IDENTITY: ShapeMatrix = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

pub struct PointQueryTarget<'a> {
    pub shapes: &'a QueryShapes,
    pub state: CompoundShapeState,
    pub hit: ShapeHitInfo,
    pub materials: Option<&'a dyn ShapeTagSource>,
}

/// The environment caller consumes body identity/material/key rather than
/// closest-point witness geometry. Keep native leaf order and duplicate hits.
#[derive(Clone, Copy, Debug)]
pub struct ShapePointContact {
    pub target: ShapeHitInfo,
}

pub struct ShapePointQuery {
    dispatch: ClosestPointDispatch,
    traversals: Vec<CompoundQuery>,
    active: Vec<usize>,
    support: ConvexSupportWorkspace,
    mesh: super::compressed_mesh_query::CompressedMeshQuery,
}
impl ShapePointQuery {
    pub fn new(enabled_shape_types: u32) -> Result<Self, String> {
        Ok(Self {
            dispatch: ClosestPointDispatch::new(enabled_shape_types)?,
            traversals: Vec::new(),
            active: Vec::new(),
            support: ConvexSupportWorkspace::default(),
            mesh: Default::default(),
        })
    }

    /// Append the all-hit projection; never replace world broadphase ordering
    /// with asset order or accept a broadphase box as an environment contact.
    pub fn all_hits(
        &mut self,
        target: PointQueryTarget<'_>,
        point: [f32; 4],
        query: ShapeHitInfo,
        filter: Option<CollisionQueryFilter>,
        contacts: &mut Vec<ShapePointContact>,
    ) -> Result<(), String> {
        self.hits_with_distance(target, point, query, filter, &mut |contact, _| {
            contacts.push(contact);
        })
    }

    /// Execute the same shape traversal while retaining the signed distance
    /// needed by closest-hit callers. Metadata-only callers use all_hits.
    pub fn hits_with_distance(
        &mut self,
        target: PointQueryTarget<'_>,
        point: [f32; 4],
        query: ShapeHitInfo,
        filter: Option<CollisionQueryFilter>,
        submit: &mut dyn FnMut(ShapePointContact, f32),
    ) -> Result<(), String> {
        self.hits_with_geometry(target, point, query, filter, &mut |contact, geometry| {
            submit(contact, geometry.distance[0])
        })
    }

    /// The same traversal and collector order with actual world witness data.
    pub fn hits_with_geometry(
        &mut self,
        target: PointQueryTarget<'_>,
        point: [f32; 4],
        query: ShapeHitInfo,
        filter: Option<CollisionQueryFilter>,
        submit: &mut dyn FnMut(ShapePointContact, ConvexClosestPoint),
    ) -> Result<(), String> {
        let root = target.shapes.get(target.shapes.root())?;
        let bounds = match &root.geometry {
            QueryGeometry::Compound(shape) => {
                transform_bounds(target.state.transform.world_from_local, shape.bounds)
            }
            QueryGeometry::CompressedMesh(shape) => transform_bounds(
                target.state.transform.world_from_local,
                shape.bounds().map(|v| [v[0], v[1], v[2], 0.]),
            ),
            QueryGeometry::Polytope(shape) => super::convex_support::convex_bounds(
                &shape.vertices,
                target.state.transform.world_from_local,
                shape.radius,
            ),
        };
        let Some(mut sphere) = PointSphere::for_query(point, 0., bounds) else {
            return Ok(());
        };
        let relative = prepare_transforms(
            IDENTITY,
            target.state.transform.world_from_local,
            0.,
            &mut [0.; 4],
        );
        self.active.clear();
        self.visit(
            &target,
            target.shapes.root(),
            target.state,
            relative,
            ShapeKey {
                value: target.hit.shape_key,
                bits: 0,
            },
            target.hit,
            &mut sphere,
            query,
            filter,
            submit,
        )
    }

    fn visit(
        &mut self,
        target: &PointQueryTarget<'_>,
        item: usize,
        state: CompoundShapeState,
        relative: ShapeMatrix,
        key: ShapeKey,
        hit: ShapeHitInfo,
        sphere: &mut PointSphere,
        query: ShapeHitInfo,
        filter: Option<CollisionQueryFilter>,
        submit: &mut dyn FnMut(ShapePointContact, ConvexClosestPoint),
    ) -> Result<(), String> {
        let shape = target.shapes.get(item)?;
        let handler = self.dispatch.select(
            2,
            shape.header.shape_type,
            filter,
            [query.filter_info, hit.filter_info],
            false,
        )?;
        if handler == ClosestPointHandler::NoContact {
            return Ok(());
        }
        if self.active.contains(&item) {
            return Err("cyclic compound query geometry".into());
        }
        let depth = self.active.len();
        self.active.push(item);
        let result: Result<(), String> = (|| match (&shape.geometry, handler) {
            (QueryGeometry::Polytope(polytope), ClosestPointHandler::ConvexConvex) => {
                let geometry = if !state.scaled && polytope.connectivity.is_some() {
                    let local = transform_point(relative, sphere.vertex);
                    if polytope.radius != 0.
                        || !local.iter().all(|v| v.is_finite())
                        || !polytope.planes.iter().flatten().all(|v| v.is_finite())
                    {
                        return Err(
                            "specialized nonfinite/radius polytope point query is not implemented"
                                .into(),
                        );
                    }
                    zero_distance_point_geometry(polytope, local)?
                } else {
                    let prepared = self.support.prepare(ConvexSupportInput {
                        shape_type: shape.header.shape_type,
                        vertices: &polytope.vertices,
                        radius: polytope.radius,
                        scaled: state.scaled.then_some(ConvexSupportScale {
                            scale: [
                                state.transform.scale[0],
                                state.transform.scale[1],
                                state.transform.scale[2],
                            ],
                            offset: [
                                state.transform.offset[0],
                                state.transform.offset[1],
                                state.transform.offset[2],
                            ],
                            radius: state.radius,
                        }),
                    })?;
                    query_point(
                        sphere.vertex,
                        prepared.vertices,
                        relative,
                        sphere.radius,
                        prepared.radius,
                        0.,
                    )?
                };
                if let Some(geometry) = geometry {
                    submit(
                        ShapePointContact { target: hit },
                        ConvexClosestPoint {
                            point: transform_point(
                                state.transform.world_from_local,
                                geometry.point,
                            ),
                            normal: super::shape_transform::rotate_vector(
                                state.transform.world_from_local,
                                geometry.normal,
                            ),
                            distance: geometry.distance,
                        },
                    );
                }
                Ok(())
            }
            (QueryGeometry::CompressedMesh(mesh), ClosestPointHandler::ConvexMesh) => self
                .mesh_points(
                    mesh,
                    shape.header.radius,
                    shape.header.key_bits,
                    state,
                    relative,
                    key,
                    sphere.vertex,
                    sphere.radius,
                    &mut 0.,
                    &mut |shape_key, tag, geometry| {
                        let mut child = ShapeHitInfo { shape_key, ..hit };
                        decode_shape_tag(
                            tag,
                            target.materials.filter(|_| child.body_id.is_some()),
                            &mut child,
                        )?;
                        if filter.is_none_or(|f| {
                            f.accepts_pair(query.filter_info, child.filter_info, true)
                        }) {
                            submit(ShapePointContact { target: child }, geometry);
                        }
                        Ok(0.)
                    },
                ),
            (QueryGeometry::Compound(compound), ClosestPointHandler::ConvexCompound) => {
                let prepared = prepare(
                    sphere,
                    CompoundQueryTransform {
                        scaled: false,
                        shape: ScaledShapeTransform {
                            world_from_local: IDENTITY,
                            scale: [1.; 4],
                            offset: [0.; 4],
                        },
                    },
                    CompoundQueryTransform {
                        shape: state.transform,
                        scaled: state.scaled,
                    },
                    relative,
                    0.,
                )?;
                while self.traversals.len() <= depth {
                    self.traversals.push(CompoundQuery::default());
                }
                let mut traversal = std::mem::take(&mut self.traversals[depth]);
                let mut visitor = InstanceVisitor {
                    owner: self,
                    target,
                    compound,
                    state,
                    relative,
                    key,
                    hit,
                    sphere,
                    query,
                    filter,
                    submit,
                    error: None,
                };
                let mut distance = prepared.squared_distance;
                let result = traversal.visit(
                    CompoundTree {
                        nodes: &compound.nodes,
                        root: compound.root,
                    },
                    prepared.bounds,
                    &mut distance,
                    &mut visitor,
                );
                let error = visitor.error.take();
                self.traversals[depth] = traversal;
                result.and_then(|_| error.map_or(Ok(()), Err))
            }
            _ => Err(format!(
                "point handler {handler:?} is not implemented for retained geometry"
            )),
        })();
        self.active.pop();
        result
    }
}

struct InstanceVisitor<'a, 'b> {
    owner: &'a mut ShapePointQuery,
    target: &'a PointQueryTarget<'b>,
    compound: &'a crate::assets::physworld::compound::CompoundShape,
    state: CompoundShapeState,
    relative: ShapeMatrix,
    key: ShapeKey,
    hit: ShapeHitInfo,
    sphere: &'a mut PointSphere,
    query: ShapeHitInfo,
    filter: Option<CollisionQueryFilter>,
    submit: &'a mut dyn FnMut(ShapePointContact, ConvexClosestPoint),
    error: Option<String>,
}
impl InstanceVisitor<'_, '_> {
    fn visit(&mut self, index: u16) -> Result<(), String> {
        let instance = self
            .compound
            .instances
            .get(usize::from(index))
            .ok_or("compound query instance missing")?;
        if instance.flags & 64 == 0 {
            return Ok(());
        }
        let key = self.key.compound_child(index, self.compound.key_bits)?;
        let mut hit = ShapeHitInfo {
            shape_key: key.value,
            ..self.hit
        };
        decode_shape_tag(
            instance.shape_tag,
            self.target.materials.filter(|_| hit.body_id.is_some()),
            &mut hit,
        )?;
        if self.filter.is_some_and(|filter| {
            !filter.accepts_pair(self.query.filter_info, hit.filter_info, true)
        }) {
            return Ok(());
        }
        let child = self.target.shapes.get(instance.shape)?;
        let vertices = match &child.geometry {
            QueryGeometry::Polytope(p) => p.vertices.as_slice(),
            _ => &[],
        };
        let prepared = prepare_instance(
            instance,
            self.state,
            self.relative,
            CompoundChildGeometry {
                convex: child.header.flags & 1 != 0,
                shape_type: child.header.shape_type,
                vertices,
                radius: child.header.radius,
            },
        )?;
        self.owner.visit(
            self.target,
            instance.shape,
            prepared.state,
            prepared.relative,
            key,
            hit,
            self.sphere,
            self.query,
            self.filter,
            self.submit,
        )
    }
}
impl CompoundQueryVisitor for InstanceVisitor<'_, '_> {
    fn query_instance(&mut self, index: u16, _: &mut f32) {
        if self.error.is_none() {
            self.error = self.visit(index).err();
        }
    }
}

/// One leaf of a closest-point query (cada50's convex dispatch, d10960):
/// the closest point on the leaf and its normal in world axes, the signed
/// distance, the leaf's shape key, and whether the leaf polytope keeps face
/// connectivity (shape flag 2 with connectivity, +0x20/+0x60).
#[derive(Clone, Copy, Debug)]
pub struct LeafClosestPoint {
    pub position: [f32; 4],
    pub normal: [f32; 4],
    pub distance: f32,
    pub shape_key: u32,
    pub connectivity: bool,
}

impl ShapePointQuery {
    /// Each convex leaf of `shapes` placed at `transform` within `threshold`
    /// of `point`; `submit` returns the collector's new early-out distance.
    /// approximate_: compound children are visited in instance order rather
    /// than through the compound tree; the nearest and second nearest leaves
    /// the collector keeps do not depend on that order.
    pub fn closest_leaves(
        &mut self,
        shapes: &QueryShapes,
        transform: ShapeMatrix,
        point: [f32; 3],
        threshold: &mut f32,
        submit: &mut dyn FnMut(LeafClosestPoint) -> f32,
    ) -> Result<(), String> {
        let state = CompoundShapeState {
            transform: ScaledShapeTransform {
                world_from_local: transform,
                scale: [1.; 4],
                offset: [0.; 4],
            },
            scaled: false,
            keep_radius: false,
            radius: 0.,
        };
        let relative = super::shape_transform::relative_rigid(IDENTITY, transform);
        let point = PointSphere::new([point[0], point[1], point[2], 1.], 0.).vertex;
        self.closest_leaf(
            shapes,
            shapes.root(),
            state,
            relative,
            ShapeKey {
                value: u32::MAX,
                bits: 0,
            },
            point,
            threshold,
            submit,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn closest_leaf(
        &mut self,
        shapes: &QueryShapes,
        item: usize,
        state: CompoundShapeState,
        relative: ShapeMatrix,
        key: ShapeKey,
        point: [f32; 4],
        threshold: &mut f32,
        submit: &mut dyn FnMut(LeafClosestPoint) -> f32,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 8 {
            return Err("compound query geometry nests too deeply".into());
        }
        let shape = shapes.get(item)?;
        match &shape.geometry {
            QueryGeometry::Polytope(polytope) => {
                let prepared = self.support.prepare(ConvexSupportInput {
                    shape_type: shape.header.shape_type,
                    vertices: &polytope.vertices,
                    radius: polytope.radius,
                    scaled: state.scaled.then_some(ConvexSupportScale {
                        scale: [
                            state.transform.scale[0],
                            state.transform.scale[1],
                            state.transform.scale[2],
                        ],
                        offset: [
                            state.transform.offset[0],
                            state.transform.offset[1],
                            state.transform.offset[2],
                        ],
                        radius: state.radius,
                    }),
                })?;
                if let Some(hit) = query_point(
                    point,
                    prepared.vertices,
                    relative,
                    0.,
                    prepared.radius,
                    *threshold,
                )? {
                    let world = state.transform.world_from_local;
                    *threshold = submit(LeafClosestPoint {
                        position: transform_point(world, hit.point),
                        normal: super::shape_transform::rotate_vector(world, hit.normal),
                        distance: hit.distance[0],
                        shape_key: key.value,
                        connectivity: shape.header.flags & 2 != 0
                            && polytope.connectivity.is_some(),
                    });
                }
                Ok(())
            }
            QueryGeometry::CompressedMesh(mesh) => self.mesh_points(
                mesh,
                shape.header.radius,
                shape.header.key_bits,
                state,
                relative,
                key,
                point,
                0.,
                threshold,
                &mut |shape_key, _, geometry| {
                    Ok(submit(LeafClosestPoint {
                        position: geometry.point,
                        normal: geometry.normal,
                        distance: geometry.distance[0],
                        shape_key,
                        connectivity: false,
                    }))
                },
            ),
            QueryGeometry::Compound(compound) => {
                for (index, instance) in compound.instances.iter().enumerate() {
                    if instance.flags & 64 == 0 {
                        continue;
                    }
                    let child = shapes.get(instance.shape)?;
                    let vertices = match &child.geometry {
                        QueryGeometry::Polytope(p) => p.vertices.as_slice(),
                        _ => &[],
                    };
                    let prepared = prepare_instance(
                        instance,
                        state,
                        relative,
                        CompoundChildGeometry {
                            convex: child.header.flags & 1 != 0,
                            shape_type: child.header.shape_type,
                            vertices,
                            radius: child.header.radius,
                        },
                    )?;
                    let child_key = key.compound_child(index as u16, compound.key_bits)?;
                    self.closest_leaf(
                        shapes,
                        instance.shape,
                        prepared.state,
                        prepared.relative,
                        child_key,
                        point,
                        threshold,
                        submit,
                        depth + 1,
                    )?;
                }
                Ok(())
            }
        }
    }
}
