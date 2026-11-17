//! arkCpntPlayerChaos4c1330/4c1610/4c16d0. Current level and accrued
//! points are separate source fields; kill outcomes never invent a level.
use crate::{archive::Catalog,entities::{Value,event_graph::atom},assets::slash_rules::chaos::{Lease,Condition}};

pub struct Definition { pub rules:Lease, pub deed_increment:i32 }
impl Definition {
    pub fn acquire(catalog:&Catalog,fields:&[(String,Value)])->Result<Self,String> {
        let name=atom(fields,"m_chaosRuleHandler").filter(|value|!value.is_empty()&&*value!="NULL")
            .ok_or("player Chaos has no ruleHandler declaration")?;
        Ok(Self{rules:Lease::acquire(catalog,name)?,deed_increment:atom(fields,"m_chaosDeedPointIncrement").unwrap_or("0")
            .parse().map_err(|_|"invalid Chaos deed increment")?})
    }
}
pub struct NpcFacts { pub morality:crate::assets::slash_rules::chaos::Morality,pub armed:bool,pub description_kind:u32 }
pub struct Runtime {
    definition:Definition,
    current_level:i32,
    current_points:i32,
    damage_type:crate::gameplay::damage::DamageType,
    cursors:Vec<usize>,
    exhausted:Vec<bool>,
    initialized:bool,
}
impl Runtime {
    ///4c1330 zeros the adjacent level/point fields, constructs780390's
    ///damage cache and leaves the compiled rule pointer absent until Init.
    pub fn new(definition:Definition)->Self {
        Self{definition,current_level:0,current_points:0,damage_type:crate::gameplay::damage::DamageType(0),
            cursors:Vec::new(),exhausted:Vec::new(),initialized:false}
    }
    pub fn initialize(&mut self){
        let count=self.definition.rules.rules.len();self.cursors.resize(count,0);self.cursors.fill(0);
        self.exhausted.resize(count,false);self.exhausted.fill(false);self.initialized=true;
    }
    pub fn initialized(&self)->bool{self.initialized}
    pub fn deinitialize(&mut self){self.initialized=false;self.cursors.clear();self.exhausted.clear();}
    pub fn level(&self)->Result<i32,String>{if self.initialized{Ok(self.current_level)}else{Err("PlayerChaos is not initialized".into())}}
    pub fn points(&self)->i32{self.current_points}
    ///4c13c0 adds a signed source message amount with native32-bit wrapping.
    pub fn add_points(&mut self,points:i32){self.current_points=self.current_points.wrapping_add(points);}
    ///537910 reads4c1460's real declaration+70 increment before4c13c0.
    pub fn add_deed(&mut self){self.add_points(self.definition.deed_increment);}
    pub fn damage_type(&self)->crate::gameplay::damage::DamageType{self.damage_type}
    ///4c14b0/4bc2c0 retains the actual accepted packet before its NPC gate.
    ///The host supplies actual native classification; no entity-name inference
    ///or fabricated NPC attributes may enter this callback.
    pub fn incapacitated(&mut self,params:&crate::gameplay::damage::Parameters,non_counting:bool,
        facts:Option<&NpcFacts>)->Result<(),String>{
        if !self.initialized{return Err("PlayerChaos callback before Init".into())}
        if non_counting{return Ok(())}
        self.damage_type=params.damage_type;
        if params.damage_type.0&1!=0{return Ok(())}
        let Some(facts)=facts.filter(|facts|facts.description_kind<2)else{return Ok(())};
        for(index,rule)in self.definition.rules.rules.iter().enumerate(){
            if self.exhausted[index]||!rule.conditions.iter().all(|condition|match condition{
                Condition::Morality(operator,value)=>operator.compare(facts.morality,*value),
                Condition::Armed(operator,value)=>operator.compare(facts.armed,*value),
            }){continue}
            let cursor=self.cursors[index];let amount=rule.outcomes[cursor%rule.outcomes.len()];
            self.cursors[index]=cursor.wrapping_add(1);
            if rule.once&&self.cursors[index]>=rule.outcomes.len(){self.exhausted[index]=true;}
            if amount>0{self.current_points=self.current_points.wrapping_add(amount);}
            return Ok(())
        }
        Ok(())
    }
}

impl super::Player {
    ///124550's actual +20 member, exposed to the canonical TI provider.
    pub fn chaos_level(&self)->Result<i32,String>{self.chaos.level()}
}
