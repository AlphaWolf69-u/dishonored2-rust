//! Host command preparation with identities reserved by the live world's
//! allocator. Only publication touches the world or its component storage.
use bevy::{
    ecs::{system::Command, world::CommandQueue},
    prelude::*,
};

pub(super) enum SetupCommands<'w, 's> {
    Direct(Commands<'w, 's>),
    Prepared {
        queue: CommandQueue,
        reservations: crate::rendering::bevy_backend::entity_preparation::Reservations,
    },
}
impl<'w, 's> SetupCommands<'w, 's> {
    pub fn prepared(world: &mut World) -> Self {
        crate::rendering::bevy_backend::entity_preparation::initialize(world);
        Self::reserved(crate::rendering::bevy_backend::entity_preparation::Reservations::new(world))
    }
    pub fn reserved(reservations: crate::rendering::bevy_backend::entity_preparation::Reservations) -> Self {
        Self::Prepared { queue: Default::default(), reservations }
    }
    pub fn queue<C: Command<Out = ()>>(&mut self, command: C) {
        match self {
            Self::Direct(commands) => commands.queue(command),
            Self::Prepared { queue, .. } => queue.push(command),
        }
    }
    pub fn insert_resource<R: Resource>(&mut self, resource: R) {
        self.queue(move |world: &mut World| {
            world.insert_resource(resource);
        });
    }
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> SetupEntity<'_, 'w, 's> {
        let id = match self {
            Self::Direct(commands) => commands.spawn(bundle).id(),
            Self::Prepared { reservations, queue } => {
                let (id, receipt) = reservations.reserve();
                queue.push(move |world: &mut World| {
                    world.spawn_at(id, bundle).expect("reserved scene entity");
                    receipt.published();
                });
                id
            }
        };
        SetupEntity { commands: self, id }
    }
    pub fn publish(self, world: &mut World) {
        if let Self::Prepared { mut queue, reservations } = self {
            reservations.validate(world);
            queue.apply(world);
        }
    }
}
pub(super) struct SetupEntity<'a, 'w, 's> {
    commands: &'a mut SetupCommands<'w, 's>,
    id: Entity,
}
impl<'a, 'w, 's> SetupEntity<'a, 'w, 's> {
    pub fn id(&self) -> Entity {
        self.id
    }
    pub fn insert<B: Bundle>(&mut self, bundle: B) -> &mut Self {
        let id = self.id;
        self.commands.queue(move |world: &mut World| {
            world.entity_mut(id).insert(bundle);
        });
        self
    }
    pub fn with_children(
        &mut self,
        spawn: impl FnOnce(&mut SetupChildren<'_, 'w, 's>),
    ) -> &mut Self {
        spawn(&mut SetupChildren {
            commands: self.commands,
            parent: self.id,
        });
        self
    }
}
pub(super) struct SetupChildren<'a, 'w, 's> {
    commands: &'a mut SetupCommands<'w, 's>,
    parent: Entity,
}
impl<'a, 'w, 's> SetupChildren<'a, 'w, 's> {
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> SetupEntity<'_, 'w, 's> {
        self.commands.spawn((bundle, ChildOf(self.parent)))
    }
}
