//! cbc5d0/d10960 convex closest leaf publication with 12329d0 traversal.
use super::*;
use crate::gameplay::compound_query::{CompoundQueryVisitor, CompoundTree};

impl ShapeConvexQuery {
    /// Publish each actual closest leaf witness within the requested signed
    /// distance. This is closest-point geometry, not a four-point manifold.
    pub fn closest_hits(
        &mut self,
        target: ConvexQueryTarget<'_>,
        request: &ConvexClosestQuery<'_>,
        submit: &mut dyn FnMut(ConvexContact),
    ) -> Result<(), String> {
        self.active.clear();
        if request
            .filter
            .is_some_and(|f| !f.accepts_pair(request.hit.filter_info, target.hit.filter_info, true))
        {
            return Ok(());
        }
        let state = CompoundShapeState {
            transform: ScaledShapeTransform {
                world_from_local: target.transform,
                scale: [1.; 4],
                offset: [0.; 4],
            },
            scaled: false,
            keep_radius: false,
            radius: 0.,
        };
        self.closest_leaf(
            &target,
            target.shapes.root(),
            state,
            relative_rigid(request.transform, target.transform),
            ShapeKey {
                value: target.hit.shape_key,
                bits: 0,
            },
            target.hit,
            request,
            submit,
        )
    }
    fn closest_leaf(
        &mut self,
        target: &ConvexQueryTarget<'_>,
        item: usize,
        state: CompoundShapeState,
        relative: ShapeMatrix,
        key: ShapeKey,
        hit: ShapeHitInfo,
        request: &ConvexClosestQuery<'_>,
        submit: &mut dyn FnMut(ConvexContact),
    ) -> Result<(), String> {
        if self.active.contains(&item) {
            return Err("cyclic closest query compound".into());
        }
        let shape = target.shapes.get(item)?;
        let depth = self.active.len();
        self.active.push(item);
        let result = (|| match &shape.geometry {
            QueryGeometry::Polytope(_) | QueryGeometry::Convex(_) => {
                let support = self.support.prepare(ConvexSupportInput {
                    shape_type: shape.header.shape_type,
                    vertices: shape
                        .geometry
                        .convex_vertices()
                        .ok_or("convex closest vertices absent")?,
                    radius: shape
                        .geometry
                        .convex_radius()
                        .ok_or("convex closest radius absent")?,
                    scaled: state.scaled.then_some(ConvexSupportScale {
                        scale: state.transform.scale[..3].try_into().unwrap(),
                        offset: state.transform.offset[..3].try_into().unwrap(),
                        radius: state.radius,
                    }),
                })?;
                if let Some(geometry) = query_pair(
                    request.geometry.vertices,
                    support.vertices,
                    relative,
                    request.geometry.radius,
                    support.radius,
                    request.distance,
                )? {
                    submit(ConvexContact {
                        query: request.hit,
                        target: hit,
                        position: transform_point(state.transform.world_from_local, geometry.point),
                        normal: rotate_vector(state.transform.world_from_local, geometry.normal),
                        distance: geometry.distance[0],
                        fraction: 0.,
                        flags: 4,
                    });
                }
                Ok(())
            }
            QueryGeometry::CompressedMesh(mesh) => self.mesh_closest(
                target,
                mesh,
                shape.header.radius,
                shape.header.key_bits,
                state,
                relative,
                key,
                hit,
                request,
                submit,
            ),
            QueryGeometry::Compound(compound) => {
                let mut query_geometry = request.geometry;
                let prepared = prepare(
                    &mut query_geometry,
                    CompoundQueryTransform {
                        shape: ScaledShapeTransform {
                            world_from_local: request.transform,
                            scale: [1.; 4],
                            offset: [0.; 4],
                        },
                        scaled: false,
                    },
                    CompoundQueryTransform {
                        shape: state.transform,
                        scaled: state.scaled,
                    },
                    relative,
                    request.distance,
                )?;
                self.closest_traversals.resize_with(
                    self.closest_traversals.len().max(depth + 1),
                    super::super::compound_query::CompoundQuery::default,
                );
                let mut traversal = std::mem::take(&mut self.closest_traversals[depth]);
                let mut visitor = Instances {
                    owner: self,
                    target,
                    compound,
                    state,
                    relative,
                    key,
                    hit,
                    request,
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
                self.closest_traversals[depth] = traversal;
                result.and_then(|_| error.map_or(Ok(()), Err))
            }
        })();
        self.active.pop();
        result
    }
}
struct Instances<'a, 'g, 'q> {
    owner: &'a mut ShapeConvexQuery,
    target: &'a ConvexQueryTarget<'g>,
    compound: &'g crate::assets::physworld::compound::CompoundShape,
    state: CompoundShapeState,
    relative: ShapeMatrix,
    key: ShapeKey,
    hit: ShapeHitInfo,
    request: &'a ConvexClosestQuery<'q>,
    submit: &'a mut dyn FnMut(ConvexContact),
    error: Option<String>,
}
impl Instances<'_, '_, '_> {
    fn visit(&mut self, index: u16) -> Result<(), String> {
        let instance = self
            .compound
            .instances
            .get(usize::from(index))
            .ok_or("closest compound instance absent")?;
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
        if self
            .request
            .filter
            .is_some_and(|f| !f.accepts_pair(self.request.hit.filter_info, hit.filter_info, true))
        {
            return Ok(());
        }
        let child = self.target.shapes.get(instance.shape)?;
        let vertices = match &child.geometry {
            QueryGeometry::Polytope(p) => p.vertices.as_slice(),
            QueryGeometry::Convex(p) => p.vertices.as_slice(),
            _ => &[],
        };
        let next = prepare_instance(
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
        self.owner.closest_leaf(
            self.target,
            instance.shape,
            next.state,
            next.relative,
            key,
            hit,
            self.request,
            self.submit,
        )
    }
}
impl CompoundQueryVisitor for Instances<'_, '_, '_> {
    fn query_instance(&mut self, index: u16, _: &mut f32) {
        if self.error.is_none() {
            self.error = self.visit(index).err();
        }
    }
}
