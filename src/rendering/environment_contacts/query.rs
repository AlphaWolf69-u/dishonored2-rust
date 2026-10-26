//! Render-view806c30 query ownership; completion is consumed by88d060.
use crate::gameplay::{
    collision_queries::{CollisionQueries, QueryError, QueryId},
    shape_point_query::ShapePointContact,
    world_point_query::WorldPointRequest,
};

pub struct EnvironmentViewQuery {
    pub id: QueryId,
}
impl Default for EnvironmentViewQuery {
    fn default() -> Self {
        Self {
            id: QueryId::INVALID,
        }
    }
}
impl EnvironmentViewQuery {
    /// Always cancel an unconsumed request first, including when this view
    /// stops being active. A missing world remains an invalid query, not a
    /// completed empty contact list.
    pub fn submit(
        &mut self,
        queries: &mut CollisionQueries<WorldPointRequest, ShapePointContact>,
        active: bool,
        world_available: bool,
        camera_position: [f32; 3],
    ) -> Result<(), QueryError> {
        if self.id.is_valid() {
            queries.cancel(&mut self.id)?;
        }
        if active {
            self.id = queries.submit(
                WorldPointRequest::environment(camera_position),
                world_available,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_requests_match_executed_active_camera_submission() {
        for row in include_str!("../../../tests/fixtures/environment_view_query.txt").lines() {
            let mut words = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap());
            let active = words.next().unwrap() != 0;
            let available = words.next().unwrap() != 0;
            let prior = words.next().unwrap() != 0;
            let point = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
            let expected_valid = words.next().unwrap() != 0;
            let n = words.next().unwrap() as usize;
            let calls: Vec<_> = words.by_ref().take(n).collect();
            assert_eq!(
                calls,
                prior
                    .then_some(0)
                    .into_iter()
                    .chain(active.then_some(1))
                    .collect::<Vec<_>>()
            );
            let mut queries = CollisionQueries::default();
            queries.set_submission_allowed(true);
            let mut view = EnvironmentViewQuery::default();
            if prior {
                view.submit(&mut queries, true, true, [0.; 3]).unwrap();
            }
            view.submit(&mut queries, active, available, point).unwrap();
            assert_eq!(view.id.is_valid(), expected_valid);
            let mut executed = 0;
            queries.resolve_pending(|request, _| {
                executed += 1;
                assert_eq!(request.point.map(f32::to_bits), point.map(f32::to_bits));
                assert_eq!(request.filter.unwrap().layers, 0x410000);
            });
            assert_eq!(executed, usize::from(expected_valid));
            if active {
                let request = WorldPointRequest::environment(point);
                let filter = request.filter.unwrap();
                let mut packet = vec![
                    filter.layers,
                    filter.required_flags,
                    u32::from(request.hit.material_id),
                    request.hit.filter_info,
                    request.hit.user_data as u32,
                    2,
                ]; // zero-distance all-hits point type
                packet.extend(point.map(f32::to_bits));
                packet.extend([0, 0]); // homogeneous point W and query distance
                assert_eq!(packet, words.collect::<Vec<_>>());
            } else {
                assert!(words.next().is_none());
            }
        }
    }
}
