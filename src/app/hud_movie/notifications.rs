//!6734b0 item presentation requested by actual pickup producers.
//!PauseMenu owns the mailbox; silent inventory mutations do not enqueue it.
use super::*;
use std::collections::VecDeque;

#[derive(Clone)]
pub(in crate::app) enum Pickup {
    Item { item: Arc<crate::assets::inventory_item::PickableItem>, count: u32, label: Option<Arc<str>> },
    Currency { count: i32 },
}
///6733c0 reads these three members of the actual HUD-equipment declaration.
///They belong to the HUD movie lifetime, not to a synthesized inventory item.
pub(super) struct CurrencyLabels { pub single:String, pub plural:String, pub image:String }
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
        let (label,image,count)=match &pickup {
            Pickup::Currency {count}=>{
                let labels=self.currency_labels.as_ref().ok_or("currency pickup HUD declaration absent")?;
                (if *count<2 {labels.single.as_str()}else{labels.plural.as_str()},labels.image.as_str(),*count)
            },
            Pickup::Item {item,count,label}=>{
                let label = if let Some(label) = label {
                    label.as_ref()
                } else if *count < 2 {
                    item.name.as_str()
                } else {
                    //6734b0 diagnoses an absent plural and then continues with
                    //the simple name.166830 returns in the installed binary.
                    item.plural_name.as_ref().unwrap_or(&item.name).as_str()
                };
                //The same source path skips this item when no pickup image
                //exists; it does not fail or retire the entire HUD movie.
                let Some(image) = item.pickup_log_image() else {
                    return Ok(());
                };
                (label,image,*count as i32)
            },
        };
        self.player
            .call_path_method(
                "_pickUpLog",
                "_C_AddItem",
                &[
                    strings.lookup(label).into(),
                    image.into(),
                    count.into(),
                ],
            )
            .map_err(|error| self.player.describe(&error))?;
        self.hud_activity = true;
        self.last = None;
        Ok(())
    }
}
