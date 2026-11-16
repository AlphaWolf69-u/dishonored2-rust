//! 1288060/cad6c0 retained-body convex cast dispatch.
use super::{
    collision_hit::ShapeHitInfo,
    point_query::PointQueryTree,
    ray_query::{HybridRayQuery, RayQuantization, RayQueryControl, RayQueryVisitor},
    shape_convex_query::{ConvexCast, ConvexContact, ConvexQueryTarget, ShapeConvexQuery},
    world_point_query::{PointQueryBodies, WorldQueryShape},
};

pub struct WorldConvexRequest<'a> {
    pub cast: ConvexCast<'a>,
    /// Segment parameter bound; displacement is never normalized into a ray.
    pub maximum_fraction: f32,
}
#[derive(Default)]
pub struct WorldConvexQuery {
    broadphase: HybridRayQuery,
    shapes: ShapeConvexQuery,
}
impl WorldConvexQuery {
    /// Keep initial contacts separate, as the two collectors in cbd060.
    /// All shape and body metadata comes from the canonical published owner.
    pub fn all_hits<'a>(
        &mut self,
        bodies: &dyn PointQueryBodies,
        trees: &[PointQueryTree<'_>],
        quantization: RayQuantization,
        request: &WorldConvexRequest<'_>,
        resolve: impl FnMut(usize) -> Result<WorldQueryShape<'a>, String>,
        hits: &mut Vec<ConvexContact>,
        start_hits: &mut Vec<ConvexContact>,
    ) -> Result<(), String> {
        hits.clear();
        start_hits.clear();
        let mut bounds = super::convex_support::convex_bounds(
            request.cast.geometry.vertices,
            request.cast.transform,
            request.cast.geometry.radius,
        );
        let expansion = if 0. > request.cast.start_tolerance {
            0.
        } else {
            request.cast.start_tolerance
        };
        for i in 0..3 {
            bounds[0][i] -= expansion;
            bounds[1][i] += expansion;
        }
        let mut visitor = Bodies {
            bodies,
            shapes: &mut self.shapes,
            request,
            resolve,
            hits,
            start_hits,
            error: None,
        };
        self.broadphase.query_swept_bounds(
            trees,
            quantization,
            [
                bounds[0][..3].try_into().unwrap(),
                bounds[1][..3].try_into().unwrap(),
            ],
            request.cast.displacement,
            request.maximum_fraction,
            &mut RayQueryControl {
                fraction: f32::from_bits(0x5f7f_fff0),
                early_out: false,
            },
            &mut visitor,
        )?;
        visitor.error.take().map_or(Ok(()), Err)
    }
}
struct Bodies<'a, 'r, F> {
    bodies: &'a dyn PointQueryBodies,
    shapes: &'a mut ShapeConvexQuery,
    request: &'a WorldConvexRequest<'r>,
    resolve: F,
    hits: &'a mut Vec<ConvexContact>,
    start_hits: &'a mut Vec<ConvexContact>,
    error: Option<String>,
}
impl<'g, F: FnMut(usize) -> Result<WorldQueryShape<'g>, String>> RayQueryVisitor
    for Bodies<'_, '_, F>
{
    fn accepts_body(&mut self, index: u32) -> bool {
        let Some(body) = self.bodies.body(index & 0x00ff_ffff) else {
            self.error
                .get_or_insert_with(|| format!("convex query body slot {index} absent"));
            return false;
        };
        body.broadphase_id != u32::MAX
            && self.bodies.accepts_body(body)
            && self
                .request
                .cast
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
                .ok_or("convex query body disappeared")?;
            let resource = (self.resolve)(body.shape)?;
            self.shapes.cast(
                ConvexQueryTarget {
                    shapes: resource.shapes,
                    transform: body.transform,
                    hit: ShapeHitInfo {
                        body_id: Some(body.id),
                        material_id: body.material_id,
                        shape_key: u32::MAX,
                        filter_info: body.filter_info,
                        user_data: body.user_data,
                    },
                    materials: resource.materials,
                },
                &self.request.cast,
                control,
                &mut |contact, start, _| {
                    if start {
                        self.start_hits.push(contact);
                    } else {
                        self.hits.push(contact);
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
