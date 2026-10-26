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
        let point = super::world_point_query::WorldPointRequest::particle_water(origin);
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
        let mut selected = None;
        let mut visitor = Bodies {
            bodies,
            shapes: &mut self.shapes,
            request,
            resolve,
            selected: &mut selected,
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
}
struct Bodies<'a, F> {
    bodies: &'a dyn PointQueryBodies,
    shapes: &'a mut ShapeRayQuery,
    request: &'a WorldRayRequest,
    resolve: F,
    selected: &'a mut Option<ShapeRayContact>,
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
            self.shapes.cast(
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
                control,
                &mut |hit, control| {
                    // ca53a0 retains the first hit on equal fractions.
                    if hit.fraction < control.fraction {
                        control.fraction = hit.fraction;
                        *self.selected = Some(hit);
                    }
                },
            )
        })();
        if let Err(error) = result {
            self.error = Some(error);
            control.early_out = true;
        }
    }
}
