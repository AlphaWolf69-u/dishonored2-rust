//! cb0fd0/123c770 ray-hit projection for rigid world-body query targets.
//! Nested instance scales are applied to the ray, as in the compound virtual;
//! closest-point scale/radius preparation is deliberately a separate path.
use super::{
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    compound_query::instance::ShapeKey,
    constraint_solver::reciprocal,
    ray_query::RayQueryControl,
    shape_tags::{decode_shape_tag, ShapeTagSource},
    shape_transform::{inverse_rotate_vector, rotate_vector, transform_point, ShapeMatrix},
};
use crate::{
    assets::physworld::{
        compound::{types::CompoundChildren, CompoundShape},
        query_geometry::{QueryGeometry, QueryShapes},
    },
    float::{multiply, subtract},
};

pub struct RayQueryTarget<'a> {
    pub shapes: &'a QueryShapes,
    pub hit: ShapeHitInfo,
    pub materials: Option<&'a dyn ShapeTagSource>,
}
#[derive(Clone, Copy, Debug)]
pub struct ShapeRayContact {
    pub target: ShapeHitInfo,
    pub fraction: f32,
    /// Geometry is expressed in the receiving query owner's frame.
    pub position: [f32; 4],
    pub normal: [f32; 4],
    /// cb0fd0 hit/inside-exit bits, separate from body/filter flags.
    pub flags: u32,
}
impl ShapeRayContact {
    /// 123ec60 transports the witness by scale then pose; normals use
    /// reciprocal scale then pose and one RSQRTPS/Newton normalization.
    /// All identity/fraction fields pass through unchanged.
    pub fn transformed_scaled(mut self, pose: ShapeMatrix, scale: [f32; 4]) -> Self {
        self.position = transform_point(
            pose,
            std::array::from_fn(|i| multiply(self.position[i], scale[i])),
        );
        let normal = rotate_vector(
            pose,
            std::array::from_fn(|i| multiply(self.normal[i], reciprocal(scale[i]))),
        );
        let squared = (normal[1] * normal[1] + normal[0] * normal[0]) + normal[2] * normal[2];
        let estimate = super::convex_support::faces::reciprocal_sqrt_estimate(squared);
        let correction = 3.0 - estimate * squared * estimate;
        self.normal = normal.map(|lane| ((lane * correction) * estimate) * 0.5);
        self
    }
    pub fn transformed_rigid(mut self, pose: ShapeMatrix) -> Self {
        self.position = transform_point(pose, self.position);
        self.normal = rotate_vector(pose, self.normal);
        self
    }
}
#[derive(Default)]
pub struct ShapeRayQuery {
    pending: Vec<Vec<u16>>,
    active: Vec<usize>,
    query_flags: u32,
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
fn inverse_direction(direction: [f32; 3]) -> [f32; 3] {
    direction.map(|v| {
        if v == 0. {
            f32::from_bits(0x7f7f_ffee)
        } else {
            reciprocal(v)
        }
    })
}
fn interval(bounds: [[f32; 3]; 2], origin: [f32; 3], inverse: [f32; 3], limit: f32) -> [f32; 2] {
    let first: [f32; 3] =
        std::array::from_fn(|i| multiply(subtract(bounds[0][i], origin[i]), inverse[i]));
    let second: [f32; 3] =
        std::array::from_fn(|i| multiply(subtract(bounds[1][i], origin[i]), inverse[i]));
    let lower: [f32; 3] = std::array::from_fn(|i| min(first[i], second[i]));
    let upper: [f32; 3] = std::array::from_fn(|i| max(first[i], second[i]));
    [
        max(max(lower[2], max(lower[1], lower[0])), 0.),
        min(min(upper[2], min(upper[1], upper[0])), limit),
    ]
}

impl ShapeRayQuery {
    /// Origin/direction are in the body's rigid local frame. 123ec60 copies
    /// the hit's fraction/identity unchanged while transporting geometry.
    pub fn cast(
        &mut self,
        target: RayQueryTarget<'_>,
        origin: [f32; 3],
        direction: [f32; 3],
        query: ShapeHitInfo,
        filter: Option<CollisionQueryFilter>,
        control: &mut RayQueryControl,
        submit: &mut dyn FnMut(ShapeRayContact, &mut RayQueryControl),
    ) -> Result<(), String> {
        self.cast_with_flags(target, origin, direction, query, filter, 0, control, submit)
    }
    pub fn cast_with_flags(
        &mut self,
        target: RayQueryTarget<'_>,
        origin: [f32; 3],
        direction: [f32; 3],
        query: ShapeHitInfo,
        filter: Option<CollisionQueryFilter>,
        query_flags: u32,
        control: &mut RayQueryControl,
        submit: &mut dyn FnMut(ShapeRayContact, &mut RayQueryControl),
    ) -> Result<(), String> {
        self.active.clear();
        self.query_flags = query_flags;
        let key = ShapeKey {
            value: target.hit.shape_key,
            bits: 0,
        };
        self.visit(
            &target,
            target.shapes.root(),
            origin,
            direction,
            key,
            target.hit,
            query,
            filter,
            control,
            submit,
        )
    }

    fn visit(
        &mut self,
        target: &RayQueryTarget<'_>,
        item: usize,
        origin: [f32; 3],
        direction: [f32; 3],
        key: ShapeKey,
        hit: ShapeHitInfo,
        query: ShapeHitInfo,
        filter: Option<CollisionQueryFilter>,
        control: &mut RayQueryControl,
        submit: &mut dyn FnMut(ShapeRayContact, &mut RayQueryControl),
    ) -> Result<(), String> {
        if self.active.contains(&item) {
            return Err("ray query has cyclic compound geometry".into());
        }
        self.active.push(item);
        let result = (|| {
            match &target.shapes.get(item)?.geometry {
                QueryGeometry::Polytope(shape) => {
                    // World requests carry query flags separately from the
                    // collision-filter packet; the water cast uses flags0.
                    if let Some(candidate) = super::polytope_query::ray::cast(
                        shape,
                        origin,
                        direction,
                        control.fraction,
                        self.query_flags,
                    )? {
                        submit(
                            ShapeRayContact {
                                target: hit,
                                fraction: candidate.fraction,
                                position: [
                                    crate::float::add(
                                        origin[0],
                                        multiply(direction[0], candidate.fraction),
                                    ),
                                    crate::float::add(
                                        origin[1],
                                        multiply(direction[1], candidate.fraction),
                                    ),
                                    crate::float::add(
                                        origin[2],
                                        multiply(direction[2], candidate.fraction),
                                    ),
                                    0.0,
                                ],
                                normal: [
                                    candidate.normal[0],
                                    candidate.normal[1],
                                    candidate.normal[2],
                                    0.0,
                                ],
                                flags: candidate.flags,
                            },
                            control,
                        );
                    }
                    Ok(())
                }
                QueryGeometry::Compound(shape) => self.compound(
                    target, shape, origin, direction, key, hit, query, filter, control, submit,
                ),
            }
        })();
        self.active.pop();
        result
    }

    fn compound(
        &mut self,
        target: &RayQueryTarget<'_>,
        shape: &CompoundShape,
        origin: [f32; 3],
        direction: [f32; 3],
        key: ShapeKey,
        hit: ShapeHitInfo,
        query: ShapeHitInfo,
        filter: Option<CollisionQueryFilter>,
        control: &mut RayQueryControl,
        submit: &mut dyn FnMut(ShapeRayContact, &mut RayQueryControl),
    ) -> Result<(), String> {
        if shape.root == 0 {
            return Ok(());
        }
        let inverse = inverse_direction(direction);
        let root = shape
            .nodes
            .get(usize::from(shape.root))
            .ok_or("ray compound root absent")?;
        let initial = interval(root.bounds, origin, inverse, control.fraction);
        if !(initial[0] <= initial[1]) {
            return Ok(());
        }
        let depth = self.active.len() - 1;
        self.pending
            .resize_with(self.pending.len().max(depth + 1), Vec::new);
        let mut pending = std::mem::take(&mut self.pending[depth]);
        pending.clear();
        let mut node_index = shape.root;
        let result = (|| {
            loop {
                let node = shape
                    .nodes
                    .get(usize::from(node_index))
                    .ok_or("ray compound node absent")?;
                match node.children {
                    CompoundChildren::Branch { left, right } => {
                        let left_bounds = shape
                            .nodes
                            .get(usize::from(left))
                            .ok_or("ray compound left child absent")?
                            .bounds;
                        let right_bounds = shape
                            .nodes
                            .get(usize::from(right))
                            .ok_or("ray compound right child absent")?
                            .bounds;
                        let l = interval(left_bounds, origin, inverse, control.fraction);
                        let r = interval(right_bounds, origin, inverse, control.fraction);
                        match (l[0] <= l[1], r[0] <= r[1]) {
                            (true, true) => {
                                // Unlike the hybrid WORLD tree, the float
                                // compound tree keeps its LEFT child on ties.
                                if r[0] < l[0] {
                                    pending.push(left);
                                    node_index = right;
                                } else {
                                    pending.push(right);
                                    node_index = left;
                                }
                                continue;
                            }
                            (true, false) => {
                                node_index = left;
                                continue;
                            }
                            (false, true) => {
                                node_index = right;
                                continue;
                            }
                            (false, false) => {}
                        }
                    }
                    CompoundChildren::Instance(index) => {
                        let instance = shape
                            .instances
                            .get(usize::from(index))
                            .ok_or("ray compound instance absent")?;
                        if instance.flags & 64 != 0 {
                            let child_key = key.compound_child(index, shape.key_bits)?;
                            let mut child_hit = ShapeHitInfo {
                                shape_key: child_key.value,
                                ..hit
                            };
                            decode_shape_tag(
                                instance.shape_tag,
                                target.materials.filter(|_| child_hit.body_id.is_some()),
                                &mut child_hit,
                            )?;
                            if filter.is_none_or(|f| {
                                f.accepts_pair(query.filter_info, child_hit.filter_info, true)
                            }) {
                                let delta = [
                                    subtract(origin[0], instance.transform[3][0]),
                                    subtract(origin[1], instance.transform[3][1]),
                                    subtract(origin[2], instance.transform[3][2]),
                                    0.,
                                ];
                                let local_origin = inverse_rotate_vector(instance.transform, delta);
                                let local_direction = inverse_rotate_vector(
                                    instance.transform,
                                    [direction[0], direction[1], direction[2], 0.],
                                );
                                let scale: [f32; 3] =
                                    std::array::from_fn(|i| reciprocal(instance.scale[i]));
                                let child_origin =
                                    std::array::from_fn(|i| multiply(local_origin[i], scale[i]));
                                let child_direction = std::array::from_fn(|i| {
                                    let value = multiply(local_direction[i], scale[i]);
                                    if value == 0. {
                                        0.
                                    } else {
                                        value
                                    }
                                });
                                self.visit(
                                    target,
                                    instance.shape,
                                    child_origin,
                                    child_direction,
                                    child_key,
                                    child_hit,
                                    query,
                                    filter,
                                    control,
                                    &mut |contact, control| {
                                        submit(
                                            contact.transformed_scaled(
                                                instance.transform,
                                                instance.scale,
                                            ),
                                            control,
                                        )
                                    },
                                )?;
                                if control.early_out {
                                    break;
                                }
                            }
                        }
                    }
                }
                // Native queued nodes proceed directly to their contents;
                // they are not retested after the collector shrinks its bound.
                let Some(next) = pending.pop() else {
                    break;
                };
                node_index = next;
            }
            Ok(())
        })();
        self.pending[depth] = pending;
        result
    }
}
