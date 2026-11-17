//!672780/672f70 crosshair and673760/673d30 equipped-item Flash binding.
//!Only scalar presentation state crosses the movie worker; inventory items,
//!weapon magazines and component declarations retain their gameplay owners.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) struct Snapshot {
    gadget: [i32; 3],
    power: Option<(i32, bool, bool)>,
    cancel_power: Option<i32>,
    crosshair: i32,
    kind: i32,
    dot: i32,
    ratio: f32,
    sizes: [f32; 2],
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            gadget: [0, -1, -1],
            power: None,
            cancel_power: None,
            crosshair: 0,
            kind: -1,
            dot: 0,
            ratio: -1.,
            sizes: [0.; 2],
        }
    }
}

pub(in crate::app) fn snapshot(
    player: &crate::gameplay::player::Player,
    inventory: &crate::app::interaction_focus::DiscoveredDocuments,
) -> Snapshot {
    let mut out = Snapshot::default();
    //460580 invokes each actually active state's status callback.856820
    //adds the selected power's cancel row for the real UsePower63 owner;
    //a selected/affordable power alone does not produce a cancel prompt.
    if player.active_player_states().iter().any(|state| state.0 == 63) {
        out.cancel_power = player.selected_power().map(|cast| cast.settings.id);
    }
    if let Some(path) = inventory.equipped(1) {
        if let Some((equipment, _, _)) = player.equipment_target_metadata(path) {
            out.kind = equipment.hud.kind;
            if !equipment.hud.use_crosshair {
                out.crosshair = -1;
            }
            if !equipment.hud.use_dot {
                out.dot = -1;
            }
            if let Some(weapon) = player
                .weapon
                .as_ref()
                .filter(|_| player.ranged_identity() == Some(path))
            {
                if let Some(ammo) = inventory.declaration(&weapon.ammunition().item) {
                    out.gadget = [ammo.hud_index, weapon.total_ammo as i32, weapon.magazine];
                }
                out.crosshair = if out.crosshair == -1 {
                    -1
                } else if weapon.can_fire() {
                    0
                } else {
                    2
                };
                if let Some(source) = &weapon.fire_source {
                    //6726d0 uses arkFlashWidth/720, not the render target's
                    //pixel width. Firing owns the actual dispersion values.
                    let width = {
                        let console = crate::platform::console::shared().read();
                        console
                            .cvars
                            .find("arkFlashWidth")
                            .map_or(1920., |index| console.cvars.list[index].float)
                    };
                    let minimum = (source.minimum_dispersion * (width / 720.))
                        .max(equipment.hud.minimum_size);
                    out.sizes = [
                        minimum,
                        (source.maximum_dispersion * (width / 720.)).max(minimum),
                    ];
                    out.ratio = weapon.dispersion_ratio();
                }
            } else if let Some(weapon) = player
                .throwable
                .as_ref()
                .filter(|_| equipment.archetype == 4)
            {
                //6729b0 selects the authored lob variant from the live
                //throwable's low-throw state, not from the inventory item.
                if weapon.low_throw && matches!(out.kind, 4 | 5) {
                    out.kind += 2;
                }
                if let Some(ammo) = weapon
                    .ammunition()
                    .and_then(|ammo| inventory.declaration(&ammo.item))
                {
                    out.gadget = [ammo.hud_index, weapon.total_ammo as i32, weapon.magazine];
                }
            } else if equipment.archetype == 5 {
                if let (Some(item), Some((_, available))) = (
                    inventory.declaration(path),
                    player.selected_power_hud_status(),
                ) {
                    //673760 also queries selectedPower virtual130. Blink's
                    //141b9bb78 slot points to43cc00, reading base byte+a4:
                    //61bc20 sets it at Begin,61be50 clears it at End. The
                    //same Cast owner retains that interval beyond UsePower63.
                    let casting = player.selected_power().is_some_and(|cast|
                        cast.phase()!=crate::gameplay::powers::Phase::Idle) || player
                        .active_player_states()
                        .iter()
                        .any(|state| state.0 == 63);
                    //673760 publishes (61bce0 == 0), and673d30 forwards it
                    //unchanged: _C_SetPower's third argument is bHasEnoughMana.
                    out.power = Some((item.hud_index, casting, available));
                }
            } else if matches!(equipment.archetype, 7 | 9) && inventory.quantity(path) > 0 {
                if let Some(item) = inventory.declaration(path) {
                    out.gadget = [item.hud_index, -1, -1];
                }
            }
            //672780's swimming gate belongs only to the crosshair.673760
            //continues displaying the selected equipment/ammunition icon.
            if player.is_swimming() && !equipment.usable_swimming {
                let default = Snapshot::default();
                out.crosshair = default.crosshair;
                out.kind = default.kind;
                out.dot = default.dot;
                out.ratio = default.ratio;
                out.sizes = default.sizes;
            }
        }
    }
    //672f70 explicitly suppresses both shapes during the real Killcam state.
    if player
        .active_player_states()
        .iter()
        .any(|state| state.0 == 83)
    {
        out.crosshair = -1;
        out.dot = -1;
    }
    out
}

impl HudMovie {
    pub(in crate::app) fn equipment(&mut self, input: Snapshot) -> Result<(), String> {
        self.equipment_input = Some(input);
        self.apply_equipment(input)
    }
    pub(super) fn apply_equipment(&mut self, input: Snapshot) -> Result<(), String> {
        self.status.publish(&mut self.player, input.cancel_power)?;
        if let Some((index, activated, available)) = input.power {
            self.player
                .call_path_method(
                    "_masterHUD",
                    "_C_SetPower",
                    &[index.into(), activated.into(), available.into()],
                )
                .map_err(|error| self.player.describe(&error))?;
        } else {
            self.player
                .call_path_method("_masterHUD", "_C_SetGadget", &input.gadget.map(Into::into))
                .map_err(|error| self.player.describe(&error))?;
        }
        let style = {
            let console = crate::platform::console::shared().read();
            console
                .cvars
                .find("ark_ui_crosshair_style")
                .map_or(2, |index| console.cvars.list[index].int)
        };
        self.player
            .call_path_method(
                "_crosshair",
                "_C_SetDot",
                &[if style == 0 { -1 } else { input.dot }.into()],
            )
            .map_err(|error| self.player.describe(&error))?;
        self.player
            .call_path_method(
                "_crosshair",
                "_C_SetCrosshair",
                &[
                    if style < 2 { -1 } else { input.crosshair }.into(),
                    input.kind.into(),
                    f64::from(input.ratio).into(),
                    f64::from(input.sizes[0]).into(),
                    f64::from(input.sizes[1]).into(),
                ],
            )
            .map_err(|error| self.player.describe(&error))?;
        self.last = None;
        Ok(())
    }
}
