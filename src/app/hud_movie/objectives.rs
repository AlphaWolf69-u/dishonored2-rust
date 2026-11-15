//! 14066eff0 coalesces queued task updates by objective. 140670a00 publishes
//! the authored Flash data; the movie's completion callback advances the queue.
use super::*;
use crate::gameplay::gameflow::{ObjectiveUpdate, TaskUpdate};
use std::collections::VecDeque;

pub(super) struct State {
    pending: VecDeque<ObjectiveUpdate>,
    showing: bool,
    notified: Vec<u32>,
    strings: crate::assets::lang::StringTable,
    first_delay: f64,
    task_delay: f64,
    portrait_delay: f64,
}
impl State {
    pub(super) fn load(
        catalog: &Catalog,
        language: &str,
    ) -> Result<(Self, super::markers::State), String> {
        use crate::gameplay::options::decl::atom;
        let fields = crate::assets::entities::player::resolved_decl(
            "ui_hudelementobjectives",
            "ui/hud_objectives.uihudelementobjectives",
            |path| {
                catalog
                    .read(path)
                    .map_err(|error| error.to_string())
                    .map(|bytes| bytes.map(crate::entities::owned_declaration_text))
            },
        )?;
        let number = |name| {
            atom(&fields, name)
                .ok_or_else(|| format!("objective HUD declaration lacks {name}"))?
                .parse::<f64>()
                .map_err(|_| format!("invalid objective HUD {name}"))
        };
        let markers = super::markers::State::from_fields(&fields)?;
        Ok((
            Self {
                pending: VecDeque::new(),
                showing: false,
                notified: Vec::new(),
                strings: crate::assets::lang::StringTable::load(catalog, language)?,
                first_delay: number("m_firstTaskDelay")?,
                task_delay: number("m_taskDelay")?,
                // 66b0f0 initializes the declaration's portrait delay (+0x88) to4s.
                portrait_delay: atom(&fields, "m_portraitDelay")
                    .map(str::parse::<f64>)
                    .transpose()
                    .map_err(|_| "invalid portrait delay")?
                    .unwrap_or(4.),
            },
            markers,
        ))
    }
    pub(super) fn enqueue(&mut self, updates: Vec<ObjectiveUpdate>) {
        for mut update in updates {
            // 671b90 removes a queued activation when that task completes,
            // fails or cancels before the movie reaches the older row.
            for task in update.tasks.iter().filter(|task| task.status != 0) {
                for previous in &mut self.pending {
                    previous
                        .tasks
                        .retain(|old| old.id != task.id || old.status != 0);
                }
            }
            self.pending
                .retain(|previous| previous.portrait.is_some() || !previous.tasks.is_empty());
            if update.portrait.is_some() {
                self.pending.push_back(update);
                continue;
            }
            if let Some(previous) = self
                .pending
                .iter_mut()
                .find(|previous| previous.portrait.is_none() && previous.name == update.name)
            {
                previous.optional = update.optional;
                previous.status = update.status;
                for task in update.tasks.drain(..) {
                    previous.tasks.retain(|old| old.id != task.id);
                    previous.tasks.push(task);
                }
                previous.tasks.sort_by_key(|task| task.condition);
            } else {
                self.pending.push_back(update);
            }
        }
    }
}
fn write_task(
    player: &mut Player,
    strings: &crate::assets::lang::StringTable,
    path: &str,
    task: &TaskUpdate,
    conditional: bool,
) -> Result<(), String> {
    player
        .set_value(&format!("{path}.label"), strings.lookup(&task.name).into())
        .map_err(|error| player.describe(&error))?;
    player
        .set_value(&format!("{path}.optional"), task.optional.into())
        .map_err(|error| player.describe(&error))?;
    player
        .set_value(&format!("{path}.status"), task.status.into())
        .map_err(|error| player.describe(&error))?;
    player
        .set_value(&format!("{path}.lethality"), task.lethality.into())
        .map_err(|error| player.describe(&error))?;
    player
        .set_value(&format!("{path}.taskID"), task.id.into())
        .map_err(|error| player.describe(&error))?;
    if conditional {
        player
            .set_value(&format!("{path}.conditionIndex"), task.condition.into())
            .map_err(|error| player.describe(&error))?;
    }
    Ok(())
}
impl HudMovie {
    pub(crate) fn queue_objectives(&mut self, updates: Vec<ObjectiveUpdate>) {
        self.objectives.enqueue(updates);
        self.last = None;
    }
    pub(crate) fn take_notified_tasks(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.objectives.notified)
    }
    pub(super) fn advance_objectives(&mut self) -> Result<(), String> {
        if self.objectives.showing {
            return Ok(());
        }
        let Some(update) = self.objectives.pending.pop_front() else {
            return Ok(());
        };
        if let Some(portrait) = update.portrait {
            let enabled = crate::platform::console::shared()
                .read()
                .cvars
                .int("ark_ui_show_target_notifications")
                .unwrap_or(1)
                != 0;
            if enabled {
                let state = match portrait.state.as_str() {
                    "GAMEFLOW_UI_PORTRAIT_STATE_ASSASSINATED" => 0,
                    "GAMEFLOW_UI_PORTRAIT_STATE_NEUTRALIZED" => 1,
                    "GAMEFLOW_UI_PORTRAIT_STATE_RESCUED" => 2,
                    other => return Err(format!("unknown portrait state {other}")),
                };
                let kind = match portrait.kind.as_str() {
                    "GAMEFLOW_UI_PORTRAIT_TYPE_TARGET" => 0,
                    "GAMEFLOW_UI_PORTRAIT_TYPE_ALLY" => 1,
                    "GAMEFLOW_UI_PORTRAIT_TYPE_GANG_LEADER" => 2,
                    "GAMEFLOW_UI_PORTRAIT_TYPE_KEY_FIGURE" => 3,
                    other => return Err(format!("unknown portrait kind {other}")),
                };
                let label = self.objectives.strings.text(portrait.name);
                let image = portrait.image.as_deref().unwrap_or(label);
                self.player
                    .call_path_method(
                        "_targetNotification",
                        "_C_ShowNotification",
                        &[
                            label.into(),
                            image.into(),
                            kind.into(),
                            state.into(),
                            portrait.female.into(),
                            self.objectives.portrait_delay.into(),
                        ],
                    )
                    .map_err(|error| self.player.describe(&error))?;
                self.objectives.showing = true;
                self.hud_activity = true;
                self.last = None;
            }
            return Ok(());
        }
        let visible = {
            let console = crate::platform::console::shared();
            let console = console.read();
            console
                .cvars
                .find("ark_ui_show_objective_notifications")
                .is_some_and(|index| console.cvars.list[index].int != 0)
        };
        if !visible {
            return Ok(());
        }
        let player = &mut self.player;
        player
            .set_value("_objectiveUpdate.objectivesData.length", 1u32.into())
            .map_err(|error| player.describe(&error))?;
        let path = "_objectiveUpdate.objectivesData.data[0]";
        player
            .set_value(
                &format!("{path}.label"),
                self.objectives.strings.lookup(&update.name).into(),
            )
            .map_err(|error| player.describe(&error))?;
        player
            .set_value(&format!("{path}.optional"), update.optional.into())
            .map_err(|error| player.describe(&error))?;
        player
            .set_value(&format!("{path}.status"), update.status.into())
            .map_err(|error| player.describe(&error))?;
        let mut groups: Vec<Vec<&TaskUpdate>> = Vec::new();
        for task in &update.tasks {
            if task.condition == -1 {
                groups.push(vec![task]);
            } else if let Some(group) = groups.iter_mut().find(|group| group[0].condition != -1) {
                group.push(task);
            } else {
                groups.push(vec![task]);
            }
        }
        let count: usize = groups
            .iter()
            .map(|group| {
                if group
                    .iter()
                    .any(|task| task.condition != group[0].condition)
                {
                    1
                } else {
                    group.len()
                }
            })
            .sum();
        player
            .set_value(&format!("{path}.tasks.length"), (count as u32).into())
            .map_err(|error| player.describe(&error))?;
        let mut row = 0;
        for group in groups {
            if group
                .iter()
                .any(|task| task.condition != group[0].condition)
            {
                let base = format!("{path}.tasks.data[{row}].tasksConditionGroup");
                player
                    .set_value(&format!("{base}.length"), (group.len() as u32).into())
                    .map_err(|error| player.describe(&error))?;
                for (index, task) in group.iter().enumerate() {
                    write_task(
                        player,
                        &self.objectives.strings,
                        &format!("{base}.data[{index}]"),
                        task,
                        true,
                    )?;
                }
                row += 1;
            } else {
                for task in group {
                    write_task(
                        player,
                        &self.objectives.strings,
                        &format!("{path}.tasks.data[{row}]"),
                        task,
                        false,
                    )?;
                    row += 1;
                }
            }
        }
        player
            .set_value(
                "_objectiveUpdate.firstTaskDelay",
                self.objectives.first_delay.into(),
            )
            .map_err(|error| player.describe(&error))?;
        player
            .set_value(
                "_objectiveUpdate.taskDuration",
                self.objectives.task_delay.into(),
            )
            .map_err(|error| player.describe(&error))?;
        player
            .call_path_method(
                "_objectiveUpdate",
                "_C_Show",
                &[crate::flash::avm2::value::Value::Undefined],
            )
            .map_err(|error| player.describe(&error))?;
        self.objectives.showing = true;
        self.hud_activity = true;
        self.last = None;
        Ok(())
    }
    pub(super) fn objective_notifications(&mut self) -> Vec<u32> {
        use crate::flash::avm2::{
            object::Kind,
            value::{Value, to_uint32},
        };
        let mut sounds = Vec::new();
        for (name, args) in self.player.take_notifications() {
            match name.as_str() {
                "OnObjectiveUpdateDisabled" | "OnTargetNotificationDisabled" => {
                    self.objectives.showing = false
                }
                "OnTaskNotified" => {
                    if let Some(Value::Object(array)) = args.first() {
                        let array = array.borrow();
                        let values = match &array.kind {
                            Kind::Array(values) => values.as_slice(),
                            Kind::Vector(values) => values.items.as_slice(),
                            _ => &[],
                        };
                        self.objectives
                            .notified
                            .extend(values.iter().filter_map(|value| {
                                if let Value::Number(id) = value {
                                    Some(to_uint32(*id))
                                } else {
                                    None
                                }
                            }));
                    }
                }
                "PostSoundEvent" => {
                    if let Some(Value::Number(id)) = args.first() {
                        sounds.push(to_uint32(*id));
                    }
                }
                _ => {}
            }
        }
        sounds
    }
}
