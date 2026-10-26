use super::*;
use crate::{
    assets::physworld::{
        compound::{
            types::{CompoundChildren, CompoundInstance, CompoundNode},
            CompoundShape,
        },
        polytope::PolytopeConnectivity,
    },
    gameplay::{
        collision_filter::CollisionQueryFilter,
        collision_hit::ShapeHitInfo,
        compound_query::instance::CompoundShapeState,
        shape_point_query::{PointQueryTarget, ShapePointQuery},
        shape_transform::ScaledShapeTransform,
    },
};

pub(super) fn compound_query_geometry() -> QueryShapes {
    let header = |flags, shape_type, key_bits| ShapeHeader {
        flags,
        shape_type,
        key_bits,
        dispatch_type: 0,
        radius: 0.,
        user_data: 0,
    };
    let mut instances = Vec::new();
    let mut nodes = Vec::new();
    for row in include_str!("../../../../tests/fixtures/shape_point_query_geometry.txt").lines() {
        let mut parts = row.split_whitespace();
        let kind = parts.next().unwrap();
        let index: usize = parts.next().unwrap().parse().unwrap();
        let w: Vec<_> = parts.map(|s| u32::from_str_radix(s, 16).unwrap()).collect();
        match kind {
            "instance" => {
                assert_eq!(index, instances.len());
                instances.push(CompoundInstance {
                    transform: std::array::from_fn(|i| {
                        std::array::from_fn(|j| f32::from_bits(w[i * 4 + j]))
                    }),
                    flags: 64,
                    scale: [1.; 4],
                    shape: index + 2,
                    shape_tag: 0xffff,
                });
            }
            "node" => {
                assert_eq!(index, nodes.len());
                nodes.push(CompoundNode {
                    bounds: std::array::from_fn(|i| {
                        std::array::from_fn(|j| f32::from_bits(w[i * 3 + j]))
                    }),
                    children: if w[6] == 0 {
                        CompoundChildren::Instance(w[7] as u16)
                    } else {
                        CompoundChildren::Branch {
                            left: w[6] as u16,
                            right: w[7] as u16,
                        }
                    },
                });
            }
            _ => panic!("invalid geometry record"),
        }
    }
    let bounds = nodes[1].bounds.map(|v| [v[0], v[1], v[2], 0.]);
    let mut shapes = BTreeMap::new();
    shapes.insert(
        1,
        QueryShape {
            header: header(4, 12, 3),
            geometry: QueryGeometry::Compound(CompoundShape {
                bounds,
                key_bits: 3,
                is_mutable: false,
                instances,
                nodes,
                root: 1,
            }),
        },
    );
    for i in 0..4 {
        shapes.insert(
            i + 2,
            QueryShape {
                header: header(2371, 1, 0),
                geometry: QueryGeometry::Polytope(ConvexPolytope {
                    vertices: (0..8)
                        .map(|j| {
                            [
                                if j & 1 == 0 { -0.5 } else { 0.5 },
                                if j & 2 == 0 { -0.625 } else { 0.625 },
                                if j & 4 == 0 { -0.5 } else { 0.5 },
                                f32::from_bits(0x3f000000 | j),
                            ]
                        })
                        .collect(),
                    planes: vec![
                        [1., 0., 0., -0.5],
                        [-1., 0., 0., -0.5],
                        [0., 1., 0., -0.625],
                        [0., -1., 0., -0.625],
                        [0., 0., 1., -0.5],
                        [0., 0., -1., -0.5],
                    ],
                    faces: Vec::new(),
                    indices: Vec::new(),
                    radius: 0.,
                    connectivity: Some(PolytopeConnectivity {
                        vertex_edges: Vec::new(),
                        face_links: Vec::new(),
                    }),
                }),
            },
        );
    }
    QueryShapes { root: 1, shapes }
}

#[test]
fn shape_points_match_complete_executed_compound_queries() {
    let mut shapes = compound_query_geometry();
    let mut query = ShapePointQuery::new(0x1ffffff).unwrap();
    let mut world_query =
        crate::gameplay::world_point_query::WorldPointQuery::new(0x1ffffff).unwrap();
    let mut bodies = crate::gameplay::body_storage::BodyStorage::new(2).unwrap();
    let body_id = bodies.reserve(&mut []).unwrap();
    let body = bodies.get_mut(body_id).unwrap();
    body.shape = 83;
    body.material_id = 0xffff;
    body.user_data = 0x1234_5678_9abc_def0;
    let nodes = [crate::gameplay::point_query::PointQueryNode {
        bounds: crate::gameplay::point_query::QuantizedBounds {
            min: [0; 3],
            max: [255; 3],
        },
        children: crate::gameplay::point_query::PointNodeChildren::Leaf { block: 0 },
    }];
    let blocks = [vec![crate::gameplay::point_query::PointQueryBody {
        bounds: nodes[0].bounds,
        index: body_id & 0xffffff,
    }]];
    let trees = [crate::gameplay::point_query::PointQueryTree {
        root: Some(0),
        nodes: &nodes,
        blocks: &blocks,
    }];
    let quantization = crate::gameplay::point_query::PointQuantization {
        offset: [128.; 3],
        scale: [1.; 3],
    };
    let mut world_contacts = Vec::new();
    let mut contacts = Vec::new();
    let mut cases = 0;
    let mut accepted = 0;
    for row in include_str!("../../../../tests/fixtures/shape_point_query.txt").lines() {
        let w: Vec<_> = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let QueryGeometry::Compound(compound) = &mut shapes.shapes.get_mut(&1).unwrap().geometry
        else {
            unreachable!()
        };
        for (i, instance) in compound.instances.iter_mut().enumerate() {
            instance.flags = if w[19] >> i & 1 != 0 {
                0x3f000040
            } else {
                0x3f000000
            };
            if w[25] >> i & 1 != 0 {
                instance.flags |= 8;
            }
            instance.transform[0][3] = f32::from_bits(instance.flags);
            instance.scale = [
                f32::from_bits(w[26 + i * 3]),
                f32::from_bits(w[27 + i * 3]),
                f32::from_bits(w[28 + i * 3]),
                1.,
            ];
        }
        for i in 0..4 {
            let child = shapes.shapes.get_mut(&(i + 2)).unwrap();
            child.header.flags = if w[24] >> i & 1 != 0 { 259 } else { 2371 };
            let QueryGeometry::Polytope(polytope) = &mut child.geometry else {
                unreachable!()
            };
            polytope.connectivity = (w[24] >> i & 1 == 0).then_some(PolytopeConnectivity {
                vertex_edges: Vec::new(),
                face_links: Vec::new(),
            });
        }
        let target = ShapeHitInfo {
            body_id: None,
            material_id: 0xffff,
            shape_key: 0xffffffff,
            filter_info: w[23],
            user_data: 0,
        };
        contacts.clear();
        query
            .all_hits(
                PointQueryTarget {
                    shapes: &shapes,
                    state: CompoundShapeState {
                        transform: ScaledShapeTransform {
                            world_from_local: std::array::from_fn(|i| {
                                std::array::from_fn(|j| f32::from_bits(w[i * 4 + j]))
                            }),
                            scale: [1.; 4],
                            offset: [0.; 4],
                        },
                        scaled: false,
                        keep_radius: false,
                        radius: 0.,
                    },
                    hit: target,
                    materials: None,
                },
                [
                    f32::from_bits(w[16]),
                    f32::from_bits(w[17]),
                    f32::from_bits(w[18]),
                    0.,
                ],
                ShapeHitInfo {
                    filter_info: 0,
                    ..target
                },
                (w[20] != 0).then_some(CollisionQueryFilter {
                    layers: w[21],
                    required_flags: w[22],
                }),
                &mut contacts,
            )
            .unwrap();
        assert_eq!(contacts.len(), w[38] as usize, "case {cases}");
        let body = bodies.get_mut(body_id).unwrap();
        body.transform =
            std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(w[i * 4 + j])));
        body.filter_info = w[23];
        world_contacts.clear();
        world_query
            .all_hits(
                &bodies,
                &trees,
                quantization,
                [
                    f32::from_bits(w[16]),
                    f32::from_bits(w[17]),
                    f32::from_bits(w[18]),
                ],
                ShapeHitInfo {
                    filter_info: 0,
                    ..target
                },
                (w[20] != 0).then_some(CollisionQueryFilter {
                    layers: w[21],
                    required_flags: w[22],
                }),
                |shape| {
                    assert_eq!(shape, 83);
                    Ok(crate::gameplay::world_point_query::WorldQueryShape {
                        shapes: &shapes,
                        materials: None,
                    })
                },
                &mut world_contacts,
            )
            .unwrap();
        assert_eq!(world_contacts.len(), contacts.len(), "world case {cases}");
        for (world, shape) in world_contacts.iter().zip(&contacts) {
            assert_eq!(world.target.body_id, Some(body_id));
            assert_eq!(world.target.user_data, 0x1234_5678_9abc_def0);
            assert_eq!(world.target.material_id, shape.target.material_id);
            assert_eq!(world.target.shape_key, shape.target.shape_key);
            assert_eq!(world.target.filter_info, shape.target.filter_info);
        }
        assert_eq!(
            contacts
                .iter()
                .map(|h| h.target.shape_key)
                .collect::<Vec<_>>(),
            w[39..],
            "case {cases}"
        );
        accepted += contacts.len();
        cases += 1;
    }
    assert_eq!(cases, 1536);
    assert!(accepted > 0);
}
