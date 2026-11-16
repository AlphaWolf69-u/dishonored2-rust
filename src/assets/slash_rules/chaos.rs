//! RULE_TYPE_CHAOS, compiled by4c1610 through the canonical RuleHandler owner.
use super::{Fields,Operator};
use crate::{archive::Catalog,entities::event_graph::{atom,child}};
use std::sync::Arc;

#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
#[repr(i32)]
pub enum Morality { Sympathetic=0,Guilty=1,Murderous=2,Undefined=3 }
impl Morality {
    ///141a0d3d0 registers this precise enum ordering.
    pub fn read(value:&str)->Result<Self,String>{match value{
        "MORALITY_SYMPATHETIC"|"0"=>Ok(Self::Sympathetic),"MORALITY_GUILTY"|"1"=>Ok(Self::Guilty),
        "MORALITY_MURDEROUS"|"2"=>Ok(Self::Murderous),"MORALITY_UNDEFINED"|"3"=>Ok(Self::Undefined),
        _=>Err(format!("unknown morality level {value}")),
    }}
}

#[derive(Debug,PartialEq)]
pub enum Condition { Morality(Operator,Morality), Armed(Operator,bool) }
#[derive(Debug,PartialEq)]
pub struct Rule { pub conditions:Vec<Condition>,pub outcomes:Vec<i32>,pub once:bool }
#[derive(Debug,PartialEq)]
pub struct Definition { pub rules:Vec<Rule> }
#[derive(Clone,Debug,PartialEq)]
pub struct Lease(Arc<super::Definition>);
impl std::ops::Deref for Lease {
    type Target=Definition;
    fn deref(&self)->&Definition {self.0.chaos.as_ref().expect("checked CHAOS resource projection")}
}
impl Lease {
    pub fn acquire(catalog:&Catalog,name:&str)->Result<Self,String> {
        let definition=super::Definition::acquire_shared(catalog,name)?;
        if definition.chaos.is_none(){return Err(format!("{name}: ruleHandler is not CHAOS"))}
        Ok(Self(definition))
    }
}
impl Definition {
    pub fn read(fields:&Fields)->Result<Self,String> {
        let mut rules=Vec::new();
        for fields in super::list(fields,"m_rules")? {
            let conditions=super::list(fields,"m_conditions")?.into_iter().map(|fields|{
                let class=atom(fields,"m_class").ok_or("Chaos condition class absent")?;
                let fields=child(fields,"m_object");let operator=Operator::read(fields)?;
                let variable=atom(fields,"m_variable").ok_or("Chaos condition variable absent")?;
                let value=atom(fields,"m_value").ok_or("Chaos condition value absent")?;
                match (class,variable) {
                    ("arkRuleCondition_arkMoralityLevel_t","description.npc.morality")=>Ok(Condition::Morality(operator,Morality::read(value)?)),
                    ("arkRuleCondition_bool","description.npc.isArmed")=>Ok(Condition::Armed(operator,match value {"true"|"1"=>true,"false"|"0"=>false,_=>return Err("invalid Chaos armed value".into())})),
                    _=>Err(format!("unsupported Chaos condition {class}:{variable}")),
                }
            }).collect::<Result<_,String>>()?;
            let outcomes=super::list(fields,"m_actions")?.into_iter().map(|fields|{
                if atom(fields,"m_class")!=Some("arkRuleAction_ChaosOutcome"){return Err("unsupported Chaos outcome class".into())}
                //695c50 constructor stores3 before authored edits.
                atom(child(fields,"m_object"),"m_chaos").unwrap_or("3").parse::<i32>().map_err(|_|"invalid Chaos outcome value".into())
            }).collect::<Result<Vec<_>,String>>()?;
            if outcomes.is_empty(){return Err("Chaos rule has no outcomes".into())}
            let once=match atom(fields,"m_executionPolicy").unwrap_or("RULE_EXEC_NONE") {
                "RULE_EXEC_NONE"|"0"|"RULE_EXEC_SEQUENCE"|"1"=>false,
                "RULE_EXEC_SEQUENCE_ONCE"|"3"=>true,
                policy=>return Err(format!("unsupported Chaos execution policy {policy}")),
            };
            rules.push(Rule{conditions,outcomes,once});
        }
        Ok(Self{rules})
    }
    pub(super) fn storage_bytes(&self)->usize {
        self.rules.capacity()*std::mem::size_of::<Rule>()+self.rules.iter().map(|rule|
            rule.conditions.capacity()*std::mem::size_of::<Condition>()+rule.outcomes.capacity()*4).sum::<usize>()
    }
}
