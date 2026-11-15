//! Player-slash `ruleHandler` declarations. The native declaration owner
//! registers these at 1400607f0; 1408947f0/1408952a0 instantiate their rules,
//! and 140895660 retires instance state separately from declaration leases.
//! Generic source trees are projected once and released after acquisition.
use crate::{archive::Catalog, entities::{self, event_graph::{atom, child}, Value}};
use std::sync::Arc;

type Fields = [(String, Value)];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Operator { Equal, Different, GreaterEqual, Greater, LowerEqual, Lower }
impl Operator {
    fn read(fields: &Fields) -> Result<Self, String> {
        Ok(match atom(fields, "m_operator").unwrap_or("ARK_OP_EQUAL") {
            "ARK_OP_EQUAL" | "0" => Self::Equal,
            "ARK_OP_DIFFERENT" | "1" => Self::Different,
            "ARK_OP_GREATER_OR_EQUAL_THAN" | "2" => Self::GreaterEqual,
            "ARK_OP_GREATER_THAN" | "3" => Self::Greater,
            "ARK_OP_LOWER_OR_EQUAL_THAN" | "4" => Self::LowerEqual,
            "ARK_OP_LOWER_THAN" | "5" => Self::Lower,
            value => return Err(format!("unknown rule operator {value}")),
        })
    }
    pub(crate) fn compare<T: PartialEq + PartialOrd>(self, actual: T, expected: T) -> bool {
        match self {
            Self::Equal => actual == expected, Self::Different => actual != expected,
            Self::GreaterEqual => actual >= expected, Self::Greater => actual > expected,
            Self::LowerEqual => actual <= expected, Self::Lower => actual < expected,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Boolean {
    GenericFatality, FocusBlinkAttack, Focused, ThrowHitAvailable, BlinkAttack,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Condition {
    Boolean(Boolean, Operator, bool),
    LastSlash(Operator, i32),
    Posture(Operator, usize),
    TimeSinceLastSlash(Operator, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub midnight: Option<String>,
    /// arkPlayerRecoilType_t order: None, hard surface, NPC parry left/right.
    pub recoil_midnights: [Option<String>; 4],
    pub cancel_midnight: Option<String>,
    pub attack_hit_midnight: Option<String>,
    /// arkSlashType_t: NONE=0, RIGHT=1, LEFT=2, MAX=3 (141a18720).
    pub slash_type: i32,
    pub chained_start_seconds: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rule {
    pub conditions: Vec<Condition>,
    pub outcomes: Vec<Outcome>,
    pub once: bool,
}

#[derive(Debug, PartialEq)]
pub struct Definition {
    pub(crate) rules: Vec<Rule>,
}

fn path(fields: &Fields, key: &str) -> Option<String> {
    atom(fields, key).filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
        .map(str::to_owned)
}
fn number(fields: &Fields, key: &str, fallback: f32) -> Result<f32, String> {
    atom(fields, key).map_or(Ok(fallback), |text| text.trim_end_matches('f').parse::<f32>().ok()
        .filter(|value| value.is_finite()).ok_or_else(|| format!("invalid rule {key}={text}")))
}
/// Decl arrays are indexed by their serialized index, not field insertion order.
fn list<'a>(fields: &'a Fields, name: &str) -> Result<Vec<&'a Fields>, String> {
    let list = child(fields, name);
    let count = atom(list, "num").unwrap_or("0").parse::<usize>()
        .map_err(|_| format!("invalid {name} rule list length"))?;
    (0..count).map(|index| {
        match entities::field(list, &format!("item[{index}]")) {
            Some(Value::Scope(fields)) => Ok(fields.as_slice()),
            _ => Err(format!("{name} has no rule item[{index}]")),
        }
    }).collect()
}
fn slash_type(value: &str) -> Result<i32, String> {
    match value {
        "SLASH_NONE" | "0" => Ok(0), "SLASH_RIGHT" | "1" => Ok(1),
        "SLASH_LEFT" | "2" => Ok(2), "SLASH_MAX" | "3" => Ok(3),
        value => Err(format!("unknown arkSlashType_t {value}")),
    }
}
impl Condition {
    fn read(fields: &Fields) -> Result<Self, String> {
        let class = atom(fields, "m_class").ok_or("rule condition has no m_class")?;
        let fields = child(fields, "m_object");
        let variable = atom(fields, "m_variable").ok_or("rule condition has no variable")?;
        let operator = Operator::read(fields)?;
        Ok(match (class, variable) {
            ("arkRuleCondition_bool", variable) => {
                let variable = match variable {
                    "slash.isGenericFatality" => Boolean::GenericFatality,
                    "slash.isFocusBlinkAttack" => Boolean::FocusBlinkAttack,
                    "slash.isFocused" => Boolean::Focused,
                    "slash.isThrowHitAvailable" => Boolean::ThrowHitAvailable,
                    "slash.isBlinkAttack" => Boolean::BlinkAttack,
                    other => return Err(format!("unsupported slash boolean provider {other}")),
                };
                let expected = match atom(fields, "m_value").unwrap_or("false") {
                    "false" | "0" => false, "true" | "1" => true,
                    other => return Err(format!("invalid slash boolean comparison {other}")),
                };
                Self::Boolean(variable, operator, expected)
            }
            ("arkRuleCondition_arkSlashType_t", "slash.lastSlashType") =>
                Self::LastSlash(operator, slash_type(atom(fields, "m_value").unwrap_or("SLASH_NONE"))?),
            ("arkRuleCondition_arkPlayerPosture_t", "slash.currentPosture") => {
                let name = atom(fields, "m_value").unwrap_or("ARK_PLAYER_POSTURE_STANDING");
                let posture = crate::gameplay::player::shape::posture(name).or_else(|| name.parse().ok())
                    .filter(|value| *value < crate::gameplay::player::shape::POSTURE_NAMES.len())
                    .ok_or_else(|| format!("unknown slash posture {name}"))?;
                Self::Posture(operator, posture)
            }
            ("arkRuleCondition_float", "slash.timeSinceLastSlash") =>
                Self::TimeSinceLastSlash(operator, number(fields, "m_value", 0.0)?),
            (class, variable) => return Err(format!("unsupported slash condition {class} / {variable}")),
        })
    }
}
impl Outcome {
    fn read(fields: &Fields) -> Result<Self, String> {
        if atom(fields, "m_class") != Some("arkRuleAction_SlashOutcome") {
            return Err(format!("unsupported slash rule action {:?}", atom(fields, "m_class")));
        }
        let fields = child(fields, "m_object");
        let recoil = child(fields, "m_recoilMidnights");
        // 140892fd0 defaults all scene pointers null, SlashType=MAX and .1s.
        Ok(Self {
            midnight: path(fields, "m_midnight"),
            recoil_midnights: ["PLAYER_RECOIL_NONE", "PLAYER_RECOIL_HARD_SURFACE", "PLAYER_RECOIL_NPC_PARRY_LEFT", "PLAYER_RECOIL_NPC_PARRY_RIGHT"]
                .map(|kind| path(recoil, &format!("enumItem[{kind}]"))),
            cancel_midnight: path(fields, "m_cancelMidnight"),
            attack_hit_midnight: path(fields, "m_attackHitMidnight"),
            slash_type: slash_type(atom(fields, "m_slashType").unwrap_or("SLASH_MAX"))?,
            chained_start_seconds: number(child(fields, "m_chainedSlashStartTime"), "m_seconds", 0.1)?,
        })
    }
}
impl Definition {
    pub fn read(fields: &Fields) -> Result<Self, String> {
        if !matches!(atom(fields, "m_type"), Some("RULE_TYPE_PLAYER_SLASH" | "10")) {
            return Err(format!("rule-handler outcome owner is not player Slash: {:?}", atom(fields, "m_type")));
        }
        let mut rules = Vec::new();
        for fields in list(fields, "m_rules")? {
            // 140893e30 removes execution-NONE and empty action lists.
            let once = match atom(fields, "m_executionPolicy").unwrap_or("RULE_EXEC_NONE") {
                "RULE_EXEC_NONE" | "0" => continue,
                "RULE_EXEC_SEQUENCE" | "1" => false,
                "RULE_EXEC_SEQUENCE_ONCE" | "3" => true,
                policy => return Err(format!("slash rule execution policy requires its shuffle owner: {policy}")),
            };
            let actions = list(fields, "m_actions")?;
            if actions.is_empty() { continue; }
            rules.push(Rule {
                conditions: list(fields, "m_conditions")?.into_iter().map(Condition::read).collect::<Result<_, _>>()?,
                outcomes: actions.into_iter().map(Outcome::read).collect::<Result<_, _>>()?,
                once,
            });
        }
        // 140696cc0 only sorts bark families 6..8. PLAYER_SLASH=10
        // retains authored rule order, and 140895820 takes the first match.
        Ok(Self { rules })
    }
    pub fn acquire(catalog: &Catalog, name: &str) -> Result<Arc<Self>, String> {
        catalog.rule_handlers.get_or_init(Default::default).lock().map_err(|_| "rule-handler registry is poisoned")?
            .acquire_with_sharing(&resource_path(name), 0, !catalog.has_overlays(), |_| {
                let _tag = crate::platform::allocation_tags::tag("Rule handler declarations");
                let fields = entities::player::resolved_decl("rulehandler", name,
                    |path| crate::assets::declaration_texts::read_common_or_catalog(catalog, path))?;
                Self::read(&fields).map(Arc::new)
            })
    }
    /// Scene identities for normal acquisition/preparation, not a second
    /// resource cache. Repeated references remain borrowed declaration paths.
    pub fn scene_paths(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().flat_map(|rule| &rule.outcomes).flat_map(|outcome|
            outcome.midnight.iter().chain(outcome.recoil_midnights.iter().flatten())
                .chain(outcome.cancel_midnight.iter()).chain(outcome.attack_hit_midnight.iter()).map(String::as_str))
    }
    fn storage_bytes(&self) -> usize {
        self.rules.capacity() * std::mem::size_of::<Rule>() + self.rules.iter().map(|rule|
            rule.conditions.capacity() * std::mem::size_of::<Condition>()
                + rule.outcomes.capacity() * std::mem::size_of::<Outcome>()
                + rule.outcomes.iter().map(|outcome| outcome.midnight.iter().chain(outcome.recoil_midnights.iter().flatten())
                    .chain(outcome.cancel_midnight.iter()).chain(outcome.attack_hit_midnight.iter())
                    .map(String::capacity).sum::<usize>()).sum::<usize>()).sum::<usize>()
    }
}
fn resource_path(name: &str) -> String {
    let name = name.replace('\\', "/").to_ascii_lowercase();
    if name.starts_with("generated/decls/rulehandler/") { name }
    else { format!("generated/decls/rulehandler/{}.decl", name.trim_end_matches(".decl")) }
}
pub(crate) fn register_preload(catalog: &Catalog, name: &str, marker: u8, persistent: bool) -> Result<(), String> {
    catalog.rule_handlers.get_or_init(Default::default).lock().map_err(|_| "rule-handler registry is poisoned")?
        .register(&resource_path(name), marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.rule_handlers.get() {
        owner.lock().map_err(|_| "rule-handler registry is poisoned")?.promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.rule_handlers.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "ruleHandler", marker);
        owner.report(catalog.identity(), "ruleHandler", "scope-released", Definition::storage_bytes);
    }
}
pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.rule_handlers.get() {
        owner.lock().unwrap_or_else(|error| error.into_inner())
            .report(catalog.identity(), "ruleHandler", "catalog-retiring", Definition::storage_bytes);
    }
}
