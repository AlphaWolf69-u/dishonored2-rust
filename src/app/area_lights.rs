//! Resident authored area-light images, shared by the scene light-list owner.
use super::SceneData;
use bevy::prelude::*;
use std::collections::HashMap;

#[derive(Resource)]
pub(super) struct AreaLightSources(pub Result<Vec<Option<Handle<Image>>>, String>);

pub(super) fn load(
    mut commands: Commands,
    scene: Res<SceneData>,
    mut images: ResMut<Assets<Image>>,
) {
    let result = (|| {
        let mut resident = HashMap::new();
        let mut aliases = HashMap::new();
        scene
            .placed_lights
            .iter()
            .map(|light| {
                light
                    .area_texture
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .filter(|_| light.kind == crate::entities::LightType::Area)
                    .map(|name| {
                        super::resident_images::prepare_image(
                            scene
                                .catalog
                                .as_ref()
                                .ok_or("area-light image catalog unavailable")?,
                            name,
                            &mut images,
                            &mut resident,
                            &mut aliases,
                        )
                    })
                    .transpose()
            })
            .collect()
    })();
    commands.insert_resource(AreaLightSources(result));
}
