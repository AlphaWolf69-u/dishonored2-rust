//! ae_fx schema1413964e0;8755c0 forwards both ordered joint names and
//!updatePosition toA7. Metadata belongs to the existing AnimSet declaration.
use std::sync::Arc;

#[derive(Clone,Debug)]
pub struct Effect {
    pub frame:i16,
    pub effect:Arc<str>,
    pub joint:Arc<str>,
    pub target_joint:Arc<str>,
    pub update_position:bool,
}
///Called by the shared AnimSet event walk, without lexing/scanning it twice.
pub(super) fn descriptor(fields:&[&str])->Result<Option<Effect>,String> {
    let field=|key|fields.iter().position(|token|*token==key).and_then(|index|fields.get(index+1)).map(|value|value.trim_matches('"'));
    let Some(effect)=field("fx").filter(|name|!name.is_empty()&&*name!="NULL") else{return Ok(None);};
    let frame=field("frame").unwrap_or("0").parse().map_err(|_|"invalid animation FX frame")?;
    let mut names=fields.windows(2).filter(|field|field[0]=="string").map(|field|field[1].trim_matches('"'));
    let joint=Arc::from(names.next().unwrap_or(""));
    let target_joint=Arc::from(names.next().unwrap_or(""));
    let update_position=match field("bool").unwrap_or("0") {"0"|"false"=>false,"1"|"true"=>true,_=>return Err("invalid animation FX updatePosition".into())};
    Ok(Some(Effect{frame,effect:Arc::from(effect),joint,target_joint,update_position}))
}
