//! Zero-distance all-hit world query used by render-view environment contacts.
//! Composes 1288af0 body/filter admission with ce2de0 shape execution. World
//! creation, body publication and actor/component resolution stay with the owner.
use super::{
    body_storage::BodyStorage,
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    collision_queries::CollisionQueries,
    compound_query::instance::CompoundShapeState,
    point_query::{
        HybridPointQuery, PointQuantization, PointQueryControl, PointQueryTree, PointQueryVisitor,
    },
    shape_point_query::{PointQueryTarget, ShapePointContact, ShapePointQuery},
    shape_tags::ShapeTagSource,
    shape_transform::ScaledShapeTransform,
};
use crate::assets::physworld::query_geometry::QueryShapes;

pub struct WorldQueryShape<'a> {
    pub shapes: &'a QueryShapes,
    pub materials: Option<&'a dyn ShapeTagSource>,
}

pub struct WorldPointQuery {
    broadphase: HybridPointQuery,
    shapes: ShapePointQuery,
}

pub struct WorldPointRequest {
    pub point: [f32; 3],
    pub hit: ShapeHitInfo,
    pub filter: Option<CollisionQueryFilter>,
}

impl WorldPointRequest {
    /// 806c30 submits the active render view's camera point with only ordinary
    /// and water-trigger layers (410000), no required body flags, zero distance
    /// and the all-hits collector. 963b0 supplies the filter to shape traversal.
    pub fn environment(point: [f32; 3]) -> Self {
        Self {
            point,
            hit: ShapeHitInfo {
                body_id: None,
                material_id: u16::MAX,
                shape_key: u32::MAX,
                filter_info: 0,
                user_data: 0,
            },
            filter: Some(CollisionQueryFilter {
                layers: 0x0041_0000,
                required_flags: 0,
            }),
        }
    }
}

impl WorldPointQuery {
    /// Execute the real geometry path for queued point queries. No callback
    /// supplies hits or substitutes a dry result when an asset cannot be read.
    pub fn resolve_pending<'a>(
        &mut self,
        queries: &mut CollisionQueries<WorldPointRequest, ShapePointContact>,
        bodies: &BodyStorage,
        trees: &[PointQueryTree<'_>],
        quantization: PointQuantization,
        mut resolve: impl FnMut(usize) -> Result<WorldQueryShape<'a>, String>,
    ) -> Result<(), String> {
        let mut contacts = Vec::new();
        queries.try_resolve_pending(|request, hits| {
            contacts.clear();
            self.all_hits(
                bodies,
                trees,
                quantization,
                request.point,
                request.hit,
                request.filter,
                &mut resolve,
                &mut contacts,
            )?;
            for &contact in &contacts {
                hits.push(contact);
            }
            Ok(())
        })
    }

    pub fn new(enabled_shape_types: u32) -> Result<Self, String> {
        Ok(Self {
            broadphase: HybridPointQuery::default(),
            shapes: ShapePointQuery::new(enabled_shape_types)?,
        })
    }

    /// The concrete ark query filter accepts every tree (11d080), then applies
    /// 0996a0 to each candidate body's filter before shape/tag filtering. An
    /// unsupported or missing body/shape is an execution error, never a dry hit.
    /// Only the metadata projection consumed by the view is returned.
    pub fn all_hits<'a>(
        &mut self,
        bodies: &BodyStorage,
        trees: &[PointQueryTree<'_>],
        quantization: PointQuantization,
        point: [f32; 3],
        query: ShapeHitInfo,
        filter: Option<CollisionQueryFilter>,
        resolve: impl FnMut(usize) -> Result<WorldQueryShape<'a>, String>,
        contacts: &mut Vec<ShapePointContact>,
    ) -> Result<(), String> {
        let start = contacts.len();
        let mut visitor = Bodies {
            bodies,
            shapes: &mut self.shapes,
            point: [point[0], point[1], point[2], 0.],
            query,
            filter,
            resolve,
            contacts,
            error: None,
        };
        let result = self
            .broadphase
            .visit(
                trees,
                quantization,
                point,
                0.,
                &mut PointQueryControl {
                    distance: f32::MAX,
                    early_out: false,
                },
                &mut visitor,
            )
            .and_then(|()| visitor.error.take().map_or(Ok(()), Err));
        if result.is_err() {
            visitor.contacts.truncate(start);
        }
        result
    }
}

struct Bodies<'a, F> {
    bodies: &'a BodyStorage,
    shapes: &'a mut ShapePointQuery,
    point: [f32; 4],
    query: ShapeHitInfo,
    filter: Option<CollisionQueryFilter>,
    resolve: F,
    contacts: &'a mut Vec<ShapePointContact>,
    error: Option<String>,
}

impl<'g, F: FnMut(usize) -> Result<WorldQueryShape<'g>, String>> PointQueryVisitor
    for Bodies<'_, F>
{
    fn accepts_body(&mut self, index: u32) -> bool {
        // 1288af0 masks the generation byte before indexing the body array.
        let Some(body) = self.bodies.get_by_index(index & 0xffffff) else {
            self.error
                .get_or_insert_with(|| format!("point query references missing body slot {index}"));
            return false;
        };
        self.filter
            .is_none_or(|filter| filter.accepts(body.filter_info))
    }

    fn query_shape(&mut self, index: u32, control: &mut PointQueryControl) {
        if self.error.is_some() {
            control.early_out = true;
            return;
        }
        let result = (|| {
            let body = self
                .bodies
                .get_by_index(index & 0xffffff)
                .ok_or("point query body disappeared")?;
            let resource = (self.resolve)(body.shape)?;
            self.shapes.all_hits(
                PointQueryTarget {
                    shapes: resource.shapes,
                    state: CompoundShapeState {
                        transform: ScaledShapeTransform {
                            world_from_local: body.transform,
                            scale: [1.; 4],
                            offset: [0.; 4],
                        },
                        scaled: false,
                        keep_radius: false,
                        radius: 0.,
                    },
                    hit: ShapeHitInfo {
                        body_id: Some(body.id),
                        material_id: body.material_id,
                        shape_key: u32::MAX,
                        filter_info: body.filter_info,
                        user_data: body.user_data,
                    },
                    materials: resource.materials,
                },
                self.point,
                self.query,
                self.filter,
                self.contacts,
            )
        })();
        if let Err(error) = result {
            self.error = Some(error);
            control.early_out = true;
        }
    }
}
