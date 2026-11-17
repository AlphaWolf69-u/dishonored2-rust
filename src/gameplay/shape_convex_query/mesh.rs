//! 124faa0/127f590: native packed leaves feed the ordinary convex kernels.
use super::*;
use crate::{
    assets::physworld::compressed_mesh::{CompressedMesh, PrimitiveKind},
    gameplay::compressed_mesh_query::{MeshLeaf, MeshSweep},
};

impl ShapeConvexQuery {
    pub(super) fn mesh_cast(
        &mut self,
        target: &ConvexQueryTarget<'_>,
        mesh: &CompressedMesh,
        radius: f32,
        key_bits: u8,
        state: CompoundShapeState,
        relative: ShapeMatrix,
        key: ShapeKey,
        hit: ShapeHitInfo,
        request: &ConvexCast<'_>,
        control: &mut RayQueryControl,
        submit: &mut dyn FnMut(ConvexContact, bool, &mut RayQueryControl),
    ) -> Result<(), String> {
        let mut geometry = request.geometry;
        let prepared = prepare(
            &mut geometry,
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
            request.start_tolerance,
        )?;
        let mut bounds = prepared.bounds;
        let mut expansion = max(0., request.start_tolerance);
        if state.scaled {
            expansion *= state.transform.scale[..3]
                .iter()
                .map(|v| super::super::constraint_solver::reciprocal(*v).abs())
                .fold(0., max);
        }
        for value in &mut bounds.half_extent {
            *value += expansion;
        }
        let mut direction = inverse_rotate_vector(
            state.transform.world_from_local,
            [
                request.displacement[0],
                request.displacement[1],
                request.displacement[2],
                0.,
            ],
        );
        if state.scaled {
            for i in 0..3 {
                direction[i] *=
                    super::super::constraint_solver::reciprocal(state.transform.scale[i]);
            }
        }
        let support = &mut self.support;
        self.mesh.visit_swept(
            mesh,
            MeshSweep {
                center: bounds.center[..3].try_into().unwrap(),
                half_extent: bounds.half_extent[..3].try_into().unwrap(),
                direction: direction[..3].try_into().unwrap(),
            },
            radius,
            control,
            &mut |leaf, control| {
                crate::gameplay::compressed_mesh_query::support_leaves(leaf, &mut |leaf| {
                    if leaf.kind == PrimitiveKind::Empty {
                        return Ok(());
                    }
                    let mut child = ShapeHitInfo {
                        shape_key: key.child(leaf.key, key_bits)?.value,
                        ..hit
                    };
                    decode_shape_tag(
                        leaf.tag,
                        target.materials.filter(|_| child.body_id.is_some()),
                        &mut child,
                    )?;
                    if request.filter.is_some_and(|f| {
                        !f.accepts_pair(request.hit.filter_info, child.filter_info, true)
                    }) {
                        return Ok(());
                    }
                    let prepared = support.prepare(
                        crate::gameplay::compressed_mesh_query::support_input(&leaf, state)?,
                    )?;
                    cast_support(
                        prepared.vertices,
                        prepared.radius,
                        state,
                        relative,
                        child,
                        request,
                        control,
                        submit,
                    )
                })
            },
        )
    }
    pub(super) fn mesh_closest(
        &mut self,
        target: &ConvexQueryTarget<'_>,
        mesh: &CompressedMesh,
        radius: f32,
        key_bits: u8,
        state: CompoundShapeState,
        relative: ShapeMatrix,
        key: ShapeKey,
        hit: ShapeHitInfo,
        request: &ConvexClosestQuery<'_>,
        submit: &mut dyn FnMut(ConvexContact),
    ) -> Result<(), String> {
        let mut geometry = request.geometry;
        let prepared = prepare(
            &mut geometry,
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
        let mut bounds = prepared.bounds;
        let mut expansion = max(0., request.distance);
        if state.scaled {
            expansion *= state.transform.scale[..3]
                .iter()
                .map(|v| super::super::constraint_solver::reciprocal(*v).abs())
                .fold(0., max);
        }
        for value in &mut bounds.half_extent {
            *value += expansion;
        }
        let support = &mut self.support;
        let mut control = RayQueryControl {
            fraction: 1.,
            early_out: false,
        };
        self.mesh.visit_swept(
            mesh,
            MeshSweep {
                center: bounds.center[..3].try_into().unwrap(),
                half_extent: bounds.half_extent[..3].try_into().unwrap(),
                direction: [0.; 3],
            },
            radius,
            &mut control,
            &mut |leaf, _| {
                crate::gameplay::compressed_mesh_query::support_leaves(leaf, &mut |leaf| {
                    if leaf.kind == PrimitiveKind::Empty {
                        return Ok(());
                    }
                    let mut child = ShapeHitInfo {
                        shape_key: key.child(leaf.key, key_bits)?.value,
                        ..hit
                    };
                    decode_shape_tag(
                        leaf.tag,
                        target.materials.filter(|_| child.body_id.is_some()),
                        &mut child,
                    )?;
                    if request.filter.is_some_and(|f| {
                        !f.accepts_pair(request.hit.filter_info, child.filter_info, true)
                    }) {
                        return Ok(());
                    }
                    let prepared = support.prepare(
                        crate::gameplay::compressed_mesh_query::support_input(&leaf, state)?,
                    )?;
                    if let Some(geometry) = query_pair(
                        request.geometry.vertices,
                        prepared.vertices,
                        relative,
                        request.geometry.radius,
                        prepared.radius,
                        request.distance,
                    )? {
                        submit(ConvexContact {
                            query: request.hit,
                            target: child,
                            position: transform_point(
                                state.transform.world_from_local,
                                geometry.point,
                            ),
                            normal: rotate_vector(
                                state.transform.world_from_local,
                                geometry.normal,
                            ),
                            distance: geometry.distance[0],
                            fraction: 0.,
                            flags: 4,
                        });
                    }
                    Ok(())
                })
            },
        )
    }
}
