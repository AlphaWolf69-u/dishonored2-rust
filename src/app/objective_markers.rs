//! Objective marker admission 4e8970/66d760/66fa10. Positions borrow the
//! existing entity pose publication; no entity/model/resource cache is built.
use crate::gameplay::{
    entity_lifetime::{EntityId, LiveEntities},
    gameflow::State,
};
use crate::rendering::bevy_backend::world_effects::WorldEffectFrames;
use bevy::{camera::CameraProjection, prelude::*};
use std::collections::{HashMap, HashSet};

const STYLES: [&str; 8] = [
    "MARKER_OBJECTIVE",
    "MARKER_OBJECTIVE_OPTIONAL",
    "MARKER_OBJECTIVE_LETHAL",
    "MARKER_OBJECTIVE_LETHAL_OPTIONAL",
    "MARKER_OBJECTIVE_NON_LETHAL",
    "MARKER_OBJECTIVE_NON_LETHAL_OPTIONAL",
    "MARKER_OBJECTIVE_MIXED",
    "MARKER_OBJECTIVE_MIXED_OPTIONAL",
];
struct Source {
    placement: Option<usize>,
    bounds: Option<usize>,
}
#[derive(Resource)]
pub(super) struct Positions {
    pub(super) targets: HashSet<String>,
    tags: [String; 8],
    sources: HashMap<EntityId, Source>,
}
impl Positions {
    pub(super) fn prepare(
        catalog: &crate::archive::Catalog,
        document: Option<&crate::entities::Document>,
        objectives: Option<&super::objectives::Objectives>,
        models: &[crate::entities::PlacedModel],
        joints: &mut super::joint_frames::JointFrames,
        machinery: &super::machinery::AmbientMachinery,
        animations: &super::usable_animation::UsableAnimations,
    ) -> Result<Self, String> {
        use crate::gameplay::options::decl::{atom, scope};
        let fields = crate::assets::entities::player::resolved_decl(
            "ui_hudelementobjectives",
            "ui/hud_objectives.uihudelementobjectives",
            |path| {
                Ok(catalog
                    .read(path)
                    .map_err(|error| error.to_string())?
                    .map(crate::entities::owned_declaration_text))
            },
        )?;
        let setup = scope(&fields, "m_markerSetup").ok_or("objective marker setup missing")?;
        let mut tags = std::array::from_fn(|_| String::new());
        for (index, name) in STYLES.iter().enumerate() {
            let position = scope(setup, &format!("enumItem[{name}]"))
                .and_then(|fields| scope(fields, "m_markerPosition"))
                .and_then(|fields| scope(fields, "m_info"))
                .and_then(|fields| scope(fields, "item[0]"))
                .ok_or("objective marker position missing")?;
            if atom(position, "m_reference") != Some("MARKERPOSITION_TAG") {
                return Err("objective marker reference is not a tag".into());
            }
            tags[index] = atom(position, "m_tag").unwrap_or("").to_owned();
        }
        let mut targets: HashSet<String> = objectives
            .into_iter()
            .flat_map(|owner| owner.marker_entities())
            .map(str::to_ascii_lowercase)
            .collect();
        fn collect(fields: &[(String, crate::entities::Value)], targets: &mut HashSet<String>) {
            for (name, value) in fields {
                match value {
                    crate::entities::Value::Atom(value)
                        if matches!(name.as_str(), "m_target" | "m_newTarget") =>
                    {
                        targets.insert(
                            crate::gameplay::options::decl::unquote(value).to_ascii_lowercase(),
                        );
                    }
                    crate::entities::Value::Scope(fields) => collect(fields, targets),
                    _ => {}
                }
            }
        }
        if let Some(document) = document {
            for definition in &document.definitions {
                collect(&definition.fields, &mut targets);
            }
        }
        let mut requests = Vec::new();
        let unique: HashSet<&str> = tags
            .iter()
            .map(String::as_str)
            .filter(|tag| !tag.is_empty())
            .collect();
        for (index, model) in models.iter().enumerate() {
            if targets.contains(&model.id.to_ascii_lowercase()) {
                for tag in &unique {
                    requests.push((index, (*tag).to_owned()));
                }
            }
        }
        joints.prepare_optional_tags(catalog, models, &requests, machinery, animations)?;
        Ok(Self {
            targets,
            tags,
            sources: HashMap::new(),
        })
    }
    fn point(
        &mut self,
        entity: EntityId,
        name: &str,
        kind: usize,
        origin: crate::gameplay::reference_frame::ReferenceFrame,
        models: &[crate::entities::PlacedModel],
        bounds: &super::scene_queue::ShadowModelBoundsSources,
        joints: &super::joint_frames::JointFrames,
    ) -> Result<[f32; 3], String> {
        let source = self.sources.entry(entity).or_insert_with(|| {
            let placement = models
                .iter()
                .position(|model| model.id.eq_ignore_ascii_case(name));
            Source {
                placement,
                bounds: placement.and_then(|placement| {
                    bounds
                        .0
                        .iter()
                        .position(|source| source.placement == placement)
                }),
            }
        });
        if let Some(placement) = source.placement {
            if let Some(binding) = joints.resolve(placement, None, &self.tags[kind])? {
                let model = &models[placement];
                let parent = crate::gameplay::reference_frame::ReferenceFrame {
                    position: model.position,
                    basis: model.rotation,
                };
                if let Some(frame) = joints.bound_frame(
                    placement,
                    binding.joint,
                    3,
                    binding.offset,
                    binding.orientation,
                    parent,
                )? {
                    return Ok(frame.position);
                }
            }
        }
        // 676c90: absent tag uses origin plus model/entity bounding height.
        // Point targets have no render bounds and retain their actual origin.
        let mut point = origin.position;
        if let Some(index) = source.bounds {
            point[2] += bounds.0[index].bounds[1][2] - bounds.0[index].bounds[0][2];
        }
        Ok(point)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Key {
    pub task: u32,
    pub target: u32,
    pub entity: EntityId,
}
#[derive(Clone, Copy)]
pub(super) struct Marker {
    pub key: Key,
    pub kind: u8,
    pub marked: bool,
    pub ndc: [f32; 2],
    pub distance: f32,
    pub facing: f32,
}

pub(super) fn update(
    progress: Option<Res<State>>,
    positions: Option<ResMut<Positions>>,
    models: Option<Res<super::state::RenderSourcePlacements>>,
    bounds: Option<Res<super::scene_queue::ShadowModelBoundsSources>>,
    joints: Option<Res<super::joint_frames::JointFrames>>,
    frames: Option<Res<WorldEffectFrames>>,
    live: Option<Res<LiveEntities>>,
    cameras: Query<(&Transform, &Projection), With<Camera3d>>,
    mut menu: NonSendMut<super::pause_menu::PauseMenu>,
) {
    let (Some(progress), Some(frames), Some(live), Ok((camera, projection))) =
        (progress, frames, live, cameras.single())
    else {
        return;
    };
    let (Some(mut positions), Some(models), Some(bounds), Some(joints)) =
        (positions, models, bounds, joints)
    else {
        return;
    };
    let Some(mut buffer) = menu.marker_buffer() else {
        return;
    };
    buffer.clear();
    let enabled = crate::platform::console::shared()
        .read()
        .cvars
        .int("ark_ui_show_objective_markers")
        .unwrap_or(1)
        != 0;
    if enabled {
        let clip = projection.get_clip_from_view() * camera.to_matrix().inverse();
        for task in progress
            .mission()
            .into_iter()
            .flat_map(|mission| &mission.objectives)
            .flat_map(|objective| &objective.tasks)
        {
            if progress.status(&task.name) != Some(2) {
                continue;
            }
            for target in progress.marker_targets(task) {
                let Some(entity) = live.find(&target.entity) else {
                    continue;
                };
                let Some((name, _)) = live.get(entity) else {
                    continue;
                };
                let frame = match frames.published_frame(name) {
                    Ok(Some(frame)) => frame,
                    Ok(None) => continue,
                    Err(error) => {
                        warn!("Objective marker: {error}");
                        continue;
                    }
                };
                let kind = (task.lethality.clamp(0, 3) as u8) * 2 + u8::from(task.optional);
                let point = match positions.point(
                    entity,
                    name,
                    kind as usize,
                    frame,
                    &models.0,
                    &bounds,
                    &joints,
                ) {
                    Ok(point) => super::game_to_view(point),
                    Err(error) => {
                        warn!("Objective marker: {error}");
                        continue;
                    }
                };
                let projected = clip * point.extend(1.);
                if projected.w == 0. {
                    continue;
                }
                let direction = point - camera.translation;
                let facing = direction.normalize_or_zero().dot(*camera.forward());
                buffer.push(Marker {
                    key: Key {
                        task: crate::assets::lang::key_id(&task.name),
                        target: target.name,
                        entity,
                    },
                    kind,
                    marked: progress.task_marked(&task.name),
                    ndc: [projected.x / projected.w, projected.y / projected.w],
                    distance: direction.length(),
                    facing,
                });
            }
        }
    }
    positions
        .sources
        .retain(|entity, _| buffer.iter().any(|marker| marker.key.entity == *entity));
    menu.publish_markers(buffer);
}
