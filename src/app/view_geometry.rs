//! View geometry.
use super::{
    archive, camera_projection, entities, game_settings, physworld, render_light, render_model,
    render_shadow, shadow_pass, tome, world_mesh, Aabb, FocusTarget, HashMap, Indices,
    ViewSettings, PlacedModelQueryGroup, PrimitiveTopology, Projection, RenderAssetUsages,
    DH2_BITANGENT, DH2_NORMAL_W, DEFAULT_HORIZONTAL_FOV_DEGREES,
};
use bevy::prelude::*;

pub(super) fn render_bounds(points: &[Vec3]) -> Aabb {
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for &point in points {
        low = low.min(point);
        high = high.max(point);
    }
    Aabb::from_min_max(low, high)
}

pub(super) fn authored_world_bounds(record: &world_mesh::WorldVisibility) -> Aabb {
    let [low, high] = record.bounds;
    Aabb::from_min_max(
        Vec3::new(low[0], low[2], -high[1]),
        Vec3::new(high[0], high[2], -low[1]),
    )
}

pub(super) struct Bounds {
    pub(super) low: Vec3,
    pub(super) high: Vec3,
    pub(super) points: usize,
}

impl Bounds {
    pub(super) fn empty() -> Self {
        Self {
            low: Vec3::splat(f32::INFINITY),
            high: Vec3::splat(f32::NEG_INFINITY),
            points: 0,
        }
    }
    pub(super) fn add(&mut self, point: Vec3) {
        self.low = self.low.min(point);
        self.high = self.high.max(point);
        self.points += 1;
    }
    pub(super) fn focus(&self, label: String) -> FocusTarget {
        FocusTarget {
            center: (self.low + self.high) * 0.5,
            distance: ((self.high - self.low).length() * 0.8).max(12.0),
            label,
            spawn: None,
        }
    }
}

pub(super) fn view_of(target: &FocusTarget) -> Transform {
    if let Some(start) = target.spawn {
        let eye = game_to_view(start.position) + Vec3::Y * 1.7;
        let forward =
            game_to_view([start.forward_xy[0], start.forward_xy[1], 0.0]).normalize_or_zero();
        return Transform::from_translation(eye).looking_at(eye + forward * 10.0, Vec3::Y);
    }
    let distance = target.distance;
    Transform::from_translation(
        target.center + Vec3::new(distance * 0.45, distance * 0.4, distance),
    )
    .looking_at(target.center, Vec3::Y)
}

// Diagnostic replay preserves the saved view; it does not change game culling.
pub(super) fn f8_camera_pose(report: &str) -> Option<Transform> {
    pub(super) fn vector(report: &str, label: &str) -> Option<[f32; 3]> {
        let text = report
            .lines()
            .find_map(|line| line.strip_prefix(label))?
            .trim();
        let values = text
            .strip_prefix('[')?
            .strip_suffix(']')?
            .split(',')
            .map(|part| part.trim().parse::<f32>())
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let values: [f32; 3] = values.try_into().ok()?;
        values
            .iter()
            .all(|value| value.is_finite())
            .then_some(values)
    }
    let eye = game_to_view(vector(report, "Camera game_xyz:")?);
    let direction = game_to_view(vector(report, "Camera direction game_xyz:")?);
    let up = if report
        .lines()
        .any(|line| line.starts_with("Camera up game_xyz:"))
    {
        game_to_view(vector(report, "Camera up game_xyz:")?)
    } else {
        Vec3::Y
    };
    (direction.length_squared() > 0.0
        && up.length_squared() > 0.0
        && direction.cross(up).length_squared() > 0.0)
        .then(|| Transform::from_translation(eye).looking_to(direction, up))
}

// Replay metadata is diagnostic input, not a replacement for the registered
// 80-degree default. Original live capture can contain a user-adjusted FOV.
pub(super) fn f8_reference_fov(report: &str) -> Option<f32> {
    let value = report
        .lines()
        .find_map(|line| line.strip_prefix("Reference horizontal FOV degrees:"))
        .map_or(Some(DEFAULT_HORIZONTAL_FOV_DEGREES), |text| {
            text.trim().parse::<f32>().ok()
        })?;
    (value > 0.0).then_some(())?;
    tome::render_vertical_fov_radians(value)
        .ok()
        .map(|_| value)
}

pub(super) fn reference_fov_from_projection(projection: &Projection) -> Result<f32, String> {
    if let Projection::Custom(custom) = projection {
        if let Some(game) = custom.get::<camera_projection::GameProjection>() {
            return Ok(game.reference_horizontal_degrees);
        }
    }
    let Projection::Perspective(perspective) = projection else {
        return Err("ordinary original world visibility requires a perspective projection".into());
    };
    let vertical = perspective.fov;
    if !vertical.is_finite() || vertical <= 0.0 || vertical >= std::f32::consts::PI {
        return Err("camera has an invalid vertical field of view".into());
    }
    // Inverse of the original FUN_140890b50 reference-aspect conversion.
    Ok((2.0 * ((vertical * 0.5).tan() / 0.5625).atan()).to_degrees())
}

pub(super) fn view_distance_scale(
    projection: &Projection,
    viewport: UVec2,
    settings: Option<&ViewSettings>,
) -> Result<f32, String> {
    let settings = match settings {
        Some(settings) => settings.0.clone()?,
        None => game_settings::view_settings("")?,
    };
    let horizontal = tome::render_fov_radians(
        reference_fov_from_projection(projection)?,
        viewport.x,
        viewport.y,
        1.0,
    )?
    .0;
    Ok(render_shadow::caster_distance_scale(
        horizontal,
        settings.horizontal_fov_degrees,
        settings.lod_zoom_multiplier,
    ))
}

pub(super) fn light_preset(
    light: &entities::PlacedLight,
    linear_color: [f32; 3],
) -> Option<render_light::PresetOutput> {
    let (cutoff, falloff) = light_distance(light.distance_cutoff, light.distance_falloff)?;
    let preset = render_light::preset_index(light.preset.as_deref())?;
    let reflector = if preset == 11 {
        render_light::reflector_index(light.reflector_preset.as_deref())?
    } else {
        0 // The native dispatcher does not read this input for other presets.
    };
    let kind = match light.kind {
        entities::LightType::Point => 0,
        entities::LightType::HalfPoint => 1,
        entities::LightType::Spot => 2,
        entities::LightType::Area => 3,
        entities::LightType::Parallel => 4,
    };
    render_light::convert(render_light::PresetInput {
        kind,
        preset,
        reflector,
        area_energy_conservative: light.area_energy_conservative,
        cutoff,
        falloff,
        cutoff_cosine: authored_light_cosines(light.angular_cutoff, light.angular_falloff).0,
        back_reflection: light.back_reflection?,
        area_size: light.area_size,
        color: linear_color,
    })
}

pub(super) fn authored_area_light_transform(
    position: Vec3,
    rotation: [[f32; 3]; 3],
) -> Option<Transform> {
    // arealight::CreateAreaLightContext: Z=SpotDir, X=AreaLightX,
    // Y=cross(Z,X). Bevy's right is -X for this emitting normal;
    // the rectangle's corner set and emitting side are unchanged by that sign.
    let direction = game_to_view(rotation[0]).normalize_or_zero();
    let up = direction
        .cross(game_to_view(rotation[1]))
        .normalize_or_zero();
    if direction == Vec3::ZERO || up == Vec3::ZERO {
        return None;
    }
    Some(Transform::from_translation(position).looking_to(direction, up))
}

pub(super) fn light_backend_intensity(
    normalized_intensity: f32,
    kind: entities::LightType,
    rect_size: [f32; 2],
) -> Option<f32> {
    let intensity = if kind == entities::LightType::Area {
        if !rect_size.iter().all(|size| size.is_finite() && *size > 0.0) {
            return None;
        }
        let area = rect_size[0] * rect_size[1];
        if !(area * std::f32::consts::PI).is_finite() || area <= 0.0 {
            return None;
        }
        // Cancel Bevy's extra emitter-area divisor; the native preset already
        // handles conservative area energy. Absolute photometry is unverified.
        (normalized_intensity as f64 * area as f64 * 2_000.0) as f32
    } else if kind == entities::LightType::Parallel {
        normalized_intensity * render_light::ARKANE_TO_WORLD_LUMINANCE
    } else {
        // Original HDR -> world units, then cancel Bevy's lumen-to-candela
        // division shared by point, half-point and spot extraction.
        normalized_intensity
            * render_light::ARKANE_TO_WORLD_LUMINANCE
            * (4.0 * std::f32::consts::PI)
    };
    (intensity.is_finite() && intensity >= 0.0).then_some(intensity)
}

pub(super) fn authored_light_cosines(cutoff_degrees: f32, falloff_degrees: f32) -> (f32, f32) {
    // FUN_140881640 passes m_AngularFalloff as FUN_140359b30 param_2 and
    // m_AngularCutoff as param_3. The helper clamps cutoff, then raises falloff.
    const MAX_COSINE: f32 = 0.999_847_7;
    const GAP: f64 = 0.000_152_304_841_3;
    let cutoff = (cutoff_degrees * std::f32::consts::PI / 360.0)
        .cos()
        .clamp(-MAX_COSINE, MAX_COSINE);
    let mut falloff = (falloff_degrees * std::f32::consts::PI / 360.0).cos();
    if (falloff as f64) <= cutoff as f64 + GAP {
        falloff = (cutoff as f64 + GAP) as f32;
    }
    (cutoff, falloff)
}

pub(super) fn light_distance(cutoff: f32, falloff: f32) -> Option<(f32, f32)> {
    if !cutoff.is_finite() || !falloff.is_finite() {
        return None;
    }
    Some(render_light::distance_fields(cutoff, falloff))
}

pub(super) fn preserved_half_point_angular_scale(cutoff_degrees: f32, falloff_degrees: f32) -> f32 {
    // Keep the pre-existing viewer ramp until the original HalfPoint shader is
    // verified; this is not the source-backed Spot projection threshold.
    const MAX_SINE: f32 = 0.999_847_7;
    const GAP: f32 = 0.000_152_304_84;
    let falloff_sine = (falloff_degrees * std::f32::consts::PI / 360.0)
        .sin()
        .clamp(-MAX_SINE, MAX_SINE);
    let cutoff_sine = (cutoff_degrees * std::f32::consts::PI / 360.0).sin();
    let boundary_angle = cutoff_sine.max(falloff_sine + GAP).asin();
    -1.0 / boundary_angle.sin().max(0.000_001)
}

// FUN_140359990 builds the light projection; FUN_140232bc0 stores its inverse
// at +0x130. FUN_14031bcc0 multiplies it by camera clip-from-world and tests the
// transformed [0,1]^3 light volume through FUN_140109dd0 / FUN_14010a0d0.
pub(super) fn local_light_clip_to_world(
    position: Vec3,
    rotation: [[f32; 3]; 3],
    kind: entities::LightType,
    range: f32,
    cutoff_cosine: Option<f32>,
) -> Option<Mat4> {
    if !position.is_finite() || !range.is_finite() || range <= 0.0 {
        return None;
    }

    let kind = if kind == entities::LightType::Spot {
        2
    } else {
        0
    };
    let cosine = if kind == 2 {
        cutoff_cosine?
    } else {
        0.0
    };
    let (_, inverse) = render_shadow::local_light_volume_projection(
        view_to_game(position),
        rotation,
        kind,
        range,
        cosine,
    )
    .ok()?;
    let game = Mat4::from_cols_array_2d(&std::array::from_fn(|c| {
        std::array::from_fn(|r| inverse[r][c])
    }));
    let game_to_host = Mat4::from_cols(Vec4::X, Vec4::new(0.0, 0.0, -1.0, 0.0), Vec4::Y, Vec4::W);
    let clip_to_world = game_to_host * game;
    clip_to_world.is_finite().then_some(clip_to_world)
}

pub(super) fn light_volume_intersects_camera(
    clip_from_world: Mat4,
    clip_to_world: Mat4,
) -> Option<bool> {
    let clip_from_light = clip_from_world * clip_to_world;
    let mut corners = [Vec4::ZERO; 8];
    for (index, corner) in corners.iter_mut().enumerate() {
        let point = Vec3::new(
            (index & 1) as f32,
            ((index >> 1) & 1) as f32,
            ((index >> 2) & 1) as f32,
        );
        *corner = clip_from_light * point.extend(1.0);
        if !corner.is_finite() {
            return None;
        }
    }

    // FUN_14010a0d0 rejects when all volume corners are outside one clip plane.
    let all_outside = [
        corners.iter().all(|p| p.x < -p.w),
        corners.iter().all(|p| p.x > p.w),
        corners.iter().all(|p| p.y < -p.w),
        corners.iter().all(|p| p.y > p.w),
        corners.iter().all(|p| p.z < 0.0),
        corners.iter().all(|p| p.z > p.w),
    ];
    Some(!all_outside.into_iter().any(|outside| outside))
}

pub(super) fn game_to_view([x, y, z]: [f32; 3]) -> Vec3 {
    Vec3::new(x, z, -y)
}

pub(super) fn environment_sun_ray(environment: entities::EnvironmentLighting) -> Option<Vec3> {
    // FUN_140017d30 uses environment theta as a polar angle and environment
    // phi directly. FUN_14032b280/140330c10 negate its toward-sun vector.
    // Do not substitute an authored parallel entity's yaw or an elevation.
    let toward_sun = render_shadow::environment_sun_direction(
        environment.sun_theta_degrees?,
        environment.sun_phi_degrees?,
    );
    Some(-game_to_view(toward_sun))
}

pub(super) fn view_to_game(point: Vec3) -> [f32; 3] {
    [point.x, -point.z, point.y]
}

pub(super) fn transformed(matrix: &[f32; 12], [x, y, z]: [f32; 3]) -> Vec3 {
    game_to_view([
        matrix[0] * x + matrix[1] * y + matrix[2] * z + matrix[3],
        matrix[4] * x + matrix[5] * y + matrix[6] * z + matrix[7],
        matrix[8] * x + matrix[9] * y + matrix[10] * z + matrix[11],
    ])
}

pub(super) fn transformed_direction(matrix: &[f32; 12], v: Vec3) -> Vec3 {
    let [x, y, z] = v.to_array();
    game_to_view([
        matrix[0] * x + matrix[1] * y + matrix[2] * z,
        matrix[4] * x + matrix[5] * y + matrix[6] * z,
        matrix[8] * x + matrix[9] * y + matrix[10] * z,
    ])
}

pub(super) fn placed_direction(placement: &entities::PlacedModel, v: Vec3) -> Vec3 {
    let scaled = std::array::from_fn::<_, 3, _>(|i| v[i] * placement.scale[i]);
    game_to_view(std::array::from_fn(|row| {
        (0..3).map(|i| placement.rotation[i][row] * scaled[i]).sum()
    }))
}

pub(super) fn placed_vertex(placement: &entities::PlacedModel, vertex: [f32; 3]) -> Vec3 {
    let scaled = [
        vertex[0] * placement.scale[0],
        vertex[1] * placement.scale[1],
        vertex[2] * placement.scale[2],
    ];
    game_to_view(std::array::from_fn(|row| {
        placement.position[row]
            + (0..3)
                // idMat3 stores its three basis axes as mat[0..2]. A local
                // vertex component weights the corresponding authored axis.
                .map(|column| placement.rotation[column][row] * scaled[column])
                .sum::<f32>()
    }))
}

pub(super) fn game_affine_to_view(matrix: &[f32; 12]) -> Transform {
    let game = Mat4::from_cols(
        Vec4::new(matrix[0], matrix[4], matrix[8], 0.0),
        Vec4::new(matrix[1], matrix[5], matrix[9], 0.0),
        Vec4::new(matrix[2], matrix[6], matrix[10], 0.0),
        Vec4::new(matrix[3], matrix[7], matrix[11], 1.0),
    );
    let axes = Mat4::from_cols(
        Vec4::X,
        Vec4::new(0.0, 0.0, -1.0, 0.0),
        Vec4::new(0.0, 1.0, 0.0, 0.0),
        Vec4::W,
    );
    Transform::from_matrix(axes * game * axes.inverse())
}

pub(super) fn placed_transform(placement: &entities::PlacedModel) -> Transform {
    let rows = render_model::affine_rows(placement.position, placement.rotation, placement.scale);
    let matrix = std::array::from_fn(|index| rows[index / 4][index % 4]);
    game_affine_to_view(&matrix)
}

pub(super) fn initial_placed_model_query(
    placement: &entities::PlacedModel,
    local_bounds: [[f32; 3]; 2],
) -> Option<render_model::ModelQueryInput> {
    let range = placement
        .max_visible_range
        .unwrap_or(render_model::INITIAL_MODEL_MAX_VISIBLE_RANGE);
    let bounds = render_model::transformed_bounds(
        local_bounds,
        placement.position,
        placement.rotation,
        placement.scale,
    );
    Some(render_model::ModelQueryInput {
        // FUN_140233a30 starts frame byte at1; initial bounds preparation sets4.
        // Surface bit40 is unbound: positive regular-path results are also
        // accepted by its bypass. Negative results remain unsupported there.
        frame_flags: 1 | 4,
        shape_flags: 0, // Inert with prepared frame bit4, regardless of its value.
        region: render_model::range_region(bounds, range),
        bounds,
    })
}

pub(super) fn apply_placed_model_queries(
    visible: &mut [bool],
    groups: &[PlacedModelQueryGroup],
    state: &tome::RenderQueryState,
    camera: [f32; 3],
    viewport: [u32; 2],
) -> Result<usize, String> {
    if groups
        .iter()
        .any(|group| group.indices.start > group.indices.end || group.indices.end > visible.len())
    {
        return Err("placed model query indices exceed render instances".into());
    }
    let query = tome::RenderAabbQuery::from_state(state)?;
    let mut accepted = 0;
    for group in groups {
        if render_model::model_visible(&query, camera, viewport, group.input)? {
            visible[group.indices.clone()].fill(true);
            accepted += 1;
        }
    }
    Ok(accepted)
}

pub(super) fn expanded_model_placements(
    placements: &[entities::PlacedModel],
    catalog: &archive::Catalog,
) -> Vec<entities::PlacedModel> {
    let mut expanded = placements.to_vec();
    let mut index = 0;
    while index < expanded.len() {
        let placement = &expanded[index];
        if placement.resource_path.ends_with(".blueprint.decl") {
            if placement.id.matches("#blueprint[").count() >= 16 {
                eprintln!("Blueprint nesting exceeds 16: {}", placement.id);
            } else {
                match catalog.read(&placement.resource_path) {
                    Ok(Some(bytes)) => match entities::expand_blueprint(
                        placement,
                        &String::from_utf8_lossy(&bytes),
                    ) {
                        Ok(children) => expanded.extend(children),
                        Err(error) => eprintln!(
                            "Blueprint parse failed {}: {error}",
                            placement.resource_path
                        ),
                    },
                    Ok(None) => eprintln!("Blueprint not indexed: {}", placement.resource_path),
                    Err(error) => {
                        eprintln!("Blueprint read failed {}: {error}", placement.resource_path)
                    }
                }
            }
        }
        index += 1;
    }
    expanded
}

pub(super) fn model_havok_path(
    placement: &entities::PlacedModel,
    kind: &str,
    hd: bool,
) -> Option<String> {
    if physworld::physics_shape_kind(kind) == 4 && !kind.eq_ignore_ascii_case("cvd") {
        return None;
    }
    let source = if let Some(model) = placement.resource_path.strip_prefix("generated/model/") {
        model.strip_suffix(".bmodel")?;
        physworld::replace_shape_extension(model, "lwo")
    } else {
        // The root renderer retains .blueprint; its children have no bodies.
        placement
            .resource_path
            .strip_prefix("generated/decls/blueprint/")?
            .strip_suffix(".decl")?
            .to_owned()
    };
    if placement
        .scale
        .iter()
        .any(|scale| !(scale * 100.0).is_finite())
    {
        return None;
    }
    let key = physworld::physics_shape_key(
        &source,
        physworld::physics_shape_kind(kind),
        placement.scale,
        0.01,
        hd,
    )?;
    Some(physworld::physics_shape_asset_path(&key))
}

pub(super) fn placed_collision_vertex(placement: &entities::PlacedModel, vertex: [f32; 3]) -> Vec3 {
    // The HKX filename's three percentages identify geometry already baked at
    // the placed model scale. Apply only the instance orientation and origin.
    game_to_view(std::array::from_fn(|row| {
        placement.position[row]
            + (0..3)
                .map(|column| placement.rotation[column][row] * vertex[column])
                .sum::<f32>()
    }))
}

pub(super) fn load_model_havok(
    catalog: &archive::Catalog,
    path: &str,
    depth: usize,
) -> Result<Vec<[[f32; 3]; 3]>, String> {
    if depth > 4 {
        return Err("HKX alias chain exceeds four links".into());
    }
    let bytes = catalog
        .read(path)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("HKX target not indexed: {path}"))?;
    if let Some((target, factor)) = physworld::standalone_hkx_alias(&bytes) {
        let mut triangles = load_model_havok(catalog, &target, depth + 1)?;
        for triangle in &mut triangles {
            for vertex in triangle {
                for axis in 0..3 {
                    vertex[axis] *= factor;
                }
            }
        }
        return Ok(triangles);
    }
    let physics = physworld::parse_hkx(&bytes)?;
    let geometry = physics.standalone_shapes(&bytes)?;
    if !geometry.compressed_decode_failures.is_empty() || !geometry.unknown_shapes.is_empty() {
        return Err(format!(
            "incomplete HKX geometry: {:?}, {:?}",
            geometry.compressed_decode_failures, geometry.unknown_shapes
        ));
    }
    Ok(geometry
        .triangles
        .into_iter()
        .map(|triangle| triangle.vertices)
        .collect())
}

fn collision_material_slot(
    catalog: &archive::Catalog,
    name: &str,
    slots: &mut HashMap<String, Option<u32>>,
    materials: &mut Vec<entities::physics_material::PhysicsMaterial>,
) -> Option<u32> {
    *slots.entry(name.to_owned()).or_insert_with(|| {
        match entities::physics_material::physics_material(name, |path| {
            catalog.read(path)
                .map(|bytes| bytes.map(|bytes| entities::declaration_text(&bytes).into_owned()))
                .map_err(|error| error.to_string())
        }) {
            Ok(material) => {
                let slot = materials.len() as u32;
                materials.push(material);
                Some(slot)
            }
            Err(error) => {
                eprintln!("Model collision material {name}: {error}");
                None
            }
        }
    })
}

/// Flattened static body children, with per-child material/filter retained.
#[derive(Default)]
pub(super) struct ModelCollision {
    pub triangles: Vec<([Vec3; 3], u32, u32, u32)>,
    pub owners: Vec<String>,
    pub materials: Vec<entities::physics_material::PhysicsMaterial>,
    pub matched: usize,
    pub failed: usize,
    pub extra_hd: usize,
    pub extra_hd_failed: usize,
}

pub(super) fn acquire_static_model_collision(
    catalog: &archive::Catalog,
    placements: &[entities::PlacedModel],
    bodies: &[entities::RigidBodySettings],
) -> ModelCollision {
    let roots: HashMap<_, _> = placements
        .iter()
        .map(|placement| (placement.id.as_str(), placement))
        .collect();
    let mut cached = HashMap::<String, Option<Vec<[[f32; 3]; 3]>>>::new();
    let mut triangles = Vec::new();
    let mut owners = Vec::new();
    let mut materials = Vec::new();
    let mut material_slots = HashMap::<String, Option<u32>>::new();
    let mut extra_hd = 0usize;
    let mut extra_hd_failed = 0usize;
    let mut matched = 0usize;
    let mut failed = 0usize;
    for body in bodies {
        let Some(&placement) = roots.get(body.entity_id.as_str()) else {
            continue;
        };
        // Dynamic/keyframed bodies need simulation, not frozen walk colliders.
        if body
            .physics_behavior
            .as_deref()
            .and_then(entities::rigid_body_behavior_kind)
            != Some(0)
        {
            continue;
        }
        let kind = match body
            .shape_type
            .as_deref()
            .and_then(entities::rigid_body_shape_kind)
        {
            Some(index) => ["hkt", "cvx", "msh", "box", "cvd"][index as usize],
            other => {
                failed += 1;
                eprintln!(
                    "Collision {}: unsupported declared shape {:?} ({other:?})",
                    placement.id, body.shape_type
                );
                continue;
            }
        };
        let Some(filter) = body.collision_filter() else {
            failed += 1;
            eprintln!(
                "Collision {}: unknown layer {:?} or malformed m_collisionSettings",
                placement.id, body.collision_layer
            );
            continue;
        };
        let Some(path) = model_havok_path(placement, kind, false) else {
            failed += 1;
            eprintln!("Collision {}: unsupported resource recipe", placement.id);
            continue;
        };
        let local = cached.entry(path.clone()).or_insert_with(|| {
            let parsed = load_model_havok(catalog, &path, 0);
            match parsed {
                Ok(triangles) => Some(triangles),
                Err(error) => {
                    eprintln!("Model collision {path}: {error}");
                    None
                }
            }
        });
        let Some(local) = local else {
            failed += 1;
            continue;
        };
        matched += 1;
        let owner = owners.len() as u32;
        owners.push(placement.id.clone());
        // FUN_140aa0360 is applied to the entity basis before native body creation.
        let body_basis =
            physworld::rigid_body_basis(placement.rotation.map(|[x, y, z]| [x, y, z, 0.0]));
        let body_pose = entities::PlacedModel {
            rotation: body_basis.map(|[x, y, z, _]| [x, y, z]),
            ..placement.clone()
        };
        let hd_material = body.extra_hd_material.as_deref()
            .filter(|name| !name.is_empty() && *name != "NULL");
        // Combining the children creates a material table even when the
        // primary resource had none (140808ea0). 140807c80 fills its primary
        // slots from m_MaterialOverride; its block flags must be applied as
        // well, or the coarse primary would still block weapon/vision rays.
        let primary_slot = if hd_material.is_some() {
            body.material.as_deref().and_then(|name| {
                collision_material_slot(catalog, name, &mut material_slots, &mut materials)
            })
        } else { None };
        let primary_filter = primary_slot.map_or(filter, |slot| {
            crate::gameplay::shape_tags::material_filter(filter, materials[slot as usize].collision.flags)
        });
        triangles.extend(local.iter().map(|triangle| (
            triangle.map(|vertex| placed_collision_vertex(&body_pose, vertex)),
            owner, primary_slot.unwrap_or(super::navigation::NO_MATERIAL), primary_filter,
        )));

        let Some(material_name) = hd_material else { continue };
        // 140808ea0 always acquires the HD child as HKT (kind 0), even if
        // the primary shape uses convex/mesh/box geometry. Both children
        // retain the same body pose and owner; HD is not a second body.
        let Some(hd_path) = model_havok_path(placement, "hkt", true) else {
            extra_hd_failed += 1;
            continue;
        };
        let hd = cached.entry(hd_path.clone()).or_insert_with(|| {
            match load_model_havok(catalog, &hd_path, 0) {
                Ok(triangles) => Some(triangles),
                Err(error) => {
                    eprintln!("Model collision {hd_path}: {error}");
                    None
                }
            }
        });
        let Some(hd) = hd else {
            // The game's optional HD acquisition leaves the primary intact.
            extra_hd_failed += 1;
            continue;
        };
        let slot = collision_material_slot(catalog, material_name, &mut material_slots, &mut materials);
        let Some(slot) = slot else {
            extra_hd_failed += 1;
            continue;
        };
        // 140808ea0 fills every HD material slot with this declaration.
        // Flattening that child into triangles therefore has one material
        // and the same 14081a5f0 filter for every leaf, including aliases.
        let hd_filter = crate::gameplay::shape_tags::material_filter(
            filter, materials[slot as usize].collision.flags,
        );
        triangles.extend(hd.iter().map(|triangle| (
            triangle.map(|vertex| placed_collision_vertex(&body_pose, vertex)),
            owner, slot, hd_filter,
        )));
        extra_hd += 1;
    }
    ModelCollision { triangles, owners, materials, matched, failed, extra_hd, extra_hd_failed }
}

#[cfg(test)]
pub(super) fn static_model_collision(
    catalog: &archive::Catalog,
    placements: &[entities::PlacedModel],
    bodies: &[entities::RigidBodySettings],
) -> (Vec<([Vec3; 3], u32, u32)>, Vec<String>, usize, usize) {
    let result = acquire_static_model_collision(catalog, placements, bodies);
    (result.triangles.into_iter().map(|(vertices, owner, _, filter)| (vertices, owner, filter)).collect(),
     result.owners, result.matched, result.failed)
}

pub(super) fn render_indices(indices: &[u16]) -> Indices {
    // FUN_1402c53e0 uses clockwise fronts (D3D FrontCounterClockwise=0).
    // Imported DH2 triangles are CW; Bevy's raster fronts are CCW. Convert
    // render indices once, preserving native normals and collision indices.
    assert_eq!(indices.len() % 3, 0, "incomplete native triangle");
    Indices::U16(
        indices
            .chunks_exact(3)
            .flat_map(|t| [t[0], t[2], t[1]])
            .collect(),
    )
}

pub(super) fn mirrored_render_mesh(mut mesh: Mesh, indices: &[u16], flipped: bool) -> Mesh {
    // A reflected affine instance reverses winding again. Preserve the native
    // CW indices for that variant so its final raster fronts remain CCW.
    if flipped {
        mesh.insert_indices(Indices::U16(indices.to_vec()));
    }
    mesh
}

pub(super) fn mesh_from_points(points: &[Vec3], uvs: &[[f32; 2]], indices: &[u16]) -> Mesh {
    let mut normals = vec![Vec3::ZERO; points.len()];
    for face in indices.chunks_exact(3) {
        let [a, b, c] = [face[0] as usize, face[1] as usize, face[2] as usize];
        let normal = (points[c] - points[a]).cross(points[b] - points[a]);
        normals[a] += normal;
        normals[b] += normal;
        normals[c] += normal;
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        points
            .iter()
            .map(|point| point.to_array())
            .collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        normals
            .into_iter()
            .map(|normal| normal.normalize_or_zero().to_array())
            .collect::<Vec<_>>(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec())
    .with_inserted_indices(render_indices(indices))
}

pub(super) fn mean_game_position(points: &[Vec3]) -> [f32; 3] {
    let mean = points.iter().copied().sum::<Vec3>() / points.len().max(1) as f32;
    view_to_game(mean)
}

pub(super) fn authored_mesh<const N: usize>(
    points: &[Vec3],
    uvs: &[[f32; 2]],
    indices: &[u16],
    packed: &[[u8; N]],
    transform_direction: impl Fn(Vec3) -> Vec3,
) -> Mesh {
    assert_eq!(points.len(), packed.len(), "authored vertex channel count");
    assert!(N == 12 || N == 16);
    let mut normals = Vec::with_capacity(points.len());
    let mut tangents = Vec::with_capacity(points.len());
    let mut bitangents = Vec::with_capacity(points.len());
    let mut normal_w = Vec::with_capacity(points.len());
    let mut uv1 = Vec::with_capacity(points.len());
    let mut colors = Vec::with_capacity(points.len());
    for p in packed {
        let snorm = |b: u8| ((b as i8 as f32) / 127.0).max(-1.0);
        let n = Vec3::new(snorm(p[4]), snorm(p[5]), snorm(p[6]));
        let t = Vec3::new(snorm(p[8]), snorm(p[9]), snorm(p[10]));
        let w = snorm(p[11]);
        normal_w.push(snorm(p[7]));
        normals.push(transform_direction(n).to_array());
        tangents.push(transform_direction(t).extend(w).to_array());
        bitangents.push(transform_direction(n.cross(t) * w).to_array());
        uv1.push([
            half::f16::from_bits(u16::from_le_bytes([p[0], p[1]])).to_f32(),
            half::f16::from_bits(u16::from_le_bytes([p[2], p[3]])).to_f32(),
        ]);
        if N == 16 {
            colors.push(std::array::from_fn::<_, 4, _>(|i| p[12 + i] as f32 / 255.0));
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        points.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec())
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, uv1)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, tangents)
    .with_inserted_attribute(DH2_BITANGENT, bitangents)
    .with_inserted_attribute(DH2_NORMAL_W, normal_w)
    .with_inserted_indices(render_indices(indices));
    if N == 16 {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    let flipped = transform_direction(Vec3::X)
        .cross(transform_direction(Vec3::Y))
        .dot(transform_direction(Vec3::Z))
        < 0.0;
    mirrored_render_mesh(mesh, indices, flipped)
}

pub(super) fn source_mesh<const N: usize>(
    positions: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u16],
    packed: &[[u8; N]],
    flipped: bool,
) -> Mesh {
    let points: Vec<Vec3> = positions.iter().copied().map(game_to_view).collect();
    mirrored_render_mesh(
        authored_mesh(&points, uvs, indices, packed, |n| {
            game_to_view(n.to_array())
        }),
        indices,
        flipped,
    )
}

/// VS14200 consumes original game-local POSITION, SNORM NORMAL.xy and UNORM
/// COLOR.xyz before wind and placement. Forward positions/normals may already
/// include axis conversion, placement and shear, and cannot supply those inputs.
/// These additional attributes are uploaded only for actual vegetation programs;
/// the forward mesh and its winding/UVs/basis remain unchanged.
pub(super) fn retain_world_shadow_channels<const N: usize>(
    mesh: &mut Mesh,
    positions: &[[f32; 3]],
    packed: &[[u8; N]],
) -> Result<usize, String> {
    if N != 12 && N != 16 {
        return Err(format!("unsupported world channel stride {N}"));
    }
    if positions.len() != packed.len() || mesh.count_vertices() != positions.len() {
        return Err("world source/forward vertex counts disagree".into());
    }
    let normals: Vec<[f32; 2]> = packed
        .iter()
        .map(|p| [p[4], p[5]].map(|b| ((b as i8 as f32) / 127.0).max(-1.0)))
        .collect();
    mesh.insert_attribute(shadow_pass::VEGETATION_POSITION, positions.to_vec());
    mesh.insert_attribute(shadow_pass::VEGETATION_NORMAL, normals);
    if N == 16 {
        let colors: Vec<[f32; 3]> = packed
            .iter()
            .map(|p| [p[12], p[13], p[14]].map(|b| b as f32 / 255.0))
            .collect();
        mesh.insert_attribute(shadow_pass::VEGETATION_COLOR, colors);
    } else {
        mesh.remove_attribute(shadow_pass::VEGETATION_COLOR);
    }
    Ok(positions.len())
}

pub(super) fn retain_vegetation_channels<const N: usize>(
    mesh: &mut Mesh,
    positions: &[[f32; 3]],
    packed: &[[u8; N]],
) -> Result<(), String> {
    if N != 16 {
        return Err(format!(
            "vegetation requires authored COLOR; {N}-byte channels have no COLOR"
        ));
    }
    retain_world_shadow_channels(mesh, positions, packed).map(|_| ())
}

pub(super) fn needs_exact_world_mesh(
    instance: &Transform,
    positions: &[[f32; 3]],
    exact: &[Vec3],
) -> bool {
    let approximate = instance.to_matrix();
    positions.iter().zip(exact).any(|(&local, &world)| {
        approximate
            .transform_point3(game_to_view(local))
            .distance(world)
            > 0.002
    })
}
