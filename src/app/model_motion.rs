//! Publish one physics pose consistently to color geometry and both shadow paths.
use super::{
    initial_placed_model_query, placed_transform, render_bounds, render_model,
    shadow_updates::ModelShadowSources, PlacedModelQueries, RenderSourcePlacements,
    SceneRenderQueue, ShadowModelBoundsSources,
};
use bevy::prelude::*;
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub(super) struct ModelPose {
    pub placement: usize,
    /// Z-up rigid world delta from the body's initial pose. Geometry may have
    /// additional authored scale, shear or a blueprint offset from that body.
    pub delta: [[f32; 4]; 3],
}

#[derive(Resource, Default)]
pub(super) struct ModelPoseUpdates(pub Vec<ModelPose>);

struct SurfaceBinding {
    index: usize,
    /// World-baked vertices retain their initial pose. Only their rigid delta
    /// changes: no new meshes, vertex copies or per-frame asset allocations.
    baked: bool,
    initial_transform: Transform,
    initial_bounds: bevy::camera::primitives::Aabb,
}

struct Binding {
    initial_position: [f32; 3],
    initial_rotation: [[f32; 3]; 3],
    source: usize,
    shadow_bounds: Option<usize>,
    query: Option<usize>,
    surfaces: Vec<SurfaceBinding>,
    lighting: crate::probe_lighting::ProbeQuery,
}

#[derive(Resource)]
pub(super) struct ModelProbeLighting(pub Handle<super::ShaderBuffer>);

#[derive(Resource, Default)]
pub(super) struct ModelMotionBindings(HashMap<usize, Binding>);

#[cfg(test)]
fn rigid_matrix(position: [f32; 3], rotation: [[f32; 3]; 3]) -> Mat4 {
    let rows = render_model::affine_rows(position, rotation, [1.; 3]);
    super::game_affine_to_view(&std::array::from_fn(|i| rows[i / 4][i % 4])).to_matrix()
}

fn moved_bounds(
    bounds: bevy::camera::primitives::Aabb,
    delta: Mat4,
) -> bevy::camera::primitives::Aabb {
    let center = Vec3::from(bounds.center);
    let half = Vec3::from(bounds.half_extents);
    let corners: [Vec3; 8] = std::array::from_fn(|i| {
        let sign = Vec3::new(
            if i & 1 == 0 { -1. } else { 1. },
            if i & 2 == 0 { -1. } else { 1. },
            if i & 4 == 0 { -1. } else { 1. },
        );
        delta.transform_point3(center + half * sign)
    });
    render_bounds(&corners)
}

pub(super) fn publish_model_poses(
    mut pending: ResMut<ModelPoseUpdates>,
    mut bindings: ResMut<ModelMotionBindings>,
    mut placements: ResMut<RenderSourcePlacements>,
    mut models: ResMut<ModelShadowSources>,
    mut bounds: ResMut<ShadowModelBoundsSources>,
    mut queries: ResMut<PlacedModelQueries>,
    mut queue: ResMut<SceneRenderQueue>,
    mut entities: Query<(&mut Transform, &mut GlobalTransform)>,
    scene: Option<Res<super::SceneData>>,
    probe_buffer: Option<Res<ModelProbeLighting>>,
    mut buffers: Option<ResMut<Assets<super::ShaderBuffer>>>,
) {
    for pose in pending.0.drain(..) {
        let Some(placement) = placements.0.get_mut(pose.placement) else {
            error!(
                "Physics pose references missing placement {}",
                pose.placement
            );
            continue;
        };
        if !bindings.0.contains_key(&pose.placement) {
            let Some(source) = models.0.iter().position(|s| s.placement == pose.placement) else {
                continue;
            };
            let range = models.0[source].instances.clone();
            let surfaces = range
                .clone()
                .map(|index| {
                    let instance = &queue.instances[index];
                    SurfaceBinding {
                        index,
                        baked: instance.source.is_some_and(|source| source.world_baked),
                        initial_transform: instance.transform,
                        initial_bounds: instance.bounds,
                    }
                })
                .collect();
            let mut lighting = crate::probe_lighting::ProbeQuery::default();
            if let Some(probes) = scene.as_ref().and_then(|s| s.environment_probes.as_ref()) {
                let bounds = models.0[source].bounds;
                let _ = probes.interpolate_query_sh(
                    std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5),
                    &mut lighting,
                );
            }
            bindings.0.insert(
                pose.placement,
                Binding {
                    initial_position: placement.position,
                    initial_rotation: placement.rotation,
                    source,
                    shadow_bounds: bounds.0.iter().position(|b| b.placement == pose.placement),
                    query: queries.0.iter().position(|q| q.indices == range),
                    surfaces,
                    lighting,
                },
            );
        }
        let binding = bindings.0.get_mut(&pose.placement).unwrap();
        let rows = std::array::from_fn(|i| pose.delta[i / 4][i % 4]);
        let delta = super::game_affine_to_view(&rows).to_matrix();
        placement.position = std::array::from_fn(|r| {
            pose.delta[r][3]
                + (0..3)
                    .map(|c| pose.delta[r][c] * binding.initial_position[c])
                    .sum::<f32>()
        });
        placement.rotation = std::array::from_fn(|c| {
            std::array::from_fn(|r| {
                (0..3)
                    .map(|i| pose.delta[r][i] * binding.initial_rotation[c][i])
                    .sum::<f32>()
            })
        });
        let source = &mut models.0[binding.source];
        source.bounds = render_model::transformed_bounds(
            source.local_bounds,
            placement.position,
            placement.rotation,
            placement.scale,
        );
        source.flags_1c30 = render_model::model_owner_bounds_flags(
            source.flags_1c30,
            [
                source.bounds[0][0],
                source.bounds[0][1],
                source.bounds[0][2],
                0.,
            ],
            [
                source.bounds[1][0],
                source.bounds[1][1],
                source.bounds[1][2],
                0.,
            ],
        );
        render_model::set_model_dynamic_caster_flag(&mut source.flags_18bc, 1);
        let lighting = scene
            .as_ref()
            .and_then(|s| s.environment_probes.as_ref())
            .and_then(|probes| {
                probes.interpolate_query_sh(
                    std::array::from_fn(|i| (source.bounds[0][i] + source.bounds[1][i]) * 0.5),
                    &mut binding.lighting,
                )
            });
        if let (Some(lighting), Some(handle), Some(buffers)) =
            (lighting, probe_buffer.as_ref(), buffers.as_mut())
        {
            if let Some(mut buffer) = buffers.get_mut(&handle.0) {
                if let Some(data) = buffer.data.as_mut() {
                    for surface in &binding.surfaces {
                        let tag = queue.instances[surface.index].probe_index;
                        if tag & 0x80000000 != 0 {
                            continue;
                        }
                        let start = tag as usize * 9 * 16;
                        if let Some(record) = data.get_mut(start..start + 9 * 16) {
                            for (coefficient, channels) in
                                lighting.object_shader_record().iter().enumerate()
                            {
                                for (channel, value) in channels.iter().enumerate() {
                                    record[coefficient * 16 + channel * 4
                                        ..coefficient * 16 + channel * 4 + 4]
                                        .copy_from_slice(&value.to_le_bytes());
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(index) = binding.shadow_bounds {
            bounds.0[index].bounds = source.bounds;
        }
        if let Some(index) = binding.query {
            if let Some(input) = initial_placed_model_query(placement, source.local_bounds) {
                queries.0[index].input = input;
            }
        }
        for surface in &binding.surfaces {
            let instance = &mut queue.instances[surface.index];
            instance.transform = if surface.baked {
                Transform::from_matrix(delta * surface.initial_transform.to_matrix())
            } else {
                placed_transform(placement)
            };
            instance.bounds = moved_bounds(surface.initial_bounds, delta);
            if let Some(entity) = instance.entity {
                if let Ok((mut transform, mut global)) = entities.get_mut(entity) {
                    *transform = instance.transform;
                    *global = GlobalTransform::from(instance.transform);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physics_pose_updates_geometry_queries_and_both_shadow_bound_sources() {
        let mut app = App::new();
        let placement = crate::entities::PlacedModel {
            id: "rotor".into(),
            resource_path: "rotor.bmodel".into(),
            position: [3., -4., 2.],
            rotation: [[1., 0., 0.], [0.3, 1., 0.], [0., 0.2, 1.]],
            scale: [1., 2., -1.],
            custom_materials: Vec::new(),
            max_visible_range: Some(50.),
        };
        let local = [[-2., -1., -0.2], [2., 1., 0.2]];
        let input = initial_placed_model_query(&placement, local).unwrap();
        let source = super::super::shadow_updates::ModelShadowSource {
            placement: 0,
            local_bounds: local,
            bounds: input.bounds,
            skinned: false,
            surfaces: Vec::new(),
            instances: 0..1,
            pickup_id: None,
            flags_1c30: 0,
            flags_18bc: render_model::INITIAL_MODEL_CONTEXT_FLAGS_18BC,
            flags_18bd: 0,
            skinning: None,
        };
        let entity = app
            .world_mut()
            .spawn((Transform::IDENTITY, GlobalTransform::IDENTITY))
            .id();
        let corners: [Vec3; 8] = std::array::from_fn(|i| {
            super::super::placed_vertex(
                &placement,
                std::array::from_fn(|axis| local[(i >> axis) & 1][axis]),
            )
        });
        let world_bounds = render_bounds(&corners);
        let mut queue = SceneRenderQueue::default();
        queue.instances.push(super::super::RenderInstance {
            source: Some(super::super::RenderInstanceSource {
                surface: super::super::RenderModelSurface::Bmodel {
                    placement: 0,
                    surface: 0,
                },
                world_baked: true,
            }),
            mesh: Handle::default(),
            material: super::super::MaterialHandle::Probe(Handle::default()),
            diffuse_image_slot: None,
            probe_index: 0,
            transform: Transform::IDENTITY,
            bounds: world_bounds,
            pickup_id: None,
            entity: Some(entity),
            visible: true,
        });
        app.insert_resource(ModelMotionBindings::default())
            .insert_resource(ModelPoseUpdates::default())
            .insert_resource(RenderSourcePlacements(vec![placement.clone()]))
            .insert_resource(ModelShadowSources(vec![source]))
            .insert_resource(ShadowModelBoundsSources(vec![
                super::super::ShadowModelBoundsSource {
                    placement: 0,
                    instances: 0..1,
                    bounds: input.bounds,
                },
            ]))
            .insert_resource(PlacedModelQueries(vec![
                super::super::PlacedModelQueryGroup {
                    indices: 0..1,
                    input,
                },
            ]))
            .insert_resource(queue)
            .add_systems(Update, publish_model_poses);
        let rotation = bevy::math::Mat3::from_rotation_z(0.8).to_cols_array_2d();
        let rows = render_model::affine_rows([1., 2., -3.], rotation, [1.; 3]);
        let delta: [[f32; 4]; 3] = std::array::from_fn(|i| rows[i]);
        for _ in 0..2 {
            app.world_mut()
                .resource_mut::<ModelPoseUpdates>()
                .0
                .push(ModelPose {
                    placement: 0,
                    delta,
                });
            app.update();
        }
        let current = &app.world().resource::<RenderSourcePlacements>().0[0];
        let expected_position = std::array::from_fn::<_, 3, _>(|r| {
            delta[r][3]
                + (0..3)
                    .map(|c| delta[r][c] * placement.position[c])
                    .sum::<f32>()
        });
        assert_eq!(current.position, expected_position);
        // The shear must survive rotation: replacing the placement by the
        // body's orthogonal basis would change visible and shadow vertices.
        for c in 0..3 {
            let a = Vec3::from(placement.rotation[c]).length();
            let b = Vec3::from(current.rotation[c]).length();
            assert!((a - b).abs() < 0.00001);
        }
        let bounds = render_model::transformed_bounds(
            local,
            current.position,
            current.rotation,
            current.scale,
        );
        assert_eq!(
            app.world().resource::<ModelShadowSources>().0[0].bounds,
            bounds
        );
        assert_eq!(
            app.world().resource::<ShadowModelBoundsSources>().0[0].bounds,
            bounds
        );
        assert_eq!(
            app.world().resource::<PlacedModelQueries>().0[0]
                .input
                .bounds,
            bounds
        );
        assert!(render_model::model_is_dynamic_caster(
            app.world().resource::<ModelShadowSources>().0[0].flags_18bc
        ));
        let instance = &app.world().resource::<SceneRenderQueue>().instances[0];
        assert_eq!(
            *app.world().get::<Transform>(entity).unwrap(),
            instance.transform
        );
        assert_eq!(
            app.world()
                .get::<GlobalTransform>(entity)
                .unwrap()
                .to_matrix(),
            instance.transform.to_matrix()
        );
        assert!(app.world().resource::<ModelPoseUpdates>().0.is_empty());
    }

    #[test]
    fn baked_and_local_vertices_receive_the_same_rigid_motion() {
        let initial = rigid_matrix([3., -4., 2.], Mat3::from_rotation_z(0.3).to_cols_array_2d());
        let current = rigid_matrix([-2., 8., 1.], Mat3::from_rotation_z(1.4).to_cols_array_2d());
        let delta = current * initial.inverse();
        let scale = Mat4::from_scale(Vec3::new(2., 0.5, -3.));
        let point = Vec3::new(1., 2., -1.);
        let baked = (delta * initial * scale).transform_point3(point);
        let local = (current * scale).transform_point3(point);
        assert!(baked.distance(local) < 0.00001);
        let bounds = render_bounds(&[Vec3::ZERO, Vec3::ONE]);
        let moved = moved_bounds(bounds, delta);
        for i in 0..8 {
            let corner = Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32);
            let p = delta.transform_point3(corner);
            let d = (p - Vec3::from(moved.center)).abs() - Vec3::from(moved.half_extents);
            assert!(d.max_element() < 0.00001);
        }
    }
}
