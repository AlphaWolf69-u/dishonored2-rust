//! Actual retained-shape contact leaf adapter. Traversal borrows the same HKX
//! shapes as publication and queries; only numerical GSK/manifold scratch lives
//! here. Packed mesh bounds select primitives, never replace their geometry.
use super::{
    contacts::{ContactGeometry, ManifoldCandidates},
    storage::{material_sources::MaterialSourceView, WorldStorage},
};
use crate::{
    assets::{
        entities::triggers::TriggerAssets,
        physworld::{
            compressed_mesh::PrimitiveKind,
            polytope::ConvexPolytope,
            query_geometry::{QueryGeometry, QueryShapes},
        },
    },
    gameplay::{
        collision_body::CollisionBody,
        collision_filter::body_pair_collides,
        collision_hit::ShapeHitInfo,
        compound_query::instance::{
            prepare_instance, CompoundChildGeometry, CompoundShapeState, ShapeKey,
        },
        compressed_mesh_query::{support_input, support_leaves, CompressedMeshQuery, MeshSweep},
        constraint_solver::{contact::correspondence::FacePair, reciprocal},
        convex_support::{
            contact_capsule::{CapsulePolytope, CAPSULE_FLAGS},
            contact_faces::{self, SelectedFace, TriangleFaces},
            contact_faces_support,
            distance::ConvexDistanceSolution,
            manifold::{ClipFace, FaceClipInput, ManifoldWorkspace},
            pair_distance::{solve_pair_distance_with_settings, PairCache, PairSettings},
            ConvexSupportInput, ConvexSupportScale, ConvexSupportWorkspace,
        },
        ray_query::RayQueryControl,
        shape_tags::{decode_shape_tag, ShapeTagSource},
        shape_transform::{
            inverse_rigid, inverse_rotate_vector, relative_rigid, rotate_vector, transform_bounds,
            transform_point, ScaledShapeTransform, ShapeMatrix,
        },
    },
};
use std::collections::HashMap;

#[derive(Clone, Copy)]
struct Leaf<'a> {
    input: ConvexSupportInput<'a>,
    polytope: Option<&'a ConvexPolytope>,
    frame: ShapeMatrix,
    hit: ShapeHitInfo,
    flags: u16,
}
#[derive(Default)]
struct Traversal {
    mesh: CompressedMeshQuery,
    active: Vec<usize>,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    bodies: [u32; 2],
    resources: [usize; 2],
    shapes: [u32; 2],
}
struct Cache {
    gsk: PairCache,
    faces: [Option<u32>; 2],
    /// d14cb0/1412b2330's actual persistent-cache+1c cold marker.
    marker: u32,
    serial: u64,
}
struct PairWorkspace {
    supports: [ConvexSupportWorkspace; 2],
    triangles: [TriangleFaces; 2],
    capsules: [CapsulePolytope; 2],
    vertices: [[[f32; 4]; 16]; 2],
    clip: ManifoldWorkspace,
    caches: HashMap<Key, Cache>,
}
impl Default for PairWorkspace {
    fn default() -> Self {
        Self {
            supports: std::array::from_fn(|_| Default::default()),
            triangles: std::array::from_fn(|_| Default::default()),
            capsules: std::array::from_fn(|_| Default::default()),
            vertices: [[[0.; 4]; 16]; 2],
            clip: Default::default(),
            caches: Default::default(),
        }
    }
}
#[derive(Default)]
pub(crate) struct GeometryWorkspace {
    traversal: [Traversal; 2],
    pair: PairWorkspace,
    serial: u64,
}
impl GeometryWorkspace {
    pub(crate) fn invalidate_body(&mut self, id: u32) {
        self.pair.caches.retain(|key, _| !key.bodies.contains(&id));
    }
    pub(crate) fn clear(&mut self) {
        self.pair.caches.clear();
    }
    pub(crate) fn begin(&mut self, world: &WorldStorage) {
        self.serial = self.serial.wrapping_add(1);
        self.pair
            .caches
            .retain(|key, _| key.bodies.iter().all(|&id| world.bodies.get(id).is_some()));
    }
    pub(crate) fn retire_unseen(&mut self, world: &WorldStorage) {
        self.pair.caches.retain(|key, cache| {
            cache.serial == self.serial
                || key.bodies.iter().all(|&id| {
                    world
                        .bodies
                        .get(id)
                        .is_some_and(|body| body.active_list_index == u32::MAX)
                })
        });
    }
}

fn state(body: &CollisionBody) -> CompoundShapeState {
    CompoundShapeState {
        transform: ScaledShapeTransform {
            world_from_local: body.transform,
            scale: [1.; 4],
            offset: [0.; 4],
        },
        scaled: false,
        keep_radius: false,
        radius: 0.,
    }
}
fn hit(body: &CollisionBody) -> ShapeHitInfo {
    ShapeHitInfo {
        body_id: Some(body.id),
        material_id: body.material_id,
        shape_key: u32::MAX,
        filter_info: body.filter_info,
        user_data: body.user_data,
    }
}
fn identity() -> ShapeMatrix {
    [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 0.],
    ]
}

fn local_bounds(state: CompoundShapeState, world: [[f32; 4]; 2], tolerance: f32) -> [[f32; 4]; 2] {
    let mut bounds = transform_bounds(inverse_rigid(state.transform.world_from_local), world);
    for axis in 0..3 {
        let inverse = if state.scaled {
            reciprocal(state.transform.scale[axis])
        } else {
            1.
        };
        let offset = if state.scaled {
            state.transform.offset[axis]
        } else {
            0.
        };
        let a = (bounds[0][axis] - offset) * inverse;
        let b = (bounds[1][axis] - offset) * inverse;
        let expansion = tolerance * inverse.abs();
        bounds[0][axis] = a.min(b) - expansion;
        bounds[1][axis] = a.max(b) + expansion;
    }
    bounds
}

fn compound_leaves(
    compound: &crate::assets::physworld::compound::CompoundShape,
    index: u16,
    bounds: [[f32; 4]; 2],
    remaining: &mut usize,
    submit: &mut dyn FnMut(u16) -> Result<(), String>,
) -> Result<(), String> {
    if *remaining == 0 {
        return Err("compound contact BVH does not terminate".into());
    }
    *remaining -= 1;
    let node = compound
        .nodes
        .get(usize::from(index))
        .ok_or("compound contact BVH node absent")?;
    if (0..3).any(|i| node.bounds[1][i] < bounds[0][i] || bounds[1][i] < node.bounds[0][i]) {
        return Ok(());
    }
    use crate::assets::physworld::compound::types::CompoundChildren;
    match node.children {
        CompoundChildren::Branch { left, right } => {
            compound_leaves(compound, left, bounds, remaining, submit)?;
            compound_leaves(compound, right, bounds, remaining, submit)
        }
        CompoundChildren::Instance(index) => submit(index),
    }
}

/// Visit enabled compound instances in authored order. Each recursive frame
/// owns only its key/scale/transform; all topology remains in QueryShapes.
fn visit(
    shapes: &QueryShapes,
    item: usize,
    state: CompoundShapeState,
    key: ShapeKey,
    hit: ShapeHitInfo,
    other_filter: u32,
    other_world_bounds: [[f32; 4]; 2],
    tolerance: f32,
    materials: &dyn ShapeTagSource,
    traversal: &mut Traversal,
    submit: &mut dyn FnMut(Leaf<'_>) -> Result<(), String>,
) -> Result<(), String> {
    if traversal.active.contains(&item) {
        return Err("cyclic retained contact shape graph".into());
    }
    traversal.active.push(item);
    let result = (|| {
        let shape = shapes.get(item)?;
        match &shape.geometry {
            QueryGeometry::Convex(convex) => submit(Leaf {
                input: ConvexSupportInput {
                    shape_type: shape.header.shape_type,
                    vertices: &convex.vertices,
                    radius: shape.header.radius,
                    scaled: state.scaled.then_some(ConvexSupportScale {
                        scale: state.transform.scale[..3].try_into().unwrap(),
                        offset: state.transform.offset[..3].try_into().unwrap(),
                        radius: state.radius,
                    }),
                },
                polytope: None,
                frame: state.transform.world_from_local,
                hit,
                flags: shape.header.flags,
            }),
            QueryGeometry::Polytope(polytope) => submit(Leaf {
                input: ConvexSupportInput {
                    shape_type: shape.header.shape_type,
                    vertices: &polytope.vertices,
                    radius: shape.header.radius,
                    scaled: state.scaled.then_some(ConvexSupportScale {
                        scale: state.transform.scale[..3].try_into().unwrap(),
                        offset: state.transform.offset[..3].try_into().unwrap(),
                        radius: state.radius,
                    }),
                },
                polytope: Some(polytope),
                frame: state.transform.world_from_local,
                hit,
                flags: shape.header.flags,
            }),
            QueryGeometry::Compound(compound) => {
                if compound.root == 0 {
                    return Ok(());
                }
                let bounds = local_bounds(state, other_world_bounds, tolerance);
                compound_leaves(
                    compound,
                    compound.root,
                    bounds,
                    &mut compound.nodes.len(),
                    &mut |index| {
                        let instance = compound
                            .instances
                            .get(usize::from(index))
                            .ok_or("compound contact instance absent")?;
                        if instance.flags & 64 == 0 {
                            return Ok(());
                        }
                        let next_key = key.compound_child(index, compound.key_bits)?;
                        let mut next_hit = ShapeHitInfo {
                            shape_key: next_key.value,
                            ..hit
                        };
                        decode_shape_tag(instance.shape_tag, Some(materials), &mut next_hit)?;
                        if !body_pair_collides(other_filter, next_hit.filter_info) {
                            return Ok(());
                        }
                        let child = shapes.get(instance.shape)?;
                        let vertices = match &child.geometry {
                            QueryGeometry::Polytope(p) => p.vertices.as_slice(),
                            QueryGeometry::Convex(p) => p.vertices.as_slice(),
                            _ => &[],
                        };
                        let prepared = prepare_instance(
                            instance,
                            state,
                            identity(),
                            CompoundChildGeometry {
                                convex: child.header.flags & 1 != 0,
                                shape_type: child.header.shape_type,
                                vertices,
                                radius: child.header.radius,
                            },
                        )?;
                        visit(
                            shapes,
                            instance.shape,
                            prepared.state,
                            next_key,
                            next_hit,
                            other_filter,
                            other_world_bounds,
                            tolerance,
                            materials,
                            traversal,
                            submit,
                        )
                    },
                )
            }
            QueryGeometry::CompressedMesh(mesh) => {
                let bounds = local_bounds(state, other_world_bounds, tolerance);
                let sweep = MeshSweep {
                    center: std::array::from_fn(|i| (bounds[1][i] + bounds[0][i]) * 0.5),
                    half_extent: std::array::from_fn(|i| (bounds[1][i] - bounds[0][i]) * 0.5),
                    direction: [0.; 3],
                };
                let mut control = RayQueryControl {
                    fraction: 1.,
                    early_out: false,
                };
                traversal.mesh.visit_swept(
                    mesh,
                    sweep,
                    shape.header.radius,
                    &mut control,
                    &mut |leaf, _| {
                        support_leaves(leaf, &mut |leaf| {
                            let next_key = key.child(leaf.key, shape.header.key_bits)?;
                            let mut next_hit = ShapeHitInfo {
                                shape_key: next_key.value,
                                ..hit
                            };
                            decode_shape_tag(leaf.tag, Some(materials), &mut next_hit)?;
                            if !body_pair_collides(other_filter, next_hit.filter_info) {
                                return Ok(());
                            }
                            let input = support_input(&leaf, state)?;
                            let flags = match leaf.kind {
                                PrimitiveKind::Triangle => 0x3c3,
                                PrimitiveKind::Quad => 0x7c3,
                                PrimitiveKind::Convex => match leaf.convex_kind {
                                    // Actual getLeafShape14124e030 constructors:
                                    // sphere ca6550, capsule cb4830, convex shape.
                                    Some(0) => 0x111,
                                    Some(1) => 0x103,
                                    Some(2) => 1,
                                    _ => {
                                        return Err("contact compressed convex kind invalid".into());
                                    }
                                },
                                PrimitiveKind::Empty => {
                                    return Err(
                                        "empty mesh primitive reached contact generator".into()
                                    );
                                }
                            };
                            submit(Leaf {
                                input,
                                polytope: None,
                                frame: state.transform.world_from_local,
                                hit: next_hit,
                                flags,
                            })
                        })
                    },
                )
            }
        }
    })();
    traversal.active.pop();
    result
}

fn normal_factor(v: [f32; 3]) -> f32 {
    let squared = (v[1] * v[1] + v[0] * v[0]) + v[2] * v[2];
    let r = crate::gameplay::convex_support::faces::reciprocal_sqrt_estimate(squared);
    ((3. - r * squared * r) * r) * 0.5
}
fn uniform(scale: [f32; 3]) -> bool {
    scale[0] == scale[1] && scale[0] == scale[2] && scale[0] >= 0.
}
fn divide_component(value: f32, divisor: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rcp_ss, _mm_set_ss};
        let estimate = unsafe { _mm_cvtss_f32(_mm_rcp_ss(_mm_set_ss(divisor))) };
        (value * (2. - divisor * estimate)) * estimate
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        unimplemented!("scaled contact face virtuals require x86-64 RCPPS");
    }
}

/// cb5510 maps direction and witness to the actual child before invoking its
/// face virtual. cb5170 scales its cyclic vertices and reverses odd reflections.
fn select_face(
    leaf: Leaf<'_>,
    polytope: Option<&ConvexPolytope>,
    cache: PairCache,
    target: bool,
    previous: Option<u32>,
    direction: [f32; 4],
    witness: [f32; 4],
    cosines: &[f32; 256],
    out: &mut [[f32; 4]; 16],
) -> Result<(SelectedFace, usize), String> {
    let mut direction: [f32; 3] = direction[..3].try_into().unwrap();
    let mut witness = witness;
    if let Some(scale) = leaf.input.scaled {
        let inverse = scale.scale.map(reciprocal);
        for i in 0..3 {
            witness[i] = (witness[i] - scale.offset[i]) * inverse[i];
        }
        if !uniform(scale.scale) {
            for i in 0..3 {
                direction[i] = divide_component(direction[i], inverse[i]);
            }
            let factor = normal_factor(direction);
            direction = direction.map(|v| v * factor);
        }
    }
    let mut selected = if let Some(polytope) = polytope {
        if leaf.input.shape_type == 5 {
            // ca15b0 composes the authored rotation with identity+center;
            // c0ba80 copies those columns to box+70. ca3970 transforms the
            // +X/+Y/+Z planes by the same rotation, so their retained XYZ
            // are precisely cc77d0's basis (W extents are unused there).
            let basis = [0, 2, 4].map(|i| {
                polytope
                    .planes
                    .get(i)
                    .copied()
                    .ok_or("box positive face plane missing")
            });
            contact_faces::select_box(polytope, [basis[0]?, basis[1]?, basis[2]?], direction)?
        } else {
            contact_faces::select(
                polytope, leaf.flags, direction, cache, target, previous, cosines,
            )?
        }
    } else {
        contact_faces_support::select(
            leaf.input.vertices,
            direction,
            witness,
            cache,
            target,
            previous,
        )?
    };
    let count = if let Some(polytope) = polytope {
        contact_faces::vertices(polytope, selected.index, out)?
    } else {
        contact_faces_support::vertices(leaf.input.vertices, selected.index, out)?
    };
    if let Some(scale) = leaf.input.scaled {
        if scale.scale.iter().filter(|&&v| v < 0.).count() & 1 != 0 {
            out[..count].reverse();
        }
        for vertex in &mut out[..count] {
            for i in 0..3 {
                vertex[i] = vertex[i] * scale.scale[i] + scale.offset[i];
            }
        }
        if uniform(scale.scale) {
            selected.plane[3] *= scale.scale[0];
        } else {
            let v: [f32; 3] =
                std::array::from_fn(|i| divide_component(selected.plane[i], scale.scale[i]));
            let squared = (v[1] * v[1] + v[0] * v[0]) + v[2] * v[2];
            let r = crate::gameplay::convex_support::faces::reciprocal_sqrt_estimate(squared);
            let correction = 3. - r * squared * r;
            for i in 0..3 {
                selected.plane[i] = ((v[i] * correction) * r) * 0.5;
            }
            selected.plane[3] *= (correction * r) * 0.5;
        }
        selected.plane[3] -= (scale.offset[1] * selected.plane[1]
            + scale.offset[0] * selected.plane[0])
            + scale.offset[2] * selected.plane[2];
    }
    for vertex in &mut out[..count] {
        *vertex = transform_point(leaf.frame, *vertex);
    }
    let normal = rotate_vector(leaf.frame, selected.plane);
    selected.plane = [
        normal[0],
        normal[1],
        normal[2],
        selected.plane[3]
            - ((normal[1] * leaf.frame[3][1] + normal[0] * leaf.frame[3][0])
                + normal[2] * leaf.frame[3][2]),
    ];
    Ok((selected, count))
}

impl PairWorkspace {
    fn generate(
        &mut self,
        key: Key,
        leaves: [Leaf<'_>; 2],
        world: &WorldStorage,
        contact_margin: f32,
        serial: u64,
        submit: &mut dyn FnMut(ManifoldCandidates<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        let cache = self.caches.entry(key).or_insert_with(|| Cache {
            gsk: Default::default(),
            faces: [None; 2],
            marker: 0xaaa00,
            serial,
        });
        let mut leaves: [Leaf<'_>; 2] = leaves;
        let [capsule_a, capsule_b] = &mut self.capsules;
        for (leaf, capsule) in leaves.iter_mut().zip([capsule_a, capsule_b]) {
            if leaf.polytope.is_none() && leaf.input.shape_type == 3 {
                let polytope = capsule.prepare(leaf.input.vertices, leaf.input.radius)?;
                leaf.input.vertices = &polytope.vertices;
                leaf.input.radius = polytope.radius;
                if let Some(scale) = &mut leaf.input.scaled {
                    // cb4830 constructs before ce2190 applies the wrapper's
                    // uniform capsule radius scale; keep that arithmetic order.
                    scale.radius = scale.scale[0].abs() * polytope.radius;
                }
                leaf.polytope = Some(polytope);
                leaf.flags = CAPSULE_FLAGS;
            }
        }
        let [a_support, b_support] = &mut self.supports;
        let a = a_support.prepare(leaves[0].input)?;
        let b = b_support.prepare(leaves[1].input)?;
        let relative = relative_rigid(leaves[0].frame, leaves[1].frame);
        let mut solution = ConvexDistanceSolution {
            status: 0,
            distance: [0.; 4],
            normal: [0.; 4],
            witness: [0.; 4],
        };
        solve_pair_distance_with_settings(
            a.vertices,
            b.vertices,
            relative,
            &mut cache.gsk,
            //1412abc30's extended GSK transform+50 is the pair's two
            // radii plus contact tolerance; +60 is141b10c28(1.4).
            PairSettings {
                distance_limit: (a.radius + b.radius) + contact_margin,
                ..PairSettings::default()
            },
            &mut solution,
        )?;
        if solution.status > 4 || solution.distance[0] - (a.radius + b.radius) >= contact_margin {
            self.caches.remove(&key);
            return Ok(());
        }
        let normal = rotate_vector(leaves[0].frame, solution.normal);
        let bodies = key
            .bodies
            .map(|id| world.bodies.get(id).ok_or("contact geometry body retired"));
        let bodies = [bodies[0]?, bodies[1]?];
        let quality = bodies.map(|body| {
            world
                .qualities
                .get(body.quality_id)
                .ok_or("contact geometry quality absent")
        });
        let quality = [quality[0]?, quality[1]?];
        let mut quality_flags = if quality[0].priority < quality[1].priority {
            quality[1].requested_flags & quality[0].supported_flags
        } else {
            quality[0].requested_flags & quality[1].supported_flags
        } as u16;
        // d14cb0: only the actual shape or material policy enables the
        // source's single-point manifold branch (1412abc30+6401).
        if (leaves[0].flags | leaves[1].flags) & 0x10 != 0 {
            quality_flags |= 4;
        }
        let materials = leaves.map(|leaf| {
            world
                .materials
                .library
                .get(leaf.hit.material_id)
                .ok_or("contact leaf material absent")
        });
        if (materials[0]?.words[1] | materials[1]?.words[1]) & 0x20_0000 != 0 {
            quality_flags |= 4;
        }
        if quality_flags & 4 != 0 {
            let point = transform_point(
                leaves[0].frame,
                std::array::from_fn(|i| {
                    solution.witness[i] - (solution.distance[i] - b.radius) * solution.normal[i]
                }),
            );
            let candidates = [super::contacts::ManifoldCandidate {
                id: 0,
                projected_xy: [0.; 2],
                point_on_b: point,
                distance: solution.distance[0] - (a.radius + b.radius),
            }];
            cache.serial = serial;
            let new_manifold = cache.marker == 0xaaa00;
            submit(ManifoldCandidates {
                candidates: &candidates,
                selected: &[0],
                faces: FacePair {
                    a: 0,
                    b: 0,
                    compact: true,
                },
                normal: normal[..3].try_into().unwrap(),
                shape_keys: key.shapes,
                resource_shapes: key.resources,
                material_ids: leaves.map(|leaf| leaf.hit.material_id),
                new_manifold,
                separation: candidates[0].distance,
            })?;
            cache.marker = 0;
            return Ok(());
        }
        let witness_b = transform_point(
            relative,
            std::array::from_fn(|i| {
                solution.witness[i] - solution.distance[i] * solution.normal[i]
            }),
        );
        let local_normal_b = rotate_vector(relative, solution.normal).map(|v| -v);
        let [triangle_a, triangle_b] = &mut self.triangles;
        let a_polytope = if leaves[0].input.shape_type == 4 {
            Some(triangle_a.prepare(leaves[0].input.vertices, a.radius)?)
        } else {
            leaves[0].polytope
        };
        let b_polytope = if leaves[1].input.shape_type == 4 {
            Some(triangle_b.prepare(leaves[1].input.vertices, b.radius)?)
        } else {
            leaves[1].polytope
        };
        let [a_vertices, b_vertices] = &mut self.vertices;
        let (a_face, a_count) = select_face(
            leaves[0],
            a_polytope,
            cache.gsk,
            false,
            cache.faces[0],
            solution.normal,
            solution.witness,
            world.materials.library.contact_face_cosines(),
            a_vertices,
        )?;
        let (b_face, b_count) = select_face(
            leaves[1],
            b_polytope,
            cache.gsk,
            true,
            cache.faces[1],
            local_normal_b,
            witness_b,
            world.materials.library.contact_face_cosines(),
            b_vertices,
        )?;
        let manifold = self.clip.clip(FaceClipInput {
            a: ClipFace {
                vertices: &a_vertices[..a_count],
                plane: a_face.plane,
            },
            b: ClipFace {
                vertices: &b_vertices[..b_count],
                plane: b_face.plane,
            },
            normal,
            radii: [a.radius, b.radius],
            tolerance: contact_margin,
            // ce5740 shared+10->world+6d0; c8c250 Cinfo+cc->SolverInfo+90.
            reduction_tolerance: world.info.relative_collision_accuracy,
        })?;
        if manifold.selected.is_empty() {
            self.caches.remove(&key);
            return Ok(());
        }
        cache.faces = [Some(a_face.index), Some(b_face.index)];
        cache.serial = serial;
        let new_manifold = cache.marker == 0xaaa00;
        // 1412abc30+8722/8759 publishes manifold+4 from selected point
        // distances; short manifolds pad the remaining cache IDs with slot0.
        let distances: [f32; 4] = std::array::from_fn(|i| {
            manifold.candidates
                [usize::from(*manifold.selected.get(i).unwrap_or(&manifold.selected[0]))]
            .distance
        });
        let min = |a: f32, b: f32| if a < b { a } else { b };
        let separation = min(
            min(distances[2], distances[0]),
            min(distances[3], distances[1]),
        );
        submit(ManifoldCandidates {
            candidates: manifold.candidates,
            selected: manifold.selected,
            faces: FacePair {
                a: a_face.index,
                b: b_face.index,
                compact: leaves[0].flags & leaves[1].flags & 0x100 != 0,
            },
            normal: normal[..3].try_into().unwrap(),
            shape_keys: key.shapes,
            resource_shapes: key.resources,
            material_ids: leaves.map(|leaf| leaf.hit.material_id),
            new_manifold,
            separation,
        })?;
        cache.marker = 0;
        Ok(())
    }
}

impl ContactGeometry for TriggerAssets {
    fn contact_manifolds(
        &mut self,
        world: &WorldStorage,
        bodies: [&CollisionBody; 2],
        context_flags: u8,
        workspace: &mut GeometryWorkspace,
        submit: &mut dyn FnMut(ManifoldCandidates<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        let resources = [
            bodies[0].selected_shape(usize::from((context_flags >> 4) & 7))?,
            bodies[1].selected_shape(usize::from(context_flags & 7))?,
        ];
        let shapes = resources.map(|resource| {
            self.shapes
                .get(resource)
                .and_then(Option::as_ref)
                .ok_or("contact shape resource retired")
        });
        let shapes = [shapes[0]?, shapes[1]?];
        // d193f0 pairInput+30: sum body+7c swept contact distances, decoded
        // with solverInfo+70. c9f8e0 writes that reciprocal using one Newton
        // step; the ordinary world's collision tolerance alone is too small
        // for a moving body's speculative manifold.
        let contact_margin = (f32::from(bodies[0].max_contact_distance)
            + f32::from(bodies[1].max_contact_distance))
            * reciprocal(world.step.contact_distance_scale[0]);
        let a_bounds = shapes[0]
            .query_shapes
            .bounds(shapes[0].query_shapes.root(), bodies[0].transform)?;
        let b_bounds = shapes[1]
            .query_shapes
            .bounds(shapes[1].query_shapes.root(), bodies[1].transform)?;
        let materials = MaterialSourceView {
            sources: &world.material_sources,
            materials: &world.materials,
        };
        let [a_traversal, b_traversal] = &mut workspace.traversal;
        let pair = &mut workspace.pair;
        let serial = workspace.serial;
        visit(
            &shapes[0].query_shapes,
            shapes[0].query_shapes.root(),
            state(bodies[0]),
            ShapeKey {
                value: u32::MAX,
                bits: 0,
            },
            hit(bodies[0]),
            bodies[1].filter_info,
            b_bounds,
            contact_margin,
            &materials,
            a_traversal,
            &mut |a| {
                visit(
                    &shapes[1].query_shapes,
                    shapes[1].query_shapes.root(),
                    state(bodies[1]),
                    ShapeKey {
                        value: u32::MAX,
                        bits: 0,
                    },
                    hit(bodies[1]),
                    a.hit.filter_info,
                    a_bounds,
                    contact_margin,
                    &materials,
                    b_traversal,
                    &mut |b| {
                        if !body_pair_collides(a.hit.filter_info, b.hit.filter_info) {
                            return Ok(());
                        }
                        pair.generate(
                            Key {
                                bodies: bodies.map(|body| body.id),
                                resources,
                                shapes: [a.hit.shape_key, b.hit.shape_key],
                            },
                            [a, b],
                            world,
                            contact_margin,
                            serial,
                            submit,
                        )
                    },
                )
            },
        )
    }
}
