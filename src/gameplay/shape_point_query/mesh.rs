//! 127f590's packed primitive support feeds the existing point-distance owner.
use super::*;
use crate::{
    assets::physworld::compressed_mesh::{CompressedMesh, PrimitiveKind},
    gameplay::{compressed_mesh_query::MeshSweep, ray_query::RayQueryControl},
};
impl ShapePointQuery {
    pub(super) fn mesh_points(
        &mut self,
        mesh: &CompressedMesh,
        radius: f32,
        key_bits: u8,
        state: CompoundShapeState,
        relative: ShapeMatrix,
        key: ShapeKey,
        point: [f32; 4],
        query_radius: f32,
        threshold: &mut f32,
        submit: &mut dyn FnMut(u32, u16, ConvexClosestPoint) -> Result<f32, String>,
    ) -> Result<(), String> {
        let mut sphere = PointSphere::new(point, query_radius);
        let prepared = prepare(
            &mut sphere,
            CompoundQueryTransform {
                shape: ScaledShapeTransform {
                    world_from_local: IDENTITY,
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
            *threshold,
        )?;
        let mut bounds = prepared.bounds;
        let mut expansion = if 0. > *threshold { 0. } else { *threshold };
        if state.scaled {
            expansion *= state.transform.scale[..3]
                .iter()
                .map(|v| crate::gameplay::constraint_solver::reciprocal(*v).abs())
                .fold(0., |a, b| if a > b { a } else { b });
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
                center: bounds.center,
                half_extent: bounds.half_extent,
                direction: [0.; 3],
            },
            radius,
            &mut control,
            &mut |leaf, _| {
                crate::gameplay::compressed_mesh_query::support_leaves(leaf, &mut |leaf| {
                    if leaf.kind == PrimitiveKind::Empty {
                        return Ok(());
                    }
                    let prepared = support.prepare(
                        crate::gameplay::compressed_mesh_query::support_input(&leaf, state)?,
                    )?;
                    if let Some(geometry) = query_point(
                        point,
                        prepared.vertices,
                        relative,
                        query_radius,
                        prepared.radius,
                        *threshold,
                    )? {
                        let geometry = ConvexClosestPoint {
                            point: transform_point(
                                state.transform.world_from_local,
                                geometry.point,
                            ),
                            normal: crate::gameplay::shape_transform::rotate_vector(
                                state.transform.world_from_local,
                                geometry.normal,
                            ),
                            distance: geometry.distance,
                        };
                        *threshold =
                            submit(key.child(leaf.key, key_bits)?.value, leaf.tag, geometry)?;
                    }
                    Ok(())
                })
            },
        )
    }
}
