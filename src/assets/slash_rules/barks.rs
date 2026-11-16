//! Typed DEVICE_BARK projection of the shared ruleHandler resource.
//! Declaration693.../696c50, context8959b0 and native selection8958c0.
use super::{Fields, Operator};
use crate::{
    archive::Catalog,
    entities::{
        self, Value,
        event_graph::{atom, child},
    },
};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Debug, PartialEq)]
pub enum Expected {
    Boolean(bool),
    Symbol(Arc<str>),
    AreaTag(u32),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub variable: Arc<str>,
    pub(crate) operator: Operator,
    pub expected: Expected,
    pub trigger: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Sequence,
    Shuffle,
    SequenceOnce,
    ShuffleOnce,
}
impl Policy {
    pub fn once(self) -> bool {
        matches!(self, Self::SequenceOnce | Self::ShuffleOnce)
    }
    pub fn shuffled(self) -> bool {
        matches!(self, Self::Shuffle | Self::ShuffleOnce)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub line: u32,
    pub speech: Option<Arc<str>>,
    pub priority: i32,
    pub frequency: Arc<str>,
    pub game_properties: Option<Arc<str>>,
    pub followup_tag: Option<Arc<str>>,
    pub followup_target: Arc<str>,
    pub followup_count: i32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub conditions: Vec<Condition>,
    pub outcomes: Vec<Outcome>,
    pub policy: Policy,
    pub fallback: bool,
}
#[derive(Debug, PartialEq)]
pub struct Definition {
    pub rules: Vec<Rule>,
}
/// This projection retains the canonical ruleHandler generation itself.
/// A child-only Arc would allow its parent weak header to acquire a duplicate.
#[derive(Clone, Debug, PartialEq)]
pub struct Lease(Arc<super::Definition>);
impl std::ops::Deref for Lease {
    type Target = Definition;
    fn deref(&self) -> &Definition {
        self.0
            .barks
            .as_ref()
            .expect("checked DEVICE_BARK resource projection")
    }
}
impl Lease {
    pub fn acquire(catalog: &Catalog, name: &str) -> Result<Self, String> {
        let definition = super::Definition::acquire_shared(catalog, name)?;
        if definition.barks.is_none() {
            return Err(format!("{name}: ruleHandler is not DEVICE_BARK"));
        }
        Ok(Self(definition))
    }
}
fn priority(value: &str) -> Result<i32, String> {
    //1419fc170, arkFaceControlPriority_t. Numeric values are authored enum
    //values, not a guessed LOW/MEDIUM/HIGH ordering.
    const VALUES: [&str; 11] = [
        "PRIORITY_INVALID",
        "PRIORITY_IDLE",
        "PRIORITY_LOW",
        "PRIORITY_DEFAULT",
        "PRIORITY_MEDIUM",
        "PRIORITY_NPCTALK",
        "PRIORITY_HIGH",
        "PRIORITY_VERYHIGH",
        "PRIORITY_MIDNIGHT",
        "PRIORITY_DAMAGED",
        "PRIORITY_FATALITY",
    ];
    value
        .parse()
        .ok()
        .or_else(|| {
            VALUES
                .iter()
                .position(|name| *name == value)
                .map(|index| index as i32)
        })
        .ok_or_else(|| format!("unknown bark priority {value}"))
}
fn list<'a>(fields: &'a Fields, key: &str) -> Result<Vec<&'a Fields>, String> {
    let fields = child(fields, key);
    let count = atom(fields, "num")
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| format!("invalid bark {key} count"))?;
    (0..count)
        .map(
            |index| match entities::field(fields, &format!("item[{index}]")) {
                Some(Value::Scope(fields)) => Ok(fields.as_slice()),
                _ => Err(format!("bark {key}.item[{index}] is not a scope")),
            },
        )
        .collect()
}
impl Definition {
    pub(super) fn read(fields: &Fields) -> Result<Self, String> {
        let mut interned = HashMap::<String, Arc<str>>::new();
        let mut intern = |name: &str| {
            interned
                .entry(name.to_owned())
                .or_insert_with(|| Arc::from(name))
                .clone()
        };
        let mut rules = Vec::new();
        for fields in list(fields, "m_rules")? {
            let policy = match atom(fields, "m_executionPolicy").unwrap_or("RULE_EXEC_NONE") {
                "RULE_EXEC_NONE" | "0" => continue,
                "RULE_EXEC_SEQUENCE" | "1" => Policy::Sequence,
                "RULE_EXEC_SHUFFLE" | "2" => Policy::Shuffle,
                "RULE_EXEC_SEQUENCE_ONCE" | "3" => Policy::SequenceOnce,
                "RULE_EXEC_SHUFFLE_ONCE" | "4" => Policy::ShuffleOnce,
                value => return Err(format!("unknown bark execution policy {value}")),
            };
            let mut conditions = Vec::new();
            for fields in list(fields, "m_conditions")? {
                let class = atom(fields, "m_class").ok_or("bark condition class absent")?;
                let fields = child(fields, "m_object");
                let variable = atom(fields, "m_variable").ok_or("bark variable absent")?;
                let expected = match class {
                    "arkRuleCondition_bool" => {
                        Expected::Boolean(match atom(fields, "m_value").unwrap_or("true") {
                            "true" | "1" => true,
                            "false" | "0" => false,
                            value => return Err(format!("invalid bark boolean {value}")),
                        })
                    }
                    "arkRuleCondition_arkAreaTag" => {
                        Expected::AreaTag(match atom(fields, "m_value") {
                            Some(value) => {
                                crate::assets::entities::components::decode_identifier(value)?
                            }
                            None => u32::MAX, //782250 actual arkAreaTag constructor.
                        })
                    }
                    "arkRuleCondition_idAtomicString"
                    | "arkRuleCondition_arkAIBarkTrigger_t"
                    | "arkRuleCondition_arkMoralityLevel_t"
                    | "arkRuleCondition_arkGender_t"
                    | "arkRuleCondition_arkChaosLevel_t" => {
                        Expected::Symbol(intern(atom(fields, "m_value").unwrap_or("")))
                    }
                    value => {
                        return Err(format!(
                            "unsupported actual bark condition {value} for {variable}"
                        ));
                    }
                };
                conditions.push(Condition {
                    variable: intern(variable),
                    operator: Operator::read(fields)?,
                    expected,
                    trigger: class == "arkRuleCondition_arkAIBarkTrigger_t",
                });
            }
            let mut outcomes = Vec::new();
            for fields in list(fields, "m_actions")? {
                if atom(fields, "m_class") != Some("arkRuleAction_BarkOutcome") {
                    return Err(format!(
                        "unsupported actual bark action {:?}",
                        atom(fields, "m_class")
                    ));
                }
                let fields = child(fields, "m_object");
                let mut reference = |key| {
                    atom(fields, key)
                        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
                        .map(&mut intern)
                };
                let speech = reference("m_lines");
                let game_properties = reference("m_gameProperties");
                drop(reference);
                let followup = child(fields, "m_followUpProperties");
                outcomes.push(Outcome {
                    line: atom(fields, "m_lineID")
                        .unwrap_or("0")
                        .parse()
                        .map_err(|_| "invalid bark line ID")?,
                    speech,
                    priority: priority(
                        atom(fields, "m_barkPriority").unwrap_or("PRIORITY_DEFAULT"),
                    )?,
                    frequency: intern(atom(fields, "m_frequency").unwrap_or("BARK_ALMOST_NEVER")),
                    game_properties,
                    followup_tag: atom(followup, "m_followupTag")
                        .filter(|value| !value.is_empty())
                        .map(&mut intern),
                    followup_target: intern(
                        atom(followup, "m_followupTarget")
                            .unwrap_or("ARK_BARK_FOLLOWUP_NEAREST_ALLY"),
                    ),
                    followup_count: atom(followup, "m_followupTargetCount")
                        .unwrap_or("1")
                        .parse()
                        .map_err(|_| "invalid bark followup count")?,
                });
            }
            if outcomes.is_empty() {
                continue;
            }
            let fallback = match atom(fields, "m_fallback").unwrap_or("false") {
                "true" | "1" => true,
                "false" | "0" => false,
                value => return Err(format!("invalid bark fallback {value}")),
            };
            rules.push(Rule {
                conditions,
                outcomes,
                policy,
                fallback,
            });
        }
        //894650 sorts bark handlers by first outcome priority, then trigger,
        //then condition count. Every currently admitted DEVICE_BARK trigger
        //uses actual AMBIENT; reject mixed values until its enum owner exists.
        let mut trigger: Option<Arc<str>> = None;
        for rule in &rules {
            for condition in &rule.conditions {
                if condition.trigger {
                    if let Expected::Symbol(value) = &condition.expected {
                        if trigger.as_ref().is_some_and(|first| first != value) {
                            return Err(
                                "mixed bark triggers require the actual trigger enum ordering"
                                    .into(),
                            );
                        }
                        trigger = Some(value.clone());
                    }
                }
            }
        }
        rules.sort_by(|left, right| {
            right.outcomes[0]
                .priority
                .cmp(&left.outcomes[0].priority)
                .then_with(|| right.conditions.len().cmp(&left.conditions.len()))
        });
        Ok(Self { rules })
    }
    pub(super) fn storage_bytes(&self) -> usize {
        self.rules.capacity() * std::mem::size_of::<Rule>()
            + self
                .rules
                .iter()
                .map(|rule| {
                    rule.conditions.capacity() * std::mem::size_of::<Condition>()
                        + rule.outcomes.capacity() * std::mem::size_of::<Outcome>()
                })
                .sum::<usize>()
    }
}
