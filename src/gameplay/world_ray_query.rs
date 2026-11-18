//! 1287c70/128ed70 world ray dispatch into actual retained shape geometry.
use super::{
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    point_query::PointQueryTree,
    ray_query::{HybridRayQuery, RayQuantization, RayQueryControl, RayQueryVisitor},
    shape_ray_query::{RayQueryTarget, ShapeRayContact, ShapeRayQuery},
    shape_transform::inverse_rotate_vector,
    world_point_query::{PointQueryBodies, WorldQueryShape},
};

pub struct WorldRayRequest {
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub maximum_fraction: f32,
    pub hit: ShapeHitInfo,
    pub filter: Option<CollisionQueryFilter>,
}
impl WorldRayRequest {
    /// 1fc710 reuses the water filter and casts down by the authored distance.
    pub fn particle_water(origin: [f32; 3], distance: f32) -> Self {
        let point = super::world_point_query::WorldPointRequest::water(origin);
        Self {
            origin,
            direction: [0., 0., f32::from_bits(distance.to_bits() ^ 0x8000_0000)],
            maximum_fraction: 1.,
            hit: point.hit,
            filter: point.filter,
        }
    }
}
#[derive(Default)]
pub struct WorldRayQuery {
    broadphase: HybridRayQuery,
    shapes: ShapeRayQuery,
}
impl WorldRayQuery {
    pub fn closest_hit<'a>(
        &mut self,
        bodies: &dyn PointQueryBodies,
        trees: &[PointQueryTree<'_>],
        quantization: RayQuantization,
        request: &WorldRayRequest,
        resolve: impl FnMut(usize) -> Result<WorldQueryShape<'a>, String>,
    ) -> Result<Option<ShapeRayContact>, String> {
        crate::gameplay::physics_world::thread_policy::enter();
        let mut selected = None;
        let mut visitor = Bodies {
            bodies,
            shapes: &mut self.shapes,
            request,
            resolve,
            selected: &mut selected,
            all_hits: None,
            stop_at_first: false,
            query_flags: 0,
            error: None,
        };
        self.broadphase.query(
            trees,
            quantization,
            request.origin,
            request.direction,
            request.maximum_fraction,
            &mut RayQueryControl {
                fraction: f32::from_bits(0x5f7f_fff0),
                early_out: false,
            },
            &mut visitor,
        )?;
        let error = visitor.error.take();
        drop(visitor);
        error.map_or(Ok(selected), Err)
    }

    /// Retain every submitted hit without reducing the traversal bound.
    /// The caller owns scratch storage and any later source-specific ordering.
    pub fn all_hits<'a>(
        &mut self,
        bodies: &dyn PointQueryBodies,
        trees: &[PointQueryTree<'_>],
        quantization: RayQuantization,
        request: &WorldRayRequest,
        resolve: impl FnMut(usize) -> Result<WorldQueryShape<'a>, String>,
        hits: &mut Vec<ShapeRayContact>,
    ) -> Result<(), String> {
        self.all_hits_with_flags(bodies, trees, quantization, request, 0, resolve, hits)
    }

    /// cb0fd0 bit 2 admits inside exits; all other query flags are forwarded.
    pub fn all_hits_with_flags<'a>(
        &mut self,
        bodies: &dyn PointQueryBodies,
        trees: &[PointQueryTree<'_>],
        quantization: RayQuantization,
        request: &WorldRayRequest,
        query_flags: u32,
        resolve: impl FnMut(usize) -> Result<WorldQueryShape<'a>, String>,
        hits: &mut Vec<ShapeRayContact>,
    ) -> Result<(), String> {
        hits.clear();
        crate::gameplay::physics_world::thread_policy::enter();
        let mut selected = None;
        let mut visitor = Bodies {
            bodies,
            shapes: &mut self.shapes,
            request,
            resolve,
            selected: &mut selected,
            all_hits: Some(hits),
            stop_at_first: false,
            query_flags,
            error: None,
        };
        self.broadphase.query(
            trees,
            quantization,
            request.origin,
            request.direction,
            request.maximum_fraction,
            &mut RayQueryControl {
                fraction: f32::from_bits(0x5f7f_fff0),
                early_out: false,
            },
            &mut visitor,
        )?;
        visitor.error.take().map_or(Ok(()), Err)
    }

    /// 0966e0 collector3: stop at the first actual accepted shape witness.
    pub fn any_hit<'a>(
        &mut self,
        bodies: &dyn PointQueryBodies,
        trees: &[PointQueryTree<'_>],
        quantization: RayQuantization,
        request: &WorldRayRequest,
        query_flags: u32,
        resolve: impl FnMut(usize) -> Result<WorldQueryShape<'a>, String>,
    ) -> Result<Option<ShapeRayContact>, String> {
        crate::gameplay::physics_world::thread_policy::enter();
        let mut selected = None;
        let mut visitor = Bodies { bodies, shapes: &mut self.shapes, request, resolve,
            selected: &mut selected, all_hits: None, stop_at_first: true,
            query_flags, error: None };
        self.broadphase.query(trees, quantization, request.origin, request.direction,
            request.maximum_fraction,
            &mut RayQueryControl { fraction: f32::from_bits(0x5f7f_fff0), early_out: false },
            &mut visitor)?;
        let error = visitor.error.take();
        drop(visitor);
        error.map_or(Ok(selected), Err)
    }
}
struct Bodies<'a, F> {
    bodies: &'a dyn PointQueryBodies,
    shapes: &'a mut ShapeRayQuery,
    request: &'a WorldRayRequest,
    resolve: F,
    selected: &'a mut Option<ShapeRayContact>,
    all_hits: Option<&'a mut Vec<ShapeRayContact>>,
    stop_at_first: bool,
    query_flags: u32,
    error: Option<String>,
}
impl<'g, F: FnMut(usize) -> Result<WorldQueryShape<'g>, String>> RayQueryVisitor for Bodies<'_, F> {
    fn accepts_body(&mut self, index: u32) -> bool {
        let Some(body) = self.bodies.body(index & 0x00ff_ffff) else {
            self.error
                .get_or_insert_with(|| format!("ray query body slot {index} absent"));
            return false;
        };
        // 128ed70 rejects the invalid broadphase ID at body+78 before filter.
        body.broadphase_id != u32::MAX
            && self.bodies.accepts_body(body)
            && self
                .request
                .filter
                .is_none_or(|f| f.accepts(body.filter_info))
    }
    fn query_shape(&mut self, index: u32, control: &mut RayQueryControl) {
        if self.error.is_some() {
            control.early_out = true;
            return;
        }
        let result = (|| {
            let body = self
                .bodies
                .body(index & 0x00ff_ffff)
                .ok_or("ray query body disappeared")?;
            let resource = (self.resolve)(body.shape)?;
            let delta = std::array::from_fn(|i| {
                if i < 3 {
                    crate::float::subtract(self.request.origin[i], body.transform[3][i])
                } else {
                    0.
                }
            });
            let origin = inverse_rotate_vector(body.transform, delta);
            let direction = inverse_rotate_vector(
                body.transform,
                [
                    self.request.direction[0],
                    self.request.direction[1],
                    self.request.direction[2],
                    0.,
                ],
            );
            let direction =
                std::array::from_fn(|i| if direction[i] == 0. { 0. } else { direction[i] });
            // 128ed70 keeps the collector bound separate from the ray's own
            // query+4c limit, then takes their SIMD minimum for leaf work.
            // Broadphase admission alone cannot enforce that segment limit
            // on a large body's individual triangles/convex leaves.
            let maximum_fraction = self.request.maximum_fraction;
            let leaf_limit = |collector: f32| {
                if maximum_fraction < collector {
                    maximum_fraction
                } else {
                    collector
                }
            };
            let mut leaf_control = RayQueryControl {
                fraction: leaf_limit(control.fraction),
                early_out: control.early_out,
            };
            self.shapes.cast_with_flags(
                RayQueryTarget {
                    shapes: resource.shapes,
                    hit: ShapeHitInfo {
                        body_id: Some(body.id),
                        material_id: body.material_id,
                        shape_key: u32::MAX,
                        filter_info: body.filter_info,
                        user_data: body.user_data,
                    },
                    materials: resource.materials,
                },
                origin[..3].try_into().unwrap(),
                direction,
                self.request.hit,
                self.request.filter,
                self.query_flags,
                &mut leaf_control,
                &mut |hit, leaf_control| {
                    let hit = hit.transformed_rigid(body.transform);
                    if self.stop_at_first {
                        *self.selected = Some(hit);
                        control.early_out = true;
                    } else if let Some(hits) = self.all_hits.as_deref_mut() {
                        hits.push(hit);
                    } else if hit.fraction < control.fraction {
                        // ca53a0 retains the first hit on equal fractions.
                        control.fraction = hit.fraction;
                        *self.selected = Some(hit);
                    }
                    leaf_control.fraction = leaf_limit(control.fraction);
                    leaf_control.early_out = control.early_out;
                },
            )
        })();
        if let Err(error) = result {
            self.error = Some(error);
            control.early_out = true;
        }
    }
}
