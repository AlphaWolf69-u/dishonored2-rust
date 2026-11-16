//!6734b0 item presentation requested by actual pickup producers.
//!PauseMenu owns the mailbox; silent inventory mutations do not enqueue it.
use super::*;
use std::collections::VecDeque;

#[derive(Clone)]
pub(in crate::app) struct Pickup {
    pub(in crate::app) item: Arc<crate::assets::inventory_item::PickableItem>,
    pub(in crate::app) count: u32,
}
#[derive(Default)]
pub(in crate::app) struct Mailbox {
    pub generation: u64,
    pub pending: VecDeque<Pickup>,
}
impl HudMovie {
    pub(in crate::app) fn pickup(&mut self, pickup: Pickup) -> Result<(), String> {
        let enabled = {
            let console = crate::platform::console::shared().read();
            console
                .cvars
                .find("ark_ui_show_pickup_log")
                .is_some_and(|index| console.cvars.list[index].int != 0)
        };
        if !enabled {
            return Ok(());
        }
        let strings = self
            .player
            .state
            .borrow()
            .strings
            .clone()
            .ok_or("HUD string table absent")?;
        let label =
            if pickup.count < 2 {
                &pickup.item.name
            } else {
                //6734b0 diagnoses an absent plural and then continues with
                //the simple name.166830 returns in the installed binary.
                pickup.item.plural_name.as_ref().unwrap_or(&pickup.item.name)
            };
        //The same source path skips this item when no pickup image exists;
        //it does not fail or retire the entire HUD movie.
        let Some(image) = pickup.item.pickup_log_image() else {
            return Ok(());
        };
        self.player
            .call_path_method(
                "_pickUpLog",
                "_C_AddItem",
                &[
                    strings.lookup(label).into(),
                    image.into(),
                    (pickup.count as i32).into(),
                ],
            )
            .map_err(|error| self.player.describe(&error))?;
        self.hud_activity = true;
        self.last = None;
        Ok(())
    }
}
