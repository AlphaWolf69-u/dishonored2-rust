//! Sensor27 query-phase composition; all slots and query executors stay in
//! PhysicsScene's existing shared owner, including synchronous3b4610 refine.
use super::*;
use crate::gameplay::{
    collision_queries::QueryId,
    entity_lifetime::{EntityId, LiveEntities},
    occluded_sensor::{ConvexRequest, Hit, Request},
    powers::far_reach::{self as acquisition, Acquired, Input},
};
struct Host<'a> {
    scene: &'a mut crate::app::physics_scene::PhysicsScene,
    live: &'a LiveEntities,
    player: EntityId,
    classes: &'a crate::app::physics_scene::occluded_sensor::GrappleClasses,
}
impl acquisition::Host for Host<'_> {
    fn submit(&mut self, request: Request) -> Result<QueryId, String> {
        self.scene.submit_sensor_query(request)
    }
    fn take(&mut self, id: &mut QueryId, output: &mut Vec<Hit>) -> Result<bool, String> {
        self.scene.take_sensor_query(id, output)
    }
    fn cancel(&mut self, id: &mut QueryId) -> Result<(), String> {
        self.scene.cancel_sensor_query(id)
    }
    fn refine(&mut self, request: ConvexRequest, output: &mut Vec<Hit>) -> Result<(), String> {
        self.scene.refine_sensor_query(&request, output)
    }
    fn choose(&self, hits: &[Hit], input: Input, radius: f32) -> Result<Option<Acquired>, String> {
        self.scene
            .choose_grapple_witness(self.live, self.player, hits, input, radius, self.classes)
    }
}
impl Runtime {
    pub fn acquire(
        &mut self,
        scene: &mut crate::app::physics_scene::PhysicsScene,
        live: &LiveEntities,
        player: &Player,
        view: &View,
        snapping: bool,
    ) -> Result<(), String> {
        let id = player
            .managed_entity()
            .ok_or("sensor27 Player identity absent")?;
        if self.acquisition_classes.is_none() {
            self.acquisition_classes = Some(scene.grapple_classes()?);
        }
        let mut host = Host {
            scene,
            live,
            player: id,
            classes: self.acquisition_classes.as_ref().unwrap(),
        };
        self.acquisition_frame = self.acquisition.submitted_input();
        let acquired = self.acquisition.gather(&mut host)?;
        self.acquired = acquired.map(|(_, acquired)| acquired);
        let vertical = player
            .attributes
            .get("ARK_ATTRIBUTE_GRAPPLE_VERTICAL_RANGE")
            + if self
                .acquisition_frame
                .is_some_and(|frame| frame.direction.z > 0.)
            {
                (player.physics.posture_settings(0).height
                    - player.physics.current_settings().height)
                    .max(0.)
            } else {
                0.
            };
        let (origin, direction, length) = Sensor::query_segment(
            &player.physics,
            view.origin,
            view.axis,
            player
                .attributes
                .get("ARK_ATTRIBUTE_GRAPPLE_HORIZONTAL_RANGE"),
            vertical,
        );
        //3b6c90 compares the current float against the retained authored base;
        //a positive literal or owning the upgrade is not an equivalent test.
        let tag = "ARK_ATTRIBUTE_GRAPPLE_PULL_OBJECTS";
        let pull_objects = player
            .attributes
            .1
            .get(tag)
            .is_some_and(|value| player.attributes.get(tag) != value.base);
        self.next_acquisition_input = Some(Input {
            eye: view.origin,
            view_axis: view.axis,
            player_origin: player.physics.position(),
            player_axis: player.physics.axis(),
            origin,
            direction,
            length,
            horizontal_range: player
                .attributes
                .get("ARK_ATTRIBUTE_GRAPPLE_HORIZONTAL_RANGE"),
            vertical_range: vertical,
            snapping,
            pull_objects,
        });
        Ok(())
    }
    pub fn submit_acquisition(
        &mut self,
        scene: &mut crate::app::physics_scene::PhysicsScene,
        live: &LiveEntities,
        player: &Player,
    ) -> Result<(), String> {
        self.acquisition.confirm_target(
            self.object
                .map(|target| target.origin)
                .or_else(|| self.target.map(|target| target.sensor_position)),
        );
        let Some(input) = self.next_acquisition_input.take() else {
            return Ok(());
        };
        let id = player
            .managed_entity()
            .ok_or("sensor27 Player identity absent")?;
        let classes = self
            .acquisition_classes
            .as_ref()
            .ok_or("sensor27 class ranges absent")?;
        self.acquisition.submit(
            &mut Host {
                scene,
                live,
                player: id,
                classes,
            },
            input,
        )
    }
    pub fn cancel_acquisition(
        &mut self,
        scene: &mut crate::app::physics_scene::PhysicsScene,
        live: &LiveEntities,
        player: &Player,
    ) -> Result<(), String> {
        let id = player
            .managed_entity()
            .ok_or("sensor27 Player identity absent during retirement")?;
        if let Some(classes) = self.acquisition_classes.as_ref() {
            self.acquisition.cancel(&mut Host {
                scene,
                live,
                player: id,
                classes,
            })?;
        }
        self.acquired = None;
        self.acquisition_frame = None;
        self.next_acquisition_input = None;
        if !self.has_activity() {
            self.object = None;
        }
        Ok(())
    }
}
