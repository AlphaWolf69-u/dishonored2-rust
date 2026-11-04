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
    prepared: Option<Res<AreaLightSources>>,
    resident: Res<super::resident_images::ResidentImages>,
) {
    if prepared.is_some() { return; }
    commands.insert_resource(prepare(&scene, &mut images, &resident));
}

pub(super) fn prepare(scene: &SceneData, images: &mut dyn crate::rendering::bevy_backend::asset_preparation::AssetWrite<Image>, registry: &super::resident_images::ResidentImages) -> AreaLightSources {
    let result = (|| {
        let resident = registry.consumer();
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
                            images,
                            &resident,
                            &mut aliases,
                        )
                    })
                    .transpose()
            })
            .collect()
    })();
    AreaLightSources(result)
}
