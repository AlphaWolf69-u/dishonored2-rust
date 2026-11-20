//! A placed health potion and the saved laboratory occlusion viewpoint.
//! The player's real sensor, F binding, inventory, and pickup lifecycle run normally.
use super::{super as app, *};
use crate::gameplay::{entity_lifetime::LiveEntities, interaction_sensor::Scratch};
use std::sync::Arc;

const POTION: &str = "pickups_life_potion_01_14";
const BOTTLE: &str = "env_interiors_lab_bottle_03_5";
// Installed script_waypoint_174 and F8 1793082964979.
const OPEN_FEET: Vec3 = Vec3::new(37.880363, 94.37499, 12.829023);
const BLOCKED_FEET: Vec3 = Vec3::new(68.673, 8.303004, 32.943993);

#[derive(Debug, Clone, Copy)]
enum Phase {
    AimPotion,
    ReleasePotion,
    Collect,
    AimWall,
    ReleaseWall,
    ObserveWall,
}

pub(super) struct Observation {
    potion: EntityId,
    bottle: EntityId,
    path: Arc<str>,
    before: u32,
    expected: u32,
    phase: Phase,
    since: Instant,
    input: Input,
    movement: Option<Input>,
}

fn restore(world: &mut World, feet: Vec3, toward: Vec3) -> Result<(), String> {
    let delta = toward - feet;
    let mut owner = world.resource_mut::<PlayerState>();
    owner
        .player
        .as_mut()
        .ok_or("pickup observation player absent")?
        .teleport(feet, delta.y.atan2(delta.x));
    Ok(())
}

fn center(world: &World, id: EntityId) -> Result<Vec3, String> {
    let bounds = app::world_models::entity_bounds(world, world.resource::<LiveEntities>(), id)?
        .ok_or("pickup observation placed model bounds absent")?;
    Ok((Vec3::from_array(bounds[0]) + Vec3::from_array(bounds[1])) * 0.5)
}

fn selection(world: &World) -> Option<EntityId> {
    world
        .resource::<app::interactions::Interactions>()
        .selected_interaction_target(world.resource::<LiveEntities>())
}

pub(super) fn within_pickup_distance(world: &World, point: Vec3) -> Result<bool, String> {
    let p = player(world)?;
    let state = p.machine.state(StateId::LOOT_OBJECT)
        .ok_or("pickup observation has no LootObject declaration")?;
    let crate::gameplay::player::states::Kind::LootObject(decl) = &state.decl.kind else {
        return Err("pickup observation found a different declared state".into());
    };
    let bounds = world.resource::<app::player_arms::PlayerArms>().body_model_bounds(p)?;
    let center = (Vec3::from(bounds[0]) + Vec3::from(bounds[1])) * 0.5;
    Ok(center.distance_squared(point - Vec3::Z * decl.object_pickup_z_offset)
        < decl.distance * decl.distance)
}

impl Observation {
    pub(super) fn begin(world: &mut World) -> Result<Self, String> {
        let live = world.resource::<LiveEntities>();
        let potion = live
            .find(POTION)
            .ok_or("laboratory health potion not placed")?;
        let bottle = live.find(BOTTLE).ok_or("laboratory bottle not placed")?;
        let scene = world.resource::<app::physics_scene::PhysicsScene>();
        let (_, component) = scene
            .interaction_pickable(live, potion)?
            .ok_or("placed health potion lacks started Pickable")?;
        let item = component
            .item
            .as_ref()
            .ok_or("placed health potion inventory lease absent")?;
        if component.definition.hold.is_some() || item.note_type != -1 || item.type_number != 14 {
            return Err("laboratory potion does not exercise the ordinary inventory route".into());
        }
        let path = item.path.clone();
        let before = world.resource::<DiscoveredDocuments>().quantity(&path);
        let count = u32::try_from(component.definition.quantity)
            .map_err(|_| "negative placed potion quantity")?;
        let expected = before
            .checked_add(count)
            .ok_or("placed potion quantity overflow")?;
        if count == 0
            || expected
                > world
                    .resource::<app::interactions::Interactions>()
                    .limits
                    .maximum(&path)
        {
            return Err(format!(
                "placed potion observation requires room for authored pack: before={before} count={count}"
            ));
        }
        let point = center(world, potion)?;
        restore(world, OPEN_FEET, point)?;
        // Use the same physical mouse capture entry as the wall observation.
        let capture = input_for(19)?;
        send(world, capture, true)?;
        send(world, capture, false)?;
        eprintln!(
            "ACTION_PLACED_PICKUP_BEGIN item={path} actor={potion:?} before={before} expected={expected} installed_quantity={count}"
        );
        Ok(Self {
            potion,
            bottle,
            path,
            before,
            expected,
            phase: Phase::AimPotion,
            since: Instant::now(),
            input: input_for(22)?,
            movement: None,
        })
    }

    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool, String> {
        if self.since.elapsed() > Duration::from_secs(45) {
            if let Some(movement) = self.movement.take() {
                send(world, movement, false)?;
            }
            return Err(format!(
                "placed pickup observation timed out in {:?}: selected={:?} quantity={} player={:?}",
                self.phase,
                selection(world),
                world.resource::<DiscoveredDocuments>().quantity(&self.path),
                player(world)?.feet()
            ));
        }
        let quantity = world.resource::<DiscoveredDocuments>().quantity(&self.path);
        match self.phase {
            Phase::AimPotion | Phase::AimWall => {
                let wall = matches!(self.phase, Phase::AimWall);
                let id = if wall { self.bottle } else { self.potion };
                let live = world.resource::<LiveEntities>();
                let point = if wall {
                    Vec3::from_array(
                        world
                            .resource::<app::physics_scene::PhysicsScene>()
                            .entity_frame(live, id)?
                            .ok_or("blocked bottle transform absent")?
                            .position,
                    )
                } else {
                    center(world, id)?
                };
                let (eye, _) = player(world)?.published_camera_frame();
                if !super::wall_thrust::Observation::steer(
                    world,
                    (point - eye).normalize_or_zero(),
                )? {
                    return Ok(false);
                }
                let selected = selection(world);
                if wall {
                    // Require the current camera cast to actually touch the intended
                    // bottle in a normal group, and an occluder before that witness.
                    let frame = *world
                        .query_filtered::<&Transform, With<Camera3d>>()
                        .single(world)
                        .map_err(|e| e.to_string())?;
                    let eye = Vec3::from_array(app::view_to_game(frame.translation));
                    let forward = Vec3::from_array(app::view_to_game(frame.forward().as_vec3()));
                    let sensor = world.resource::<app::interactions::Interactions>().sensor;
                    let live = world.resource::<LiveEntities>();
                    let scene = world.resource::<app::physics_scene::PhysicsScene>();
                    let mut query = Default::default();
                    let mut casts = Scratch::default();
                    scene.cast_interaction_sensor(
                        live, eye, forward, sensor, &mut query, &mut casts,
                    )?;
                    let mut blocked_witness = None;
                    for hit in casts.hits[..3].iter().flatten().filter(|hit| hit.entity == Some(id)) {
                        let witness = Vec3::from_array(app::view_to_game(hit.position));
                        if !scene.interaction_occluded(live, id, eye, witness, sensor.shape_radii[0], &mut query)? {
                            return Err(format!(
                                "saved laboratory bottle has an exposed surface from actual eye {eye:?} at {witness:?}"
                            ));
                        }
                        blocked_witness = Some(witness);
                    }
                    let Some(witness) = blocked_witness else { return Ok(false) };
                    if selected == Some(id) {
                        return Err(format!("occluded laboratory bottle still selected from {eye:?} at {witness:?}"));
                    }
                    if selected.is_some() {
                        return Ok(false);
                    }
                    eprintln!(
                        "ACTION_PLACED_PICKUP_WALL body={id:?} eye={eye:?} contact={witness:?} blocked=true selected={selected:?}"
                    );
                } else if selected != Some(id) {
                    return Ok(false);
                }
                if !wall {
                    if !within_pickup_distance(world, point)? {
                        if self.movement.is_none() {
                            let movement = input_for(2)?;
                            send(world, movement, true)?;
                            self.movement = Some(movement);
                        }
                        return Ok(false);
                    }
                    if let Some(movement) = self.movement.take() {
                        send(world, movement, false)?;
                    }
                }
                send(world, self.input, true)?;
                self.phase = if wall {
                    Phase::ReleaseWall
                } else {
                    Phase::ReleasePotion
                };
                self.since = Instant::now();
            }
            Phase::ReleasePotion | Phase::ReleaseWall => {
                send(world, self.input, false)?;
                self.phase = if matches!(self.phase, Phase::ReleasePotion) {
                    Phase::Collect
                } else {
                    Phase::ObserveWall
                };
            }
            Phase::Collect => {
                if quantity != self.before && quantity != self.expected {
                    return Err(format!("unexpected potion quantity {quantity}"));
                }
                let live = world.resource::<LiveEntities>();
                let scene = world.resource::<app::physics_scene::PhysicsScene>();
                if quantity != self.expected
                    || scene.interaction_pickable(live, self.potion)?.is_some()
                    || selection(world) == Some(self.potion)
                {
                    return Ok(false);
                }
                eprintln!(
                    "ACTION_SMOKE_PASS check=placed_ordinary_potion_F inventory_before={} after={} same_generation_unavailable=true",
                    self.before, quantity
                );
                let toward = center(world, self.bottle)?;
                restore(world, BLOCKED_FEET, toward)?;
                self.phase = Phase::AimWall;
                self.since = Instant::now();
            }
            Phase::ObserveWall => {
                // Wait for ordinary player input/state callbacks and check that the
                // earlier accepted pickup was delivered only once.
                let p = player(world)?;
                if quantity != self.expected
                    || selection(world) == Some(self.bottle)
                    || p.world_pickup_target() == Some(self.bottle)
                    || p.pending_loot_object() == Some(self.bottle)
                    || p.carrying
                        .held
                        .as_ref()
                        .is_some_and(|held| held.entity == self.bottle)
                    || world
                        .resource::<app::physics_scene::PhysicsScene>()
                        .interaction_pickable(world.resource::<LiveEntities>(), self.bottle)?
                        .is_none()
                {
                    return Err("blocked F acquired the bottle, changed the quantity, or retained its prompt".into());
                }
                if self.since.elapsed() >= Duration::from_secs(1) {
                    eprintln!(
                        "ACTION_SMOKE_PASS check=blocked_placed_bottle_F target_still_present=true held=false quantity={quantity}"
                    );
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}
