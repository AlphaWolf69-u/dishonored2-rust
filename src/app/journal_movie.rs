//! The in-game journal movie and its per-open resource ownership.
use super::interaction_focus::{DocumentEntry, JournalItem};
use crate::assets::inventory_item::{self, ReadableKind};
use crate::{
    assets::{archive::Catalog, installation, lang},
    flash::{Player, player::CharRef, render},
    gameplay::pause_state,
    platform::bindings::{self, Keyboard},
    rendering::bevy_backend::flash_ui::{FlashFrame, Image},
};
use std::{collections::HashMap, sync::Arc};

pub(super) struct PowerData {
    pub definitions: Arc<(Vec<crate::assets::power_journal::Power>, String)>,
    pub quantities: HashMap<String, u32>,
    pub has_mark: bool,
    pub new_game_plus: bool,
    pub character: pause_state::Character,
}

fn quantity(data: &PowerData, path: &str) -> u32 {
    if path.bytes().all(|byte| !byte.is_ascii_uppercase()) {
        data.quantities.get(path).copied().unwrap_or(0)
    } else {
        data.quantities.get(&path.to_ascii_lowercase()).copied().unwrap_or(0)
    }
}

fn write_power_tree<'a>(player: &mut Player, strings: &crate::assets::lang::StringTable, data: &PowerData,
    target: &str, nodes: &'a [crate::assets::power_journal::Upgrade], ancestors: &mut Vec<&'a crate::assets::power_journal::Upgrade>) -> Result<(), String> {
    player.set_value(&format!("{target}.length"), (nodes.len() as u32).into()).map_err(|e| player.describe(&e))?;
    for (index, node) in nodes.iter().enumerate() {
        ancestors.push(node);
        let entry = format!("{target}.data[{index}]");
        if node.children.is_empty() {
            let upgrades = format!("{entry}.upgrades");
            player.set_value(&format!("{upgrades}.length"), (ancestors.len() as u32).into()).map_err(|e| player.describe(&e))?;
            for (index, upgrade) in ancestors.iter().enumerate() {
                let Some(info) = &upgrade.info else { return Err(format!("power upgrade {} has no item", upgrade.id)); };
                let id = crate::assets::power_journal::upgrade_id(&upgrade.id).ok_or_else(|| format!("unknown power upgrade {}", upgrade.id))?;
                let path = format!("{upgrades}.data[{index}]");
                for (field, text) in [("label", &info.name), ("description", &info.description), ("strategicTips", &info.tips)] {
                    player.set_value(&format!("{path}.{field}"), strings.lookup(text).into()).map_err(|e| player.describe(&e))?;
                }
                if let Some(image) = &info.image { player.set_value(&format!("{path}.image"), image.as_str().into()).map_err(|e| player.describe(&e))?; }
                let owned = upgrade.item.as_ref().is_some_and(|item| quantity(data, item) > 0);
                player.set_value(&format!("{path}.cost"), (if owned { 0 } else { info.rune_cost }).into()).map_err(|e| player.describe(&e))?;
                player.set_value(&format!("{path}.powerUpgradeID"), id.into()).map_err(|e| player.describe(&e))?;
            }
        } else {
            write_power_tree(player, strings, data, &format!("{entry}.tree"), &node.children, ancestors)?;
        }
        ancestors.pop();
    }
    Ok(())
}

pub(super) enum Action {
    Seen(ReadableKind, Option<u32>),
    Show(String),
    SeenMap(Option<u32>),
    SeenAudiographs,
    PlayAudiograph { item: u32, path: String },
    ShowHistory(bool),
    ShowMap(String),
    MarkTasks(bool, Vec<u32>),
    PurchasePower { root: bool, id: i32 },
}

pub(super) struct Movie {
    player: Player,
    images: Arc<HashMap<CharRef, Arc<Image>>>,
    cache: render::Cache,
    generation: u64,
    serial: u64,
    accumulated: f64,
    interval: f64,
    last: Option<FlashFrame>,
    documents: Vec<DocumentEntry>,
    maps: Vec<JournalItem>,
    audiographs: Vec<JournalItem>,
    purchase_object: Option<crate::flash::avm2::object::Obj>,
    focused: bool,
    _residency: Option<super::pause_menu::MovieResidency>,
}

impl Movie {
    pub(super) fn load(
        screen: (i32, i32),
        generation: u64,
        state_path: &str,
        documents: Vec<DocumentEntry>,
        maps: Vec<JournalItem>,
        audiographs: Vec<JournalItem>,
        playing_audiograph: Option<u32>,
        powers: Option<PowerData>,
        gameflow: Option<crate::gameplay::gameflow::State>,
    ) -> Result<Self, String> {
        let base = installation::game_base().map_err(|e| e.to_string())?;
        let catalog = Catalog::shared(&base).map_err(|e| e.to_string())?;
        let language = lang::read_text_language(&base)?;
        let path = super::pause_menu::state_movie(
            &catalog,
            "gamelogicstatejournal",
            state_path,
        )?;
        let (mut player, resources) = Player::load_installed(&catalog, language, &path)?;
        pause_state::file_ready(&mut player, screen)?;
        let strings = player
            .state
            .borrow()
            .strings
            .clone()
            .ok_or("journal has no string table")?;
        let shared = crate::platform::console::shared().read();
        let mut keyboard = Keyboard::default();
        let actions = bindings::bindable_actions(&shared.bindings, &strings, &mut keyboard);
        let keys = shared.bindings.keys_for_action(bindings::IN_GAME, 44);
        let first = keys
            .first()
            .map_or(-1, |&key| keyboard.virtual_key(key).unwrap_or(key) as i32);
        let second = keys
            .get(1)
            .map_or(-1, |&key| keyboard.virtual_key(key).unwrap_or(key) as i32);
        drop(shared);
        pause_state::write_bindable_actions(&mut player, &actions)
            .map_err(|e| player.describe(&e))?;
        // 140789b40 writes the selected character minus one before Show.
        // The movie's EnumCharacter defines Corvo=0 and Emily=1.
        let character = match super::pause_menu::debug_character() {
            pause_state::Character::Corvo => 0,
            pause_state::Character::Emily => 1,
        };
        player
            .set_value("characterIndex", character.into())
            .map_err(|e| player.describe(&e))?;
        player
            .call_path_method(
                "controllerTypeHandler",
                "_C_UpdateControllerType",
                &[1.into()],
            )
            .map_err(|e| player.describe(&e))?;
        // 140520890/140520af0 populate the two inventory lists in reverse
        // order, reading the actual declaration leases retained by inventory.
        for (kind, provider, new_flag) in [
            (
                ReadableKind::Document,
                "journalData.notes.documentsData.elements",
                "hasNewDocuments",
            ),
            (
                ReadableKind::Book,
                "journalData.notes.booksData.elements",
                "hasNewBooks",
            ),
        ] {
            let entries: Vec<_> = documents
                .iter()
                .rev()
                .filter(|entry| entry.kind == kind)
                .collect();
            player
                .set_value(&format!("{provider}.length"), (entries.len() as u32).into())
                .map_err(|e| player.describe(&e))?;
            let mut any_new = false;
            for (index, entry) in entries.iter().enumerate() {
                let item = &entry.declaration;
                let readable = match item.readable.as_ref() {
                    Some(inventory_item::PickableReadable::Note(readable)) => readable,
                    _ => return Err(format!("not a readable item: {}", entry.path)),
                };
                let element = format!("{provider}.data[{index}]");
                let set = |p: &mut Player, field: &str, value| {
                    p.set_value(&format!("{element}.{field}"), value)
                        .map_err(|e| p.describe(&e))
                };
                set(&mut player, "label", strings.lookup(&readable.title).into())?;
                set(
                    &mut player,
                    "description",
                    strings.lookup(&readable.text).into(),
                )?;
                if let Some(image) = &readable.journal_image {
                    set(&mut player, "image", image.as_str().into())?;
                }
                set(&mut player, "itemID", entry.item_id.into())?;
                set(&mut player, "isNew", entry.is_new.into())?;
                any_new |= entry.is_new;
            }
            player
                .set_value(&format!("journalData.notes.{new_flag}"), any_new.into())
                .map_err(|e| player.describe(&e))?;
        }
        // 1405216e0 consumes the actual inventory slots, outsider-mark
        // state, power component character flags and the power-system tree.
        if let Some(data) = &powers {
            let list: Vec<_> = data.definitions.0.iter().filter(|power| {
                let permitted = match data.character {
                    pause_state::Character::Corvo => power.usable_by.contains("USAGE_BY_CORVO"),
                    pause_state::Character::Emily => power.usable_by.contains("USAGE_BY_EMILY"),
                };
                data.has_mark && (data.new_game_plus || quantity(data, &power.item) > 0 || permitted)
            }).collect();
            player.set_value("journalData.powersData.runesCount", quantity(data, &data.definitions.1).into()).map_err(|e| player.describe(&e))?;
            player.set_value("journalData.powersData.powers.length", (list.len() as u32).into()).map_err(|e| player.describe(&e))?;
            for (index, power) in list.into_iter().enumerate() {
                let entry = format!("journalData.powersData.powers.data[{index}]");
                for (field, text) in [("label", &power.name), ("description", &power.description), ("strategicTips", &power.tips)] {
                    player.set_value(&format!("{entry}.{field}"), strings.lookup(text).into()).map_err(|e| player.describe(&e))?;
                }
                if let Some(image) = &power.image { player.set_value(&format!("{entry}.image"), image.as_str().into()).map_err(|e| player.describe(&e))?; }
                if let Some(image) = &power.header_image { player.set_value(&format!("{entry}.headerImage"), image.as_str().into()).map_err(|e| player.describe(&e))?; }
                player.set_value(&format!("{entry}.cost"), (if quantity(data, &power.item) > 0 { 0 } else { power.rune_cost }).into()).map_err(|e| player.describe(&e))?;
                let id = crate::assets::power_journal::power_id(&power.power_id).ok_or_else(|| format!("unknown player power {}", power.power_id))?;
                player.set_value(&format!("{entry}.powerUpgradeID"), id.into()).map_err(|e| player.describe(&e))?;
                player.set_value(&format!("{entry}.itemIndex"), power.hud_index.into()).map_err(|e| player.describe(&e))?;
                write_power_tree(&mut player, &strings, data, &format!("{entry}.tree"), &power.upgrades, &mut Vec::new())?;
            }
        }
        // 1405208e0 publishes the admitted audiographs in reverse order.
        let audio_entries: Vec<_> = audiographs.iter().rev().collect();
        player.set_value("journalData.notes.audiographsData.elements.length", (audio_entries.len() as u32).into()).map_err(|e| player.describe(&e))?;
        let mut audio_new = false;
        for (index, entry) in audio_entries.iter().enumerate() {
            let item = &entry.declaration;
            let (name, description, image) = item.audiograph_text().ok_or_else(|| format!("not an audiograph: {}", entry.path))?;
            let element = format!("journalData.notes.audiographsData.elements.data[{index}]");
            for (field, key) in [("label", name), ("description", description)] {
                player.set_value(&format!("{element}.{field}"), strings.lookup(key).into()).map_err(|e| player.describe(&e))?;
            }
            if let Some(image) = image {
                player.set_value(&format!("{element}.image"), image.into()).map_err(|e| player.describe(&e))?;
            }
            player.set_value(&format!("{element}.isNew"), entry.is_new.into()).map_err(|e| player.describe(&e))?;
            player.set_value(&format!("{element}.playing"), (playing_audiograph == Some(entry.item_id)).into()).map_err(|e| player.describe(&e))?;
            player.set_value(&format!("{element}.itemID"), entry.item_id.into()).map_err(|e| player.describe(&e))?;
            audio_new |= entry.is_new;
        }
        player.set_value("journalData.notes.hasNewAudiographs", audio_new.into()).map_err(|e| player.describe(&e))?;
        // 140520d50 uses the map-info title as label and item ID, in
        // reverse admission order, with the small journal image.
        let map_entries: Vec<_> = maps.iter().rev().collect();
        player.set_value("journalData.notes.mapsData.elements.length", (map_entries.len() as u32).into()).map_err(|e| player.describe(&e))?;
        let mut any_new = false;
        for (index, entry) in map_entries.iter().enumerate() {
            let item = &entry.declaration;
            let map = item.map().map_err(str::to_owned)?
                .ok_or_else(|| format!("not an interactive map: {}", entry.path))?;
            let element = format!("journalData.notes.mapsData.elements.data[{index}]");
            player.set_value(&format!("{element}.label"), strings.lookup(&map.title).into()).map_err(|e| player.describe(&e))?;
            player.set_value(&format!("{element}.description"), strings.lookup(&map.description).into()).map_err(|e| player.describe(&e))?;
            if let Some(image) = &map.thumbnail { player.set_value(&format!("{element}.image"), image.as_str().into()).map_err(|e| player.describe(&e))?; }
            player.set_value(&format!("{element}.itemID"), entry.item_id.into()).map_err(|e| player.describe(&e))?;
            player.set_value(&format!("{element}.isNew"), entry.is_new.into()).map_err(|e| player.describe(&e))?;
            any_new |= entry.is_new;
        }
        player.set_value("journalData.notes.hasNewMaps", any_new.into()).map_err(|e| player.describe(&e))?;
        // 14051e950 reads the selected live mission and only journal-visible
        // objectives in statuses 2..5. Definitions alone do not imply progress.
        // No live progression producer exists in the current scalar host.
        // Its absence leaves the movie's authored defaults; never synthesize
        // a mission or status snapshot from the selected map.
        if let Some(gameflow) = gameflow {
        player.set_value("journalData.mission.showHistory", gameflow.show_history().into()).map_err(|e| player.describe(&e))?;
        if let Some(mission) = gameflow.mission() {
                player.set_value("journalData.mission.name", strings.lookup(&mission.name).into())
                    .map_err(|e| player.describe(&e))?;
                player.set_value("journalData.mission.description", strings.lookup(&mission.description).into())
                    .map_err(|e| player.describe(&e))?;
                let objectives: Vec<_> = mission.objectives.iter().filter_map(|objective|
                    gameflow.journal_status(&objective.name).filter(|status| *status >= 0 && objective.show_in_journal).map(|status| (objective, status))
                ).collect();
                player.set_value("journalData.objectives.length", (objectives.len() as u32).into())
                    .map_err(|e| player.describe(&e))?;
                for (index, (objective, status)) in objectives.into_iter().enumerate() {
                    let path = format!("journalData.objectives.data[{index}]");
                    for (field, value) in [("label", &objective.name), ("description", &objective.description)] {
                        player.set_value(&format!("{path}.{field}"), strings.lookup(value).into())
                            .map_err(|e| player.describe(&e))?;
                    }
                    player.set_value(&format!("{path}.optional"), objective.optional.into()).map_err(|e| player.describe(&e))?;
                    player.set_value(&format!("{path}.status"), status.into()).map_err(|e| player.describe(&e))?;
                    if let Some(image) = &objective.image {
                        player.set_value(&format!("{path}.image"), image.as_str().into()).map_err(|e| player.describe(&e))?;
                    }
                    // 1404eb3d0 retains singleton unconditional tasks and
                    // combines conditional tasks into their first group.
                    let mut groups: Vec<Vec<&crate::gameplay::gameflow::Task>> = Vec::new();
                    for task in objective.tasks.iter().filter(|task| gameflow.task_visible(task)) {
                        if task.condition_index == -1 {
                            groups.push(vec![task]);
                        } else if let Some(group) = groups.iter_mut().find(|group| group[0].condition_index != -1) {
                            group.push(task);
                        } else { groups.push(vec![task]); }
                    }
                    let count: usize = groups.iter().map(|group| {
                        if group.iter().any(|task| task.condition_index != group[0].condition_index) { 1 } else { group.len() }
                    }).sum();
                    player.set_value(&format!("{path}.tasks.length"), (count as u32).into()).map_err(|e| player.describe(&e))?;
                    let write_task = |player: &mut Player, task_path: &str, task: &crate::gameplay::gameflow::Task, conditional: bool| -> Result<(), String> {
                        for (field, value) in [("label", &task.name), ("description", &task.description)] {
                            player.set_value(&format!("{task_path}.{field}"), strings.lookup(value).into()).map_err(|e| player.describe(&e))?;
                        }
                        player.set_value(&format!("{task_path}.optional"), task.optional.into()).map_err(|e| player.describe(&e))?;
                        player.set_value(&format!("{task_path}.status"), gameflow.journal_status(&task.name).unwrap_or(-1).into()).map_err(|e| player.describe(&e))?;
                        player.set_value(&format!("{task_path}.lethality"), task.lethality.into()).map_err(|e| player.describe(&e))?;
                        player.set_value(&format!("{task_path}.marked"), gameflow.task_marked(&task.name).into()).map_err(|e| player.describe(&e))?;
                        player.set_value(&format!("{task_path}.taskID"), crate::assets::lang::key_id(&task.name).into()).map_err(|e| player.describe(&e))?;
                        if conditional { player.set_value(&format!("{task_path}.conditionIndex"), task.condition_index.into()).map_err(|e| player.describe(&e))?; }
                        if let Some(image) = &task.image { player.set_value(&format!("{task_path}.image"), image.as_str().into()).map_err(|e| player.describe(&e))?; }
                        Ok(())
                    };
                    let mut row = 0;
                    for group in groups {
                        let mixed = group.iter().any(|task| task.condition_index != group[0].condition_index);
                        if mixed {
                            let group_path = format!("{path}.tasks.data[{row}].tasksConditionGroup");
                            player.set_value(&format!("{group_path}.length"), (group.len() as u32).into()).map_err(|e| player.describe(&e))?;
                            for (index, task) in group.into_iter().enumerate() {
                                write_task(&mut player, &format!("{group_path}.data[{index}]"), task, true)?;
                            }
                            row += 1;
                        } else {
                            for task in group {
                                write_task(&mut player, &format!("{path}.tasks.data[{row}]"), task, false)?;
                                row += 1;
                            }
                        }
                    }

                }
        }
        }
        // 14078cc50 supplies a direct-access section, then two translated
        // journal bindings; 14078c620 returns -1 for ordinary entry.
        player
            .call_root_method("_C_Show", &[(-1i32).into(), first.into(), second.into()])
            .map_err(|e| player.describe(&e))?;
        let image_map = super::pause_menu::library_images(&catalog, &player, &resources)?;
        let residency =
            super::pause_menu::MovieResidency::new("journal", &catalog, &resources, &image_map);
        let images = Arc::new(image_map);
        let stage = player.state.borrow().stage_size;
        let mut cache = render::Cache::default();
        cache.set_font_caching(pause_state::font_caching(
            &catalog,
            &path,
            stage,
            (screen.0 as u32, screen.1 as u32),
        ));
        let interval = 1.0 / player.state.borrow().frame_rate;
        super::pause_menu::report_uncaught(&mut player, "Journal");
        Ok(Self {
            player,
            images,
            cache,
            generation,
            serial: 0,
            accumulated: 0.0,
            interval,
            last: None,
            documents,
            maps,
            audiographs,
            purchase_object: None,
            focused: false,
            _residency: residency,
        })
    }

    pub(super) fn item(&self, path: &str) -> Option<&inventory_item::PickableItem> {
        self.documents.iter().find(|entry| entry.path.eq_ignore_ascii_case(path)).map(|entry| entry.declaration.as_ref())
            .or_else(|| self.maps.iter().chain(&self.audiographs).find(|entry| entry.path.eq_ignore_ascii_case(path))
                .map(|entry| entry.declaration.as_ref()))
    }

    /// The native callback writes cost=0 and refreshes runes only on a
    /// successful inventory purchase (140521580 / 14078d1f0).
    pub(super) fn audiograph_playing(&mut self, item: u32, playing: bool) -> Result<(), String> {
        if let Some(index) = self.audiographs.iter().rev().position(|entry| entry.item_id == item) {
            self.player.call_path_method("journalData.notes.audiographsData", "updatePlayingStatusAt", &[(index as i32).into(), playing.into()])
                .map_err(|e| self.player.describe(&e))?;
            self.last = None;
        }
        Ok(())
    }

    pub(super) fn set_history(&mut self, show: bool) -> Result<(), String> {
        self.player.set_value("journalData.mission.showHistory", show.into()).map_err(|e| self.player.describe(&e))?;
        self.last = None;
        Ok(())
    }

    pub(super) fn purchase_result(&mut self, success: bool, runes: u32) -> Result<(), String> {
        if let Some(object) = self.purchase_object.take() {
            if success {
                self.player.vm.set_property(&crate::flash::avm2::value::Value::Object(object),
                    &crate::flash::avm2::name::Name::public("cost"), 0.into()).map_err(|e| self.player.describe(&e))?;
                self.player.set_value("journalData.powersData.runesCount", runes.into()).map_err(|e| self.player.describe(&e))?;
                self.last = None;
            }
        }
        Ok(())
    }

    pub(super) fn report_uncaught(&mut self) {
        super::pause_menu::report_uncaught(&mut self.player, "Journal");
    }

    pub(super) fn interval(&self) -> f64 {
        self.interval
    }

    /// The journal's message-box handler is suspended while another game
    /// logic state presents a document (14078c830 binds this callback).
    pub(super) fn subscreen_closed(&mut self) -> Result<(), String> {
        self.last = None;
        self.player
            .call_path_method(
                "gameService.gameMessageBoxHandler",
                "_C_SubscreenClosed",
                &[],
            )
            .map(|_| ())
            .map_err(|e| self.player.describe(&e))
    }

    pub(super) fn frame(
        &mut self,
        dt: f64,
        force: bool,
    ) -> Result<(Option<FlashFrame>, bool, Vec<u32>, Vec<Action>), String> {
        self.accumulated += dt;
        let tick = self.accumulated >= self.interval;
        let changed = self.last.is_none() || tick;
        if tick {
            let step = self.accumulated.min(self.interval);
            self.accumulated = (self.accumulated - self.interval).max(0.0);
            self.player
                .tick(step)
                .map_err(|e| self.player.describe(&e))?;
        }
        let mut closed = false;
        let mut sounds = Vec::new();
        let mut actions = Vec::new();
        for (name, args) in self.player.take_notifications() {
            use crate::flash::avm2::value::{Value, to_uint32};
            // UIScene.focused creates its cursor before returning the focus
            // notification (035020/034fe0). Route input only while this movie
            // owns focus, as the shared Iggy input stack does.
            if name == "SetFlashFocus" {
                self.player.call_path_method("controllerTypeHandler", "_C_UpdateControllerType",
                    &[pause_state::controller_type().into()])
                    .map_err(|e| self.player.describe(&e))?;
                self.focused = true;
                continue;
            }
            if name == "KillFlashFocus" { self.focused = false; continue; }
            if name == "OnJournalDisabled" {
                closed = true;
                continue;
            }
            let number = match args.first() {
                Some(Value::Number(value)) => Some(*value),
                _ => None,
            };
            if name == "PostSoundEvent" {
                if let Some(value) = number {
                    sounds.push(to_uint32(value));
                }
                continue;
            }
            if name == "OnPowerUpgradeAcquired" {
                if let Some(Value::Object(object)) = args.first() {
                    use crate::flash::avm2::name::Name;
                    let target = Value::Object(object.clone());
                    let root = self.player.vm.get_property(&target, &Name::public("isRootUpgrade")).map_err(|e| self.player.describe(&e))?.to_boolean();
                    let id = self.player.vm.get_property(&target, &Name::public("powerUpgradeID")).map_err(|e| self.player.describe(&e))?.primitive_to_number() as i32;
                    self.purchase_object = Some(object.clone());
                    actions.push(Action::PurchasePower { root, id });
                }
                continue;
            }
            if name == "OnTasksMarked" {
                if let Some(Value::Object(array)) = args.first() {
                    if let crate::flash::avm2::object::Kind::Array(values) = &array.borrow().kind {
                        if let Some(first) = values.first() {
                            let marked = first.primitive_to_number() != 0.0;
                            let ids = values.iter().skip(1).filter_map(|value| match value {
                                Value::Number(number) => Some(to_uint32(*number)), _ => None,
                            }).collect();
                            actions.push(Action::MarkTasks(marked, ids));
                        }
                    }
                }
                continue;
            }
            if name == "OnShowHistory" {
                if let Some(value) = args.first() { actions.push(Action::ShowHistory(value.primitive_to_number() != 0.0)); }
                continue;
            }
            if name == "PlayAudiograph" {
                if let Some(index) = number.filter(|value| *value >= 0.0).map(|value| value as usize) {
                    if let Some(entry) = self.audiographs.iter().rev().nth(index) {
                        actions.push(Action::PlayAudiograph { item: entry.item_id, path: entry.path.clone() });
                    }
                }
                continue;
            }
            if name == "OnAudiographsRead" { actions.push(Action::SeenAudiographs); continue; }
            if name == "OnMapsRead" {
                if let Some(value) = number {
                    actions.push(Action::SeenMap(if value == -1.0 { None } else { Some(to_uint32(value)) }));
                }
                continue;
            }
            if name == "ShowMap" {
                if let Some(entry) = number.and_then(|value| self.maps.iter().find(|entry| entry.item_id == to_uint32(value))) {
                    actions.push(Action::ShowMap(entry.path.clone()));
                } else { self.subscreen_closed()?; }
                continue;
            }
            let kind = match name.as_str() {
                "OnDocumentsRead" | "OnShowDocument" => ReadableKind::Document,
                "OnBooksRead" | "OnShowBook" => ReadableKind::Book,
                _ => { super::pause_menu::report_unknown_callback("Journal", &name); continue; },
            };
            let Some(value) = number else {
                if name == "OnShowDocument" || name == "OnShowBook" {
                    self.subscreen_closed()?;
                }
                continue;
            };
            if name == "OnDocumentsRead" || name == "OnBooksRead" {
                actions.push(Action::Seen(
                    kind,
                    if value == -1.0 {
                        None
                    } else {
                        Some(to_uint32(value))
                    },
                ));
            } else if let Some(entry) = self
                .documents
                .iter()
                .find(|entry| entry.kind == kind && entry.item_id == to_uint32(value))
            {
                actions.push(Action::Show(entry.path.clone()));
            } else {
                self.subscreen_closed()?;
            }
        }
        if changed {
            self.serial += 1;
            self.last = Some(FlashFrame {
                list: Arc::new(render::draw_list_cached(&self.player.vm, &mut self.cache)),
                images: self.images.clone(),
                game_images: Default::default(),
                generation: self.generation,
                serial: self.serial,
            });
        }
        self.report_uncaught();
        Ok((
            if changed || force {
                self.last.clone()
            } else {
                None
            },
            closed,
            sounds,
            actions,
        ))
    }

    pub(super) fn key(&mut self, down: bool, vk: u32) -> Result<(), String> {
        if !self.focused { return Ok(()); }
        self.last = None;
        self.player
            .key_event(down, vk, 0)
            .map_err(|e| self.player.describe(&e))
    }

    pub(super) fn cursor(&mut self, x: f64, y: f64) -> Result<(), String> {
        if !self.focused { return Ok(()); }
        self.last = None;
        self.player
            .set_value("mouseCursor.x", x.into())
            .and_then(|_| self.player.set_value("mouseCursor.y", y.into()))
            .and_then(|_| self.player.mouse_move(x, y))
            .map_err(|e| self.player.describe(&e))
    }

    pub(super) fn button(&mut self, down: bool, button: u8) -> Result<(), String> {
        if !self.focused { return Ok(()); }
        self.last = None;
        let result = if button < 3 {
            self.player.mouse_button(button, down)
        } else {
            Ok(())
        };
        result
            .and_then(|_| {
                self.player
                    .call_path_method(
                        "inputsManager.mouseHandler",
                        "IG_OnMouseEvent",
                        &[
                            (0xe0u32 + button as u32).into(),
                            (if down { 1.0 } else { 0.0 }).into(),
                        ],
                    )
                    .map(|_| ())
            })
            .map_err(|e| self.player.describe(&e))
    }

    pub(super) fn wheel(&mut self, delta: i32) -> Result<(), String> {
        if !self.focused { return Ok(()); }
        self.last = None;
        self.player
            .mouse_wheel(delta)
            .map_err(|e| self.player.describe(&e))
    }
}
