//! StatusInformation460580/45fe40, including UsePower856820's cancel row.
//! This is a separate movie owner from focused-world Interactives.
use super::*;
use crate::flash::avm2::value::Value;

pub(super) struct State {
    cancel_labels: [(i32, String); 7],
    displayed: Option<i32>,
}

impl State {
    pub(super) fn load(catalog: &Catalog) -> Result<Self, String> {
        use crate::gameplay::options::decl::{atom, scope};
        let read = |path: &str| Ok(catalog.read(path).map_err(|e| e.to_string())?
            .map(crate::assets::entities::owned_declaration_text));
        let state = crate::assets::entities::player::resolved_decl("gamelogicstateingame",
            "ui/gamelogic/state_ingame.gamelogicstateingame", read)?;
        let elements = scope(&state, "m_hudElements").ok_or("in-game state has no HUD elements")?;
        let path = atom(elements, "enumItem[ARK_HUD_ELEMENT_STATUSINFORMATION]")
            .ok_or("in-game state has no StatusInformation HUD")?;
        let fields = crate::assets::entities::player::resolved_decl("ui_hudelementstatusinformation", path, read)?;
        //856820 selects actual power subclasses; their source power IDs are
        //already decoded by power_journal. Zero/missing StringIds add no row.
        let cancel_labels = [
            (2, "m_cancelPowerUseBlinkLabel"),
            (7, "m_cancelPowerUsePossessionLabel"),
            (11, "m_cancelPowerUseDevouringSwarmLabel"),
            (5, "m_cancelPowerUseDoppelgangerLabel"),
            (4, "m_cancelPowerUseMesmerizeLabel"),
            (6, "m_cancelPowerUseGrappleLabel"),
            (12, "m_cancelPowerUsePhantasmalLinkLabel"),
        ].map(|(id, name)| (id, atom(&fields, name).unwrap_or("").to_owned()));
        Ok(Self { cancel_labels, displayed: None })
    }

    pub(super) fn publish(&mut self, player: &mut Player, power: Option<i32>) -> Result<(), String> {
        //460190 uses this same interactions cvar for StatusInformation.
        let enabled = {
            let console = crate::platform::console::shared().read();
            console.cvars.find("ark_ui_show_interactions")
                .is_some_and(|i| console.cvars.list[i].int != 0)
        };
        let label = power.filter(|_| enabled).and_then(|id| self.cancel_labels.iter()
            .find(|(kind, label)| *kind == id && !label.is_empty()));
        let next = label.map(|(id, _)| *id);
        if self.displayed == next { return Ok(()); }
        if let Some((_, label)) = label {
            let strings = player.state.borrow().strings.clone().ok_or("HUD has no string table")?;
            let row = Value::Object(player.vm.plain_object());
            player.set_member(&row, "label", strings.lookup(label).into()).map_err(|e| player.describe(&e))?;
            //856820 explicitly uses ACTION_INTERACT22;467470 resolves its
            //binding name. The movie renders the configured key/pad binding.
            player.set_member(&row, "keyName", crate::platform::bindings::action_label(22)
                .unwrap_or("").into()).map_err(|e| player.describe(&e))?;
            player.call_path_method("controllerTypeHandler", "_C_UpdateControllerType",
                &[pause_state::controller_type().into()]).map_err(|e| player.describe(&e))?;
            let rows = player.array(vec![row]);
            player.call_path_method("_statusInformation", "_C_ShowStatus", &[rows])
                .map_err(|e| player.describe(&e))?;
        } else {
            player.call_path_method("_statusInformation", "_C_Hide", &[])
                .map_err(|e| player.describe(&e))?;
        }
        self.displayed = next;
        Ok(())
    }
}
