//! Installed class/factory bindings shared by the level's typed component pools.
//! This creates pool storage; each type still supplies its real constructor to
//! ComponentPool::allocate. Reading metadata never executes foreign code.
use super::{
    declaration::DeclarationScheduling, entity::ComponentDeclaration, pool::ComponentPool,
};
use crate::{
    assets::class_metadata::{ClassDeclarations, ComponentPoolDeclaration},
    gameplay::class_registry::ClassRegistry,
    platform::cvars::CVars,
};

pub struct ComponentTypes {
    classes: ClassRegistry,
    factories: Vec<ComponentPoolDeclaration>,
    initialization: Vec<crate::assets::class_metadata::ComponentTypeDeclaration>,
    definitions: Vec<crate::assets::class_metadata::ComponentDefinition>,
    initialization_plan: super::dependencies::InitializationPlan,
}

impl ComponentTypes {
    pub fn new(installed: ClassDeclarations) -> Result<Self, String> {
        let classes = installed.registry()?;
        use super::dependencies::{Dependency, DependencyClass, InitializationPlan};
        let dependency_class = |identity| -> Result<DependencyClass, String> {
            Ok(DependencyClass {
                identity,
                type_range: classes.hierarchy().type_range(identity)?,
            })
        };
        let inputs = installed
            .component_pools
            .iter()
            .zip(&installed.component_definitions)
            .map(|(factory, definition)| {
                let dependencies = definition
                    .initialization
                    .iter()
                    .map(|d| {
                        Ok(Dependency {
                            class: dependency_class(d.class)?,
                            exact: d.exact,
                            reason: d.reason.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok((dependency_class(factory.class)?, dependencies))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let initialization_plan = InitializationPlan::build(&inputs)
            .map_err(|e| format!("installed component initialization graph: {e:?}"))?;
        Ok(Self {
            classes,
            factories: installed.component_pools,
            initialization: installed.component_types,
            definitions: installed.component_definitions,
            initialization_plan,
        })
    }

    pub fn classes(&self) -> &ClassRegistry {
        &self.classes
    }

    /// Actual virtual80 priority, resolved from the installed dependency graph.
    pub fn initialization_rank(&self, pool_index: u32) -> Result<i32, String> {
        self.factory(pool_index)?;
        self.initialization_plan
            .ranks
            .get(pool_index as usize)
            .copied()
            .ok_or_else(|| "component initialization rank is missing".into())
    }

    pub fn definition(
        &self,
        pool_index: u32,
    ) -> Result<&crate::assets::class_metadata::ComponentDefinition, String> {
        self.factory(pool_index)?;
        self.definitions
            .get(pool_index as usize)
            .ok_or_else(|| "component definition is missing".into())
    }

    pub fn factory(&self, pool_index: u32) -> Result<&ComponentPoolDeclaration, String> {
        self.factories
            .get(pool_index as usize)
            .filter(|p| p.pool_index == pool_index)
            .ok_or_else(|| "component pool ordinal has no installed factory".into())
    }

    /// Literal type setup/dependency definition inputs. These are not startup
    /// ranks; the dependency graph must resolve those before lifecycle dispatch.
    pub fn initialization(
        &self,
        pool_index: u32,
    ) -> Result<&crate::assets::class_metadata::ComponentTypeDeclaration, String> {
        self.initialization
            .get(pool_index as usize)
            .filter(|d| d.pool_index == pool_index)
            .ok_or_else(|| "component initialization descriptor is missing".into())
    }

    /// 946640 -> 48fa80: the actual capacity cvar feeds the existing typed pool's
    /// class-size/chunk policy. Respect the client's current cvar value.
    pub fn create_pool<T>(
        &self,
        pool_index: u32,
        cvars: &CVars,
    ) -> Result<ComponentPool<T>, String> {
        let factory = self.factory(pool_index)?;
        let variable = cvars.get(&factory.initial_count_cvar).ok_or_else(|| {
            format!(
                "component capacity cvar is unregistered: {}",
                factory.initial_count_cvar
            )
        })?;
        if variable.reset != factory.initial_count_reset {
            return Err(format!(
                "component capacity reset differs from the installed declaration: {}",
                factory.initial_count_cvar
            ));
        }
        ComponentPool::for_class(variable.int, &self.classes, factory.class)
    }

    pub fn declaration(
        &self,
        pool_index: u32,
        identity: usize,
        scheduling: DeclarationScheduling,
    ) -> Result<ComponentDeclaration, String> {
        ComponentDeclaration::from_class(
            identity,
            self.factory(pool_index)?.class,
            self.classes.hierarchy(),
            scheduling,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires installed executable; complete class/definition/graph execution audit"]
    fn installed_component_initialization_matches_executed_graph() {
        let types = ComponentTypes::new(
            ClassDeclarations::load(&crate::assets::installation::game_base().unwrap()).unwrap(),
        )
        .unwrap();
        let fixture: Vec<_> =
            include_str!("../../../tests/fixtures/component_initialization_order_digest.txt")
                .split_whitespace()
                .map(|s| u64::from_str_radix(s, 16).unwrap())
                .collect();
        let plan = &types.initialization_plan;
        assert_eq!(plan.groups.len() as u64, fixture[0]);
        let mut canonical = Vec::new();
        canonical.extend((plan.groups.len() as u32).to_le_bytes());
        for group in &plan.groups {
            canonical.extend((group.len() as u32).to_le_bytes());
            for &index in group {
                canonical.extend((index as u32).to_le_bytes());
            }
        }
        for (index, &rank) in plan.ranks.iter().enumerate() {
            assert_eq!(types.initialization_rank(index as u32).unwrap(), rank);
            canonical.extend(rank.to_le_bytes());
        }
        let digest = canonical.iter().fold(0xcbf29ce484222325u64, |hash, &b| {
            (hash ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
        assert_eq!(digest, fixture[1]);
    }

    #[test]
    #[ignore = "requires the installed Dishonored 2 executable"]
    fn installed_factories_feed_registered_capacity_values_and_class_sizes() {
        let base = crate::assets::installation::game_base().unwrap();
        let types = ComponentTypes::new(ClassDeclarations::load(&base).unwrap()).unwrap();
        let mut cvars = CVars::new();
        for index in 0..350 {
            let factory = types.factory(index).unwrap();
            let pool = types.create_pool::<()>(index, &cvars).unwrap();
            assert_eq!(pool.roster.indices.len(), 0);
            assert_eq!(pool.items_per_chunk, 100);
            let declaration = types
                .declaration(
                    index,
                    index as usize,
                    DeclarationScheduling::from_base_flags(0),
                )
                .unwrap();
            assert_eq!(
                declaration.class_id,
                types.classes.hierarchy().type_id(factory.class).unwrap()
            );
            assert_eq!(cvars.int(&factory.initial_count_cvar), Some(0));
        }
        // Water's factory ordinal30 is separate from its derived hierarchy ID.
        // A configured non-default capacity must use the real class size.
        let water = types.factory(30).unwrap();
        let class = types.classes.metadata(water.class).unwrap();
        assert_eq!(
            types.classes.name_text(class.name).unwrap(),
            b"arkCpntWater"
        );
        assert_eq!(class.instance_size, 64);
        let cvar = cvars.find(&water.initial_count_cvar).unwrap();
        cvars.set(cvar, Some("1001"), true);
        let mut pool = types
            .create_pool::<crate::gameplay::water_component::WaterComponent>(30, &cvars)
            .unwrap();
        assert_eq!(pool.items_per_chunk, 1000);
        assert_eq!(pool.roster.indices.len(), 2000);
        // Construct the real water type through the shared entity factory.
        // Immediate start is disabled, so no unspecified virtual38 is supplied.
        let declaration = types
            .declaration(30, 91, DeclarationScheduling::from_base_flags(0))
            .unwrap();
        let mut entity = super::super::entity::EntityComponents::new(17);
        let mut registry = super::super::ComponentRegistry {
            next_generation: 37,
            maximum_registered: 0,
            registered: crate::storage::PointerList::with_capacity(64, 0x3f).unwrap(),
            pending_start: crate::storage::PointerList::with_capacity(64, 0x3f).unwrap(),
            pending_stop: crate::storage::PointerList::with_capacity(64, 0x3f).unwrap(),
            processed_stop_count: 0,
            processed_initialization_count: 0,
            processed_start_count: 0,
            queue_reset_count: 0,
            pending_release: crate::storage::PointerList::owned_with_capacity(64).unwrap(),
        };
        let handle = super::super::entity::ComponentFactory {
            declaration: &declaration,
            pool: &mut pool,
            registry: &mut registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut entity,
            false,
            |_, _, _| Ok(Default::default()),
            |_, _, _| panic!("unrequested startup notification"),
        )
        .unwrap()
        .unwrap();
        let component = pool.get(handle.slot).unwrap();
        assert_eq!(handle.class_id, declaration.class_id);
        assert_eq!(component.owner.generation, 37);
        assert_eq!(component.owner.entity, Some(17));
        assert_eq!(component.owner.declaration, Some(91));
        assert!(component.plane().is_none());
        assert_eq!(registry.registered.count(), 1);
        let mut actors = crate::gameplay::physics_actor::PhysicsActorLinks {
            actors: types.create_pool(32, &cvars).unwrap(),
            bodies: vec![crate::gameplay::physics_actor::ActorBody {
                actor: None,
                body_id: u32::MAX,
            }],
        };
        let declaration = types
            .declaration(32, 92, DeclarationScheduling::reference_frame(0))
            .unwrap();
        let handle = super::super::entity::ComponentFactory {
            declaration: &declaration,
            pool: &mut actors.actors,
            registry: &mut registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut entity,
            false,
            |_, _, _| Ok(crate::gameplay::physics_actor::PhysicsActor::new(0)),
            |_, _, _| panic!("unrequested startup notification"),
        )
        .unwrap()
        .unwrap();
        let pose = crate::gameplay::reference_frame::ReferenceFrame {
            position: [1., 2., 3.],
            ..Default::default()
        };
        actors.actors[usize::from(handle.slot)].set_transform(pose, false);
        let created = actors.actors.get(handle.slot).unwrap();
        assert_eq!(created.frame.current.position, pose.position);
        assert_eq!(created.owner.generation, 38);
        assert_eq!(created.owner.entity, Some(17));
        assert_eq!(created.owner.declaration, Some(92));

        let trigger_pool_index = types
            .factories
            .iter()
            .find(|factory| {
                let metadata = types.classes.metadata(factory.class).unwrap();
                types.classes.name_text(metadata.name).unwrap() == b"arkCpntTrigger"
            })
            .unwrap()
            .pool_index;
        let mut triggers = types
            .create_pool::<crate::gameplay::trigger_body::TriggerComponent>(
                trigger_pool_index,
                &cvars,
            )
            .unwrap();
        let declaration = types
            .declaration(
                trigger_pool_index,
                93,
                DeclarationScheduling::from_base_flags(0),
            )
            .unwrap();
        let handle = super::super::entity::ComponentFactory {
            declaration: &declaration,
            pool: &mut triggers,
            registry: &mut registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut entity,
            false,
            |_, _, _| crate::gameplay::trigger_body::TriggerComponent::new(&mut actors.bodies),
            |_, _, _| panic!("unrequested trigger startup notification"),
        )
        .unwrap()
        .unwrap();
        let trigger = triggers.get(handle.slot).unwrap();
        assert_eq!(trigger.owner.generation, 39);
        assert_eq!(trigger.owner.entity, Some(17));
        assert_eq!(trigger.owner.declaration, Some(93));
        assert_eq!(trigger.movement_body, 1);
        assert_eq!(actors.bodies[trigger.movement_body].body_id, 0x00ff_ffff);
        assert!(actors.bodies[trigger.movement_body].actor.is_none());
        let data = crate::gameplay::collision_user_data::CollisionUserData { component: handle };
        assert_eq!(
            data.entity(|h| {
                (h.class_id == declaration.class_id)
                    .then(|| triggers.get(h.slot))
                    .flatten()
                    .map(|value| &value.owner)
            })
            .unwrap(),
            17
        );
    }
}
