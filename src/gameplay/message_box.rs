//! `arkMessageBoxManagerLocal` (`gameLocal+0x30f260`): message boxes drawn
//! by `global.iggy` over the parent movie (notes:
//! `artifacts/ui-pause/notes-messagebox.md`).
use crate::assets::lang::{key_id, StringTable};
use crate::flash::avm2::value::Value;
use crate::flash::Player;

/// The movie of `code_referenced/ui/global.iggyfile`.
pub const MOVIE: &str = "ui/global.iggy";
const HANDLER: &str = "gameService.gameMessageBoxHandler";

#[derive(Clone, Debug, PartialEq)]
pub enum Text {
    /// `m_textId`, localized when shown.
    Id(u32),
    /// `m_localizedText`.
    Literal(String),
}

/// `arkMessageBoxInfo` as the box uses it.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub timer: f64,
    pub text: Text,
    /// `m_buttonLabels` (string ids).
    pub labels: Vec<u32>,
    /// `m_callbacks` (the AS3 `gameCallbacks`).
    pub callbacks: Vec<String>,
}

impl Info {
    /// `arkMessageBox` ctor `FUN_140460f90`: the button set's labels and
    /// callback names.
    pub fn with_buttons(text: &str, buttons: u32, timer: f64) -> Info {
        let (labels, callbacks): (&[&str], &[&str]) = match buttons {
            0 => (&["ui_common_OK"], &["arkMessageBox::OnOkSelected"]),
            1 => (&["ui_common_cancel", "ui_common_yes"], &["arkMessageBox::OnCancelSelected", "arkMessageBox::OnOKSelected"]),
            2 => (&["ui_common_no", "ui_common_yes"], &["arkMessageBox::OnNoSelected", "arkMessageBox::OnYesSelected"]),
            3 => (&["ui_common_cancel", "ui_common_confirm"], &["arkMessageBox::OnCancelSelected", "arkMessageBox::OnRetrySelected"]),
            4 => (&["ui_common_back", "ui_loadgame"], &["arkMessageBox::OnBackSelected", "arkMessageBox::OnLoadSelected"]),
            5 => (&["ui_common_back", "ui_common_continueGame"], &["arkMessageBox::OnBackSelected", "arkMessageBox::OnContinue"]),
            6 => (&["ui_backtomainmenu", "ui_common_continueGame"], &["arkMessageBox::OnBackToMMM", "arkMessageBox::OnContinue"]),
            _ => (&[], &[]),
        };
        Info {
            timer,
            text: Text::Id(key_id(text)),
            labels: labels.iter().map(|l| key_id(l)).collect(),
            callbacks: callbacks.iter().map(|c| c.to_string()).collect(),
        }
    }

    /// `FUN_1404630f0` (`MessageBoxRequest` from AS3): text, button keys,
    /// callback names (one empty name per button when absent), timeout (-1
    /// when absent). The data object (args[4]) belongs to the sending movie
    /// and is not passed on (the global call gets 0).
    pub fn from_request(player: &mut Player, args: &[Value]) -> Info {
        let present = |v: Option<&Value>| v.filter(|v| !v.is_nullish() && !matches!(v, Value::Bool(_))).cloned();
        let text = present(args.first()).and_then(|v| player.vm.to_string(&v).ok()).map(|s| s.to_string()).unwrap_or_default();
        let strings = |player: &mut Player, v: Option<Value>| -> Vec<String> {
            let Some(v) = v else { return Vec::new() };
            let n = player.member(&v, "length").map(|l| l.primitive_to_number() as usize).unwrap_or(0);
            (0..n)
                .map(|i| player.member(&v, &i.to_string()).ok().and_then(|e| player.vm.to_string(&e).ok()).map(|s| s.to_string()).unwrap_or_default())
                .collect()
        };
        let labels = strings(player, present(args.get(1)));
        let callbacks = match present(args.get(2)) {
            Some(v) => strings(player, Some(v)),
            None => vec![String::new(); labels.len()],
        };
        Info {
            timer: present(args.get(3)).map_or(-1.0, |v| v.primitive_to_number()),
            text: Text::Literal(text),
            labels: labels.iter().map(|l| key_id(l)).collect(),
            callbacks,
        }
    }

    /// The five `_C_DisplayMessageBox` arguments (`FUN_1404623d0` /
    /// `FUN_140463520`).
    fn arguments(&self, player: &Player, strings: &StringTable) -> Vec<Value> {
        let text = match &self.text {
            Text::Id(id) => strings.text(*id).to_string(),
            Text::Literal(s) => s.clone(),
        };
        let labels = player.array(self.labels.iter().map(|&id| Value::from(strings.text(id))).collect());
        let callbacks = player.array(self.callbacks.iter().map(|c| Value::from(c.as_str())).collect());
        vec![Value::from(text.as_str()), labels, callbacks, self.timer.into(), 0.0.into()]
    }
}

/// Who gets the answer (`m_confirmedCallback`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Confirm {
    /// The options container (`FUN_1405171b0`).
    Options,
}

struct MessageBox {
    handle: i32,
    priority: i32,
    info: Info,
    answer: i32,
    confirm: Option<Confirm>,
    /// `m_file`: this box's global movie, loaded while it is the front box,
    /// and its resources (library index, path).
    global: Option<Player>,
    resources: Vec<(usize, String)>,
}

/// What a step asks of the host.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A box was answered; run its confirmed callback.
    Confirmed { confirm: Confirm, answer: i32 },
    /// The front box's global movie changed (new images to decode).
    GlobalChanged,
}

#[derive(Default)]
pub struct Manager {
    queue: Vec<MessageBox>,
    counter: i32,
    /// `+0x30`: answered boxes (`TakeResult`).
    pub results: Vec<(i32, i32)>,
    /// The front box still needs its global `_C_DisplayMessageBox` (the
    /// playback callback runs on the next update).
    pending_global: bool,
}

impl Manager {
    /// `IsActive`.
    pub fn active(&self) -> bool {
        !self.queue.is_empty()
    }

    /// The front box's global movie.
    pub fn global(&mut self) -> Option<&mut Player> {
        self.queue.first_mut()?.global.as_mut()
    }

    /// The front box's global movie and its resources.
    pub fn global_with_resources(&self) -> Option<(&Player, &[(usize, String)])> {
        let front = self.queue.first()?;
        Some((front.global.as_ref()?, &front.resources))
    }

    /// Show (`FUN_140462a60` -> `FUN_140462b00`). `load` loads the global
    /// movie (file ready included). Returns the handle.
    pub fn show(
        &mut self,
        info: Info,
        priority: i32,
        confirm: Option<Confirm>,
        parent: &mut Player,
        strings: &StringTable,
        load: &mut dyn FnMut() -> Result<(Player, Vec<(usize, String)>), String>,
    ) -> Result<(i32, Vec<Event>), String> {
        self.counter += 1;
        let b = MessageBox { handle: self.counter, priority, info, answer: 0, confirm, global: None, resources: Vec::new() };
        let at = self.queue.iter().position(|q| priority >= q.priority);
        let mut events = Vec::new();
        match at {
            Some(i) => {
                if i == 0 {
                    // `FUN_140462020`: the preempted box closes on its global movie.
                    if let Some(front) = self.queue.first_mut() {
                        if let Some(g) = front.global.as_mut() {
                            call(g, "_C_OnMessageBoxConfirmed", &[(front.answer as f64).into()])?;
                        }
                        front.global = None;
                    }
                }
                self.queue.insert(i, b);
                if i == 0 {
                    self.display(parent, strings, load)?;
                    events.push(Event::GlobalChanged);
                }
            }
            None => {
                self.queue.push(b);
                if self.queue.len() == 1 {
                    // SetActive(true) pushes NOTIFICATION, which pauses
                    // like the pause state already does.
                    self.display(parent, strings, load)?;
                    events.push(Event::GlobalChanged);
                }
            }
        }
        Ok((self.counter, events))
    }

    /// Display (`FUN_1404627e0`) with the file ready at once
    /// (`FUN_140462210`): layer 3, DisplayOnParent when this is the only box;
    /// the global display follows on the next update (playback callback).
    fn display(&mut self, parent: &mut Player, strings: &StringTable, load: &mut dyn FnMut() -> Result<(Player, Vec<(usize, String)>), String>) -> Result<(), String> {
        let only = self.queue.len() == 1;
        let front = &mut self.queue[0];
        let (player, resources) = load()?;
        front.global = Some(player);
        front.resources = resources;
        if only {
            let args = front.info.arguments(parent, strings);
            call(parent, "_C_DisplayMessageBox", &args)?;
        }
        self.pending_global = true;
        Ok(())
    }

    /// The playback callback (state 2): DisplayOnGlobal.
    pub fn update(&mut self, strings: &StringTable) -> Result<(), String> {
        if !std::mem::take(&mut self.pending_global) {
            return Ok(());
        }
        let Some(front) = self.queue.first_mut() else { return Ok(()) };
        let Some(g) = front.global.as_mut() else { return Ok(()) };
        let args = front.info.arguments(g, strings);
        call(g, "_C_DisplayMessageBox", &args)
    }

    /// `OnMessageBoxConfirmed(index)` from the global movie
    /// (`FUN_140463090`): the answer; the caller runs the confirmed callback
    /// and then [`Manager::close_front`].
    pub fn confirmed(&mut self, args: &[Value]) -> Option<Event> {
        let front = self.queue.first_mut()?;
        front.answer = args.first().map_or(0, |v| v.primitive_to_number() as i32);
        front.confirm.map(|confirm| Event::Confirmed { confirm, answer: front.answer })
    }

    /// Close of the front box (`FUN_140463a90`): the parent, then the global
    /// movie, get `_C_OnMessageBoxConfirmed(answer)`; the movie is unloaded
    /// and the next box shown.
    pub fn close_front(
        &mut self,
        parent: &mut Player,
        strings: &StringTable,
        load: &mut dyn FnMut() -> Result<(Player, Vec<(usize, String)>), String>,
    ) -> Result<Vec<Event>, String> {
        if self.queue.is_empty() {
            return Ok(Vec::new());
        }
        let only = self.queue.len() == 1;
        let mut b = self.queue.remove(0);
        if only {
            call(parent, "_C_OnMessageBoxConfirmed", &[(b.answer as f64).into()])?;
        }
        self.results.push((b.handle, b.answer));
        if let Some(g) = b.global.as_mut() {
            call(g, "_C_OnMessageBoxConfirmed", &[(b.answer as f64).into()])?;
        }
        drop(b);
        if !self.queue.is_empty() {
            self.display(parent, strings, load)?;
        }
        Ok(vec![Event::GlobalChanged])
    }
}

/// `gameService.gameMessageBoxHandler.<method>(args)`.
fn call(player: &mut Player, method: &str, args: &[Value]) -> Result<(), String> {
    player.call_path_method(HANDLER, method, args).map(|_| ()).map_err(|e| format!("{method}: {}", player.describe(&e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_sets_follow_the_ctor() {
        let i = Info::with_buttons("ui_main_settings_confirmation", 2, 0.0);
        assert_eq!(i.labels, vec![key_id("ui_common_no"), key_id("ui_common_yes")]);
        assert_eq!(i.callbacks, vec!["arkMessageBox::OnNoSelected", "arkMessageBox::OnYesSelected"]);
        assert!(Info::with_buttons("x", 7, 0.0).labels.is_empty());
    }
}
