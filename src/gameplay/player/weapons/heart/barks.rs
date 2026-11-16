//! Mutable DEVICE_BARK rule instances:8958c0/896510 and action lists
//!8921d0/892280. Definitions stay shared; only cursors/permutations vary.
use crate::assets::slash_rules::barks::{Expected, Lease, Outcome};
use crate::{
    archive::Catalog,
    entities::{
        Value,
        scene_motion::{atom, child},
    },
};
use std::{collections::HashMap, sync::Arc};

pub struct Definition {
    pub handlers: Vec<(Arc<str>, Vec<Lease>)>,
    pub default_tags: Vec<Arc<str>>,
    pub voice: Arc<str>,
    pub voice_resource: Arc<crate::assets::speech::Definition>,
    pub sets: HashMap<Arc<str>, Arc<crate::assets::speech::Definition>>,
    pub frequency_ranges: [[f32; 2]; 5],
    pub delay: [f32; 2],
}
fn edit_values(fields: &[(String, Value)]) -> Result<Vec<&Value>, String> {
    let mut values = Vec::new();
    for (key, value) in fields {
        if key == "item_begin" || key == "num" {
            continue;
        }
        if !key.starts_with("item_add[") && !key.starts_with("item[") {
            return Err(format!("unresolved Bark edit-list field {key}"));
        }
        values.push(value);
    }
    Ok(values)
}
impl Definition {
    pub fn read(
        catalog: &Catalog,
        fields: &[(String, Value)],
        speech: &[(String, Value)],
    ) -> Result<Self, String> {
        let voice = atom(speech, "m_voice")
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
            .ok_or("Heart Barks requires its actual speech component voice")?;
        let mut fields = fields.to_vec();
        crate::assets::entities::components::normalize_edit_fields(&mut fields)?;
        let defaults = child(&fields, "m_defaultRuleHandlerTagList");
        let count = atom(defaults, "num")
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| "invalid Bark default tag count")?;
        let mut default_tags = Vec::with_capacity(count);
        for index in 0..count {
            default_tags.push(Arc::from(
                atom(defaults, &format!("item[{index}]")).ok_or("Bark default tag absent")?,
            ));
        }
        let mut handlers = Vec::new();
        let mut sets = HashMap::new();
        for value in edit_values(child(&fields, "m_barkRulehandlerLists"))? {
            let Value::Scope(fields) = value else {
                return Err("Bark handler record is not a scope".into());
            };
            let tag = atom(fields, "m_tag").ok_or("Bark handler tag absent")?;
            let list = child(child(fields, "m_rulehandlerList"), "m_ruleHandlerList");
            let mut definitions = Vec::new();
            for value in edit_values(list)? {
                let Value::Atom(name) = value else {
                    return Err("Bark handler declaration reference is not scalar".into());
                };
                let name = name.trim_matches('"');
                if name.is_empty() || name.eq_ignore_ascii_case("NULL") {
                    continue;
                }
                let definition = Lease::acquire(catalog, name)?;
                for outcome in definition.rules.iter().flat_map(|rule| &rule.outcomes) {
                    if let Some(path) = &outcome.speech {
                        if !sets.contains_key(path) {
                            sets.insert(
                                path.clone(),
                                crate::assets::speech::acquire_barks(catalog, path)?,
                            );
                        }
                    }
                }
                definitions.push(definition);
            }
            handlers.push((Arc::from(tag), definitions));
        }
        //5d9690 constructor's five frequency ranges; durations are seconds.
        //No independent cooldown table is retained on a Heart instance.
        let mut frequency_ranges = [
            [0., 0.],
            [60., 90.],
            [240., 360.],
            [1800., 2400.],
            [10000., 20000.],
        ];
        let frequency = child(&fields, "m_barkFrequencyList");
        for (key, value) in frequency {
            let Some(index) = key
                .strip_prefix("enumItem[")
                .and_then(|key| key.strip_suffix(']'))
            else {
                continue;
            };
            let index = match index {
                "BARK_ALWAYS" | "0" => 0,
                "BARK_OFTEN" | "1" => 1,
                "BARK_SOMETIMES" | "2" => 2,
                "BARK_RARELY" | "3" => 3,
                "BARK_ALMOST_NEVER" | "4" => 4,
                _ => return Err(format!("unknown Bark frequency {index}")),
            };
            let Value::Scope(value) = value else {
                return Err("Bark frequency range is not a scope".into());
            };
            frequency_ranges[index] = [
                crate::gameplay::damage::number(value, "m_min", frequency_ranges[index][0])?,
                crate::gameplay::damage::number(value, "m_max", frequency_ranges[index][1])?,
            ];
        }
        let delay = child(&fields, "m_durationBeforeBark");
        Ok(Self {
            handlers,
            default_tags,
            voice: Arc::from(voice),
            voice_resource: crate::assets::speech::acquire_voice(catalog, voice)?,
            sets,
            frequency_ranges,
            delay: [
                crate::gameplay::damage::number(delay, "m_min", 0.2)?,
                crate::gameplay::damage::number(delay, "m_max", 0.5)?,
            ],
        })
    }
}

pub trait Context {
    fn boolean(&mut self, variable: &str) -> Result<bool, String>;
    fn symbol(&mut self, variable: &str) -> Result<&str, String>;
    fn area_tag(&mut self, variable: &str) -> Result<u32, String>;
    ///Source arkAIBarkRuleFilter rejects an outcome using actual speech
    ///priority and shared BB history. false means rejected, not missing data.
    fn accepts(&mut self, outcome: &Outcome) -> Result<bool, String>;
}
#[derive(Clone, Debug, PartialEq)]
struct RuleState {
    cursor: usize,
    exhausted: bool,
    order: Vec<usize>,
    shuffle_end: Option<usize>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Runtime {
    definition: Lease,
    states: Vec<RuleState>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    definition: Lease,
    rule: usize,
    outcome: usize,
}
impl Selection {
    pub fn outcome(&self) -> &Outcome {
        &self.definition.rules[self.rule].outcomes[self.outcome]
    }
}
fn random_index(random: &mut u32, count: usize) -> usize {
    *random = random.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
    ((*random >> 10) & 0x7fff) as usize % count
}
///8923c0 shuffles the first cycle;3e8220 prevents its last entry immediately
///repeating at the start of a subsequent cycle. The caller supplies the real
///game random state, never a Heart-local seed.
fn shuffle(order: &mut [usize], random: &mut u32, first: bool) {
    if first {
        for upper in (1..order.len()).rev() {
            let selected = random_index(random, upper + 1);
            order.swap(upper, selected);
        }
    } else if order.len() > 2 {
        for upper in (1..order.len() - 1).rev() {
            let selected = random_index(random, upper + 1);
            order.swap(upper, selected);
        }
        let selected = random_index(random, order.len() - 1) + 1;
        order.swap(order.len() - 1, selected);
    }
}
impl Runtime {
    pub fn new(definition: Lease) -> Self {
        let states = definition
            .rules
            .iter()
            .map(|rule| RuleState {
                cursor: 0,
                exhausted: false,
                order: (0..rule.outcomes.len()).collect(),
                shuffle_end: None,
            })
            .collect();
        Self { definition, states }
    }
    pub fn select(
        &mut self,
        context: &mut impl Context,
        random: &mut u32,
    ) -> Result<Option<Selection>, String> {
        for index in 0..self.definition.rules.len() {
            let rule = &self.definition.rules[index];
            let state = &mut self.states[index];
            if state.exhausted {
                continue;
            }
            let mut matches = true;
            for condition in &rule.conditions {
                let accepted = match &condition.expected {
                    Expected::AreaTag(expected) => condition
                        .operator
                        .compare(context.area_tag(&condition.variable)?, *expected),
                    Expected::Boolean(expected) => condition
                        .operator
                        .compare(context.boolean(&condition.variable)?, *expected),
                    Expected::Symbol(expected) => condition
                        .operator
                        .compare(context.symbol(&condition.variable)?, expected.as_ref()),
                };
                if !accepted {
                    matches = false;
                    break;
                }
            }
            if !matches {
                continue;
            }
            let count = rule.outcomes.len();
            let shuffled = rule.policy.shuffled();
            if shuffled && state.shuffle_end.is_none() {
                shuffle(&mut state.order, random, true);
                state.shuffle_end = Some(count);
            }
            let start = state.cursor;
            let attempts = if shuffled {
                2 * count - start % count
            } else {
                count
            };
            let mut selected = None;
            for offset in 0..attempts {
                let cursor = start + offset;
                if shuffled && state.shuffle_end.is_some_and(|end| end <= cursor) {
                    shuffle(&mut state.order, random, false);
                    state.shuffle_end = Some(cursor + count);
                }
                let outcome = state.order[cursor % count];
                if context.accepts(&rule.outcomes[outcome])? {
                    selected = Some((cursor, outcome));
                    break;
                }
            }
            //896510 falls back only when conditions passed but every outcome
            //was rejected.8963f0 then uses the unfiltered current list entry.
            if selected.is_none() && rule.fallback {
                selected = Some((state.cursor, state.order[state.cursor % count]));
            }
            if let Some((cursor, outcome)) = selected {
                state.cursor = cursor + 1;
                state.exhausted = rule.policy.once() && state.cursor >= count;
                return Ok(Some(Selection {
                    definition: self.definition.clone(),
                    rule: index,
                    outcome,
                }));
            }
        }
        Ok(None)
    }
}
