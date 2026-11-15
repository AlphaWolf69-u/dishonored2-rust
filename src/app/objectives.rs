//! Game-local gameflow tree and its published Journal state. No map hint,
//! invented objective completion or second entity graph executor.
use crate::{
    archive::Catalog,
    entities::{Document, Value},
    gameplay::gameflow::{
        State,
        program::{Program, Runtime},
    },
};
use bevy::prelude::*;

#[derive(Resource)]
pub(super) struct Objectives {
    runtime: Runtime,
    connected: bool,
}
impl Objectives {
    pub(super) fn marker_entities(&self) -> impl Iterator<Item = &str> {
        self.runtime.marker_entities()
    }
    pub(super) fn load(catalog: &Catalog, document: &Document) -> Result<Option<Self>, String> {
        fn find(fields: &[(String, Value)]) -> Option<&str> {
            for (name, value) in fields {
                if name == "m_gameflowDecl" {
                    if let Value::Atom(value) = value {
                        return Some(crate::gameplay::options::decl::unquote(value));
                    }
                }
                if let Value::Scope(fields) = value {
                    if let Some(name) = find(fields) {
                        return Some(name);
                    }
                }
            }
            None
        }
        let Some(name) = document
            .definitions
            .iter()
            .find_map(|definition| find(&definition.fields))
        else {
            return Ok(None);
        };
        if name.is_empty() || name.eq_ignore_ascii_case("NULL") {
            return Ok(None);
        }
        Ok(Some(Self {
            runtime: Runtime::new(Program::load(catalog, name)?),
            connected: false,
        }))
    }
}
/// Attach at scene publication, before the manager can consume LoadingEnd.
/// A later PreUpdate alone can miss the first warmup PostUpdate.
pub(super) fn attach(
    world: &mut World,
    effects: &mut crate::rendering::bevy_backend::camera_scene::FxSimulation,
) {
    if let Some(mut owner) = world.get_resource_mut::<Objectives>() {
        effects.set_gameflow(owner.runtime.mailbox());
        owner.connected = true;
    }
}
pub(super) fn connect(
    owner: Option<ResMut<Objectives>>,
    effects: Option<ResMut<crate::rendering::bevy_backend::camera_scene::FxSimulation>>,
) {
    let (Some(mut owner), Some(mut effects)) = (owner, effects) else {
        return;
    };
    if !owner.connected {
        effects.set_gameflow(owner.runtime.mailbox());
        owner.connected = true;
    }
}
pub(super) fn update(
    owner: Option<ResMut<Objectives>>,
    progress: Option<ResMut<State>>,
    mut menu: NonSendMut<super::pause_menu::PauseMenu>,
) {
    let (Some(mut owner), Some(mut progress)) = (owner, progress) else {
        return;
    };
    if let Err(error) = owner.runtime.update() {
        error!("Mission objectives: {error}");
        return;
    }
    owner.runtime.publish_portraits(&mut progress);
    owner.runtime.mailbox().publish_markers(&mut progress);
    if owner.runtime.changed() {
        let notifications = owner.runtime.publish(&mut progress);
        menu.queue_objectives(notifications);
    }
}
