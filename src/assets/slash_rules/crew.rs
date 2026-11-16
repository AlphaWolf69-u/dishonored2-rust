//! AI_CREW projection of the same ruleHandler declaration generation.
//! 893b30 initializes execution policy0; 893e30 removes that policy before
//! 8952a0 creates per-rule instances. Declaration actions remain shared.
use super::{Fields, Operator};
use crate::{
    archive::Catalog,
    entities::event_graph::{atom, child},
};
use std::sync::Arc;

#[derive(Debug, PartialEq)]
pub struct Condition {
    pub variable: Arc<str>,
    pub value: Arc<str>,
    pub(crate) operator: Operator,
}
#[derive(Debug, PartialEq)]
pub struct Rule {
    pub conditions: Vec<Condition>,
    pub trees: Vec<Arc<str>>,
    pub policy: u8,
}
#[derive(Debug, PartialEq)]
pub struct Definition {
    pub rules: Vec<Rule>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Lease(Arc<super::Definition>);
impl std::ops::Deref for Lease {
    type Target = Definition;
    fn deref(&self) -> &Definition {
        self.0
            .crew
            .as_ref()
            .expect("checked AI_CREW resource projection")
    }
}
impl Lease {
    pub fn acquire(catalog: &Catalog, name: &str) -> Result<Self, String> {
        let definition = super::Definition::acquire_shared(catalog, name)?;
        if definition.crew.is_none() {
            return Err(format!("{name}: ruleHandler is not AI_CREW"));
        }
        Ok(Self(definition))
    }
}
impl Definition {
    pub fn read(fields: &Fields) -> Result<Self, String> {
        let mut rules = Vec::new();
        for fields in super::list(fields, "m_rules")? {
            let policy = match atom(fields, "m_executionPolicy").unwrap_or("RULE_EXEC_NONE") {
                "RULE_EXEC_NONE" | "0" => 0,
                "RULE_EXEC_SEQUENCE" | "1" => 1,
                "RULE_EXEC_SHUFFLE" | "2" => 2,
                "RULE_EXEC_SEQUENCE_ONCE" | "3" => 3,
                "RULE_EXEC_SHUFFLE_ONCE" | "4" => 4,
                value => return Err(format!("unknown crew rule execution policy {value}")),
            };
            let conditions = super::list(fields, "m_conditions")?
                .into_iter()
                .map(|fields| {
                    if atom(fields, "m_class") != Some("arkRuleCondition_idAtomicString") {
                        return Err("unsupported crew rule condition class".into());
                    }
                    let fields = child(fields, "m_object");
                    Ok(Condition {
                        variable: atom(fields, "m_variable")
                            .ok_or("crew rule variable absent")?
                            .into(),
                        value: atom(fields, "m_value").unwrap_or("").into(),
                        operator: Operator::read(fields)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let trees = super::list(fields, "m_actions")?
                .into_iter()
                .map(|fields| {
                    if atom(fields, "m_class") != Some("arkRuleAction_AISituationTreeOutcome") {
                        return Err("unsupported crew rule action class".into());
                    }
                    Ok(Arc::from(
                        atom(child(fields, "m_object"), "m_situationTree")
                            .ok_or("crew rule tree absent")?,
                    ))
                })
                .collect::<Result<Vec<Arc<str>>, String>>()?;
            rules.push(Rule {
                conditions,
                trees,
                policy,
            });
        }
        Ok(Self { rules })
    }
    pub fn active_rules(&self) -> impl Iterator<Item = (usize, &Rule)> {
        self.rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.policy != 0 && !rule.trees.is_empty())
    }
    pub(super) fn storage_bytes(&self) -> usize {
        self.rules.capacity() * std::mem::size_of::<Rule>()
            + self
                .rules
                .iter()
                .map(|rule| {
                    rule.conditions.capacity() * std::mem::size_of::<Condition>()
                        + rule.trees.capacity() * std::mem::size_of::<Arc<str>>()
                        + rule
                            .conditions
                            .iter()
                            .map(|condition| condition.variable.len() + condition.value.len())
                            .sum::<usize>()
                        + rule.trees.iter().map(|tree| tree.len()).sum::<usize>()
                })
                .sum::<usize>()
    }
}
