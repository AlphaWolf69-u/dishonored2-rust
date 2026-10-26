use super::{point_tests::compound_query_geometry, QueryGeometry};
use crate::gameplay::{
    body_storage::BodyStorage,
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    collision_queries::{CollisionQueries, QueryId},
    point_query::{
        PointNodeChildren, PointQuantization, PointQueryBody, PointQueryNode, PointQueryTree,
        QuantizedBounds,
    },
    world_point_query::{WorldPointQuery, WorldPointRequest, WorldQueryShape},
};

#[test]
fn world_points_match_executed_body_filter_and_geometry_composition() {
    let mut shapes = compound_query_geometry();
    let mut query = WorldPointQuery::new(0x1ffffff).unwrap();
    let mut contacts = Vec::new();
    let bounds = QuantizedBounds {
        min: [0; 3],
        max: [255; 3],
    };
    let nodes = [PointQueryNode {
        bounds,
        children: PointNodeChildren::Leaf { block: 0 },
    }];
    let quantization = PointQuantization {
        offset: [128.; 3],
        scale: [1.; 3],
    };
    let mut cases = 0;
    let mut hits = 0;
    for row in include_str!("../../../../tests/fixtures/world_point_query.txt").lines() {
        let w: Vec<u32> = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let QueryGeometry::Compound(compound) = &mut shapes.shapes.get_mut(&1).unwrap().geometry
        else {
            unreachable!()
        };
        for (i, instance) in compound.instances.iter_mut().enumerate() {
            instance.flags = 0x3f000000 | if w[3] >> i & 1 != 0 { 64 } else { 0 };
            instance.transform[0][3] = f32::from_bits(instance.flags);
        }
        let mut bodies = BodyStorage::new(4).unwrap();
        for i in 0..3 {
            let id = bodies.reserve(&mut []).unwrap();
            let body = bodies.get_mut(id).unwrap();
            let values = &w[10 + i * 21..];
            body.transform = std::array::from_fn(|col| {
                std::array::from_fn(|row| f32::from_bits(values[col * 4 + row]))
            });
            body.id = values[16];
            body.shape = 1;
            body.material_id = values[17] as u16;
            body.filter_info = values[18];
            body.user_data = u64::from(values[19]) | (u64::from(values[20]) << 32);
        }
        let blocks = [vec![
            PointQueryBody {
                bounds,
                index: w[7],
            },
            PointQueryBody {
                bounds,
                index: w[8],
            },
        ]];
        let second = [vec![PointQueryBody {
            bounds,
            index: w[9],
        }]];
        let trees = [
            PointQueryTree {
                root: Some(0),
                nodes: &nodes,
                blocks: &blocks,
            },
            PointQueryTree {
                root: Some(0),
                nodes: &nodes,
                blocks: &second,
            },
        ];
        contacts.clear();
        let request = WorldPointRequest {
            point: std::array::from_fn(|i| f32::from_bits(w[i])),
            hit: ShapeHitInfo {
                body_id: None,
                material_id: 0xffff,
                shape_key: u32::MAX,
                filter_info: 0,
                user_data: 0,
            },
            filter: (w[4] != 0).then_some(CollisionQueryFilter {
                layers: w[5],
                required_flags: w[6],
            }),
        };
        let resolve = |index| {
            assert_eq!(index, 1);
            Ok(WorldQueryShape {
                shapes: &shapes,
                materials: None,
            })
        };
        if cases & 1 == 0 {
            query
                .all_hits(
                    &bodies,
                    &trees,
                    quantization,
                    request.point,
                    request.hit,
                    request.filter,
                    resolve,
                    &mut contacts,
                )
                .unwrap();
        } else {
            let mut pending = CollisionQueries::default();
            let mut id = pending.submit(request, true).unwrap();
            if w[73] != 0 {
                // A geometry failure must not turn an occupied volume into a
                // successful dry result, or consume its query identity.
                assert!(query
                    .resolve_pending(&mut pending, &bodies, &trees, quantization, |_| Err(
                        "missing retained geometry".into()
                    ))
                    .is_err());
                assert!(!pending.take_completed(&mut id, &mut contacts).unwrap());
                assert!(id.is_valid());
                assert!(contacts.is_empty());
            }
            query
                .resolve_pending(&mut pending, &bodies, &trees, quantization, resolve)
                .unwrap();
            assert!(pending.take_completed(&mut id, &mut contacts).unwrap());
            assert_eq!(id, QueryId::INVALID);
        }
        assert_eq!(contacts.len(), w[73] as usize, "case {cases}");
        let actual: Vec<_> = contacts
            .iter()
            .flat_map(|contact| {
                let hit = contact.target;
                [
                    hit.body_id.unwrap(),
                    u32::from(hit.material_id),
                    hit.shape_key,
                    hit.filter_info,
                    hit.user_data as u32,
                    (hit.user_data >> 32) as u32,
                ]
            })
            .collect();
        assert_eq!(
            actual,
            w[74..],
            "case {cases}: preserve native body/leaf order and metadata"
        );
        hits += contacts.len();
        cases += 1;
    }
    assert_eq!(cases, 512);
    assert!(hits > 0);
}
