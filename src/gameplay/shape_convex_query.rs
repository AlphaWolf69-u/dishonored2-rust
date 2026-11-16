//! cbd060 convex casts and cbc5d0 closest queries on retained convex leaves.
//! The GSK/EPA owner supplies geometry; BVH bounds only admit leaf work.
use super::{
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    compound_query::{
        instance::{prepare_instance, CompoundChildGeometry, CompoundShapeState, ShapeKey},
        prepare::{prepare, CompoundQueryGeometry, CompoundQueryTransform},
    },
    convex_support::{
        self, pair_distance::query_pair, ConvexSupportInput, ConvexSupportScale,
        ConvexSupportWorkspace,
    },
    ray_query::RayQueryControl,
    shape_tags::{decode_shape_tag, ShapeTagSource},
    shape_transform::{
        inverse_rotate_vector, relative_rigid, rotate_vector, transform_point,
        ScaledShapeTransform, ShapeMatrix,
    },
};
use crate::assets::physworld::{
    compound::types::CompoundChildren,
    query_geometry::{QueryGeometry, QueryShapes},
};
#[path = "shape_convex_query/closest.rs"]
mod closest;

pub struct ConvexClosestQuery<'a> {
    pub geometry: ConvexQueryGeometry<'a>,
    pub transform: ShapeMatrix,
    pub distance: f32,
    pub hit: ShapeHitInfo,
    pub filter: Option<CollisionQueryFilter>,
}

#[derive(Clone, Copy)]
pub struct ConvexQueryGeometry<'a> {
    pub shape_type: u8,
    pub vertices: &'a [[f32; 4]],
    pub radius: f32,
}
impl CompoundQueryGeometry for ConvexQueryGeometry<'_> {
    fn calculate_bounds(&mut self, matrix: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        Ok(convex_support::convex_bounds(
            self.vertices,
            matrix,
            self.radius,
        ))
    }
    fn convex_vertices(&self) -> Option<(&[[f32; 4]], f32)> {
        Some((self.vertices, self.radius))
    }
}

pub struct ConvexQueryTarget<'a> {
    pub shapes: &'a QueryShapes,
    pub transform: ShapeMatrix,
    pub hit: ShapeHitInfo,
    pub materials: Option<&'a dyn ShapeTagSource>,
}
#[derive(Clone, Copy, Debug)]
pub struct ConvexContact {
    pub query: ShapeHitInfo,
    pub target: ShapeHitInfo,
    pub position: [f32; 4],
    pub normal: [f32; 4],
    /// Actual signed separation, retained separately from the cast fraction.
    pub distance: f32,
    pub fraction: f32,
    /// cbd060 publishes 4 for initial contacts and 2 for cast contacts.
    pub flags: u32,
}
pub struct ConvexCast<'a> {
    pub geometry: ConvexQueryGeometry<'a>,
    pub transform: ShapeMatrix,
    pub displacement: [f32; 3],
    pub tolerance: f32,
    pub start_tolerance: f32,
    pub hit: ShapeHitInfo,
    pub filter: Option<CollisionQueryFilter>,
}

#[derive(Default)]
pub struct ShapeConvexQuery {
    support: ConvexSupportWorkspace,
    pending: Vec<Vec<u16>>,
    active: Vec<usize>,
    closest_traversals: Vec<super::compound_query::CompoundQuery>,
}
impl ShapeConvexQuery {
    pub fn cast(
        &mut self,
        target: ConvexQueryTarget<'_>,
        request: &ConvexCast<'_>,
        control: &mut RayQueryControl,
        submit: &mut dyn FnMut(ConvexContact, bool, &mut RayQueryControl),
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
        self.visit(
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
            control,
            submit,
        )
    }

    fn visit(
        &mut self,
        target: &ConvexQueryTarget<'_>,
        item: usize,
        state: CompoundShapeState,
        relative: ShapeMatrix,
        key: ShapeKey,
        hit: ShapeHitInfo,
        request: &ConvexCast<'_>,
        control: &mut RayQueryControl,
        submit: &mut dyn FnMut(ConvexContact, bool, &mut RayQueryControl),
    ) -> Result<(), String> {
        if self.active.contains(&item) {
            return Err("cyclic convex query compound".into());
        }
        let shape = target.shapes.get(item)?;
        self.active.push(item);
        let result = (|| match &shape.geometry {
            QueryGeometry::Polytope(polytope) => {
                let support = self.support.prepare(ConvexSupportInput {
                    shape_type: shape.header.shape_type,
                    vertices: &polytope.vertices,
                    radius: polytope.radius,
                    scaled: state.scaled.then_some(ConvexSupportScale {
                        scale: state.transform.scale[..3].try_into().unwrap(),
                        offset: state.transform.offset[..3].try_into().unwrap(),
                        radius: state.radius,
                    }),
                })?;
                let direction = inverse_rotate_vector(
                    state.transform.world_from_local,
                    [
                        request.displacement[0],
                        request.displacement[1],
                        request.displacement[2],
                        0.,
                    ],
                );
                let mut moving = relative;
                let mut fraction = [0.; 4];
                for iteration in 0..256 {
                    let Some(geometry) = query_pair(
                        request.geometry.vertices,
                        support.vertices,
                        moving,
                        request.geometry.radius,
                        support.radius,
                        f32::from_bits(0x7f7f_ffee),
                    )?
                    else {
                        break;
                    };
                    let contact = ConvexContact {
                        query: request.hit,
                        target: hit,
                        position: transform_point(state.transform.world_from_local, geometry.point),
                        normal: rotate_vector(state.transform.world_from_local, geometry.normal),
                        distance: geometry.distance[0],
                        fraction: fraction[0],
                        flags: 2,
                    };
                    if iteration == 0 && geometry.distance[0] <= request.start_tolerance {
                        submit(
                            ConvexContact {
                                flags: 4,
                                ..contact
                            },
                            true,
                            control,
                        );
                    }
                    if geometry.distance[0] <= request.tolerance {
                        submit(contact, false, control);
                        break;
                    }
                    let approach = (geometry.normal[1] * direction[1]
                        + geometry.normal[0] * direction[0])
                        + geometry.normal[2] * direction[2];
                    if -f32::from_bits(0x3400_0000) < approach {
                        break;
                    }
                    let inverse = super::constraint_solver::reciprocal(approach);
                    fraction =
                        std::array::from_fn(|i| fraction[i] - inverse * geometry.distance[i]);
                    if control.fraction <= fraction[0] && fraction[0] != control.fraction {
                        break;
                    }
                    moving[3] =
                        std::array::from_fn(|i| fraction[i] * direction[i] + relative[3][i]);
                }
                Ok(())
            }
            QueryGeometry::Compound(compound) => {
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
                // 1232210 expands the calcAabb by the nonnegative start
                // tolerance before its swept-box traversal.
                let mut expansion = max(0., request.start_tolerance);
                if state.scaled {
                    expansion *= max(
                        super::constraint_solver::reciprocal(state.transform.scale[2]).abs(),
                        max(
                            super::constraint_solver::reciprocal(state.transform.scale[1]).abs(),
                            super::constraint_solver::reciprocal(state.transform.scale[0]).abs(),
                        ),
                    );
                }
                for lane in &mut bounds.half_extent {
                    *lane += expansion;
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
                            super::constraint_solver::reciprocal(state.transform.scale[i]);
                    }
                }
                let inverse: [f32; 3] = std::array::from_fn(|i| {
                    if direction[i] == 0. {
                        f32::from_bits(0x7f7f_ffee)
                    } else {
                        super::constraint_solver::reciprocal(direction[i])
                    }
                });
                let depth = self.active.len() - 1;
                self.pending
                    .resize_with(self.pending.len().max(depth + 1), Vec::new);
                let mut pending = std::mem::take(&mut self.pending[depth]);
                pending.clear();
                let traversal = (|| {
                    if compound.root == 0 {
                        return Ok(());
                    }
                    let interval = |index: u16, limit: f32| -> Result<[f32; 2], String> {
                        let node = compound
                            .nodes
                            .get(usize::from(index))
                            .ok_or("convex compound node absent")?;
                        let lo: [f32; 3] = std::array::from_fn(|i| {
                            ((node.bounds[0][i] - bounds.half_extent[i]) - bounds.center[i])
                                * inverse[i]
                        });
                        let hi: [f32; 3] = std::array::from_fn(|i| {
                            ((bounds.half_extent[i] + node.bounds[1][i]) - bounds.center[i])
                                * inverse[i]
                        });
                        let near: [f32; 3] = std::array::from_fn(|i| min(lo[i], hi[i]));
                        let far: [f32; 3] = std::array::from_fn(|i| max(lo[i], hi[i]));
                        Ok([
                            max(max(near[2], max(near[1], near[0])), 0.),
                            min(min(far[2], min(far[1], far[0])), limit),
                        ])
                    };
                    let first = interval(compound.root, control.fraction)?;
                    if !(first[0] <= first[1]) {
                        return Ok(());
                    }
                    let mut index = compound.root;
                    loop {
                        match compound
                            .nodes
                            .get(usize::from(index))
                            .ok_or("convex compound node absent")?
                            .children
                        {
                            CompoundChildren::Branch { left, right } => {
                                let l = interval(left, control.fraction)?;
                                let r = interval(right, control.fraction)?;
                                match (!(l[1] < l[0]), !(r[1] < r[0])) {
                                    (true, true) => {
                                        if r[0] < l[0] {
                                            pending.push(left);
                                            index = right;
                                        } else {
                                            pending.push(right);
                                            index = left;
                                        }
                                        continue;
                                    }
                                    (true, false) => {
                                        index = left;
                                        continue;
                                    }
                                    (false, true) => {
                                        index = right;
                                        continue;
                                    }
                                    _ => {}
                                }
                            }
                            CompoundChildren::Instance(child_index) => {
                                let instance = compound
                                    .instances
                                    .get(usize::from(child_index))
                                    .ok_or("convex compound instance absent")?;
                                if instance.flags & 64 != 0 {
                                    let key = key.compound_child(child_index, compound.key_bits)?;
                                    let mut hit = ShapeHitInfo {
                                        shape_key: key.value,
                                        ..hit
                                    };
                                    decode_shape_tag(
                                        instance.shape_tag,
                                        target.materials.filter(|_| hit.body_id.is_some()),
                                        &mut hit,
                                    )?;
                                    if request.filter.is_none_or(|f| {
                                        f.accepts_pair(
                                            request.hit.filter_info,
                                            hit.filter_info,
                                            true,
                                        )
                                    }) {
                                        let child = target.shapes.get(instance.shape)?;
                                        let vertices = match &child.geometry {
                                            QueryGeometry::Polytope(p) => p.vertices.as_slice(),
                                            _ => &[],
                                        };
                                        let next = prepare_instance(
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
                                        self.visit(
                                            target,
                                            instance.shape,
                                            next.state,
                                            next.relative,
                                            key,
                                            hit,
                                            request,
                                            control,
                                            submit,
                                        )?;
                                        if control.early_out {
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        let Some(next) = pending.pop() else { break };
                        index = next;
                    }
                    Ok(())
                })();
                self.pending[depth] = pending;
                traversal
            }
        })();
        self.active.pop();
        result
    }
}
fn min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}
