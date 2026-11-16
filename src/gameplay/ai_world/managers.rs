//! AIStatistics431670 and TimersManager778680 own numeric rows only.
//! No archive/model/audio resource is created by either constructor.
use crate::gameplay::{component_state::{ComponentInstance,ComponentOwner},entity_lifetime::EntityId};

pub const STATISTICS_EVENTS:[u16;4]=[0x117,0xa5,0x3b,0x2b];
pub struct Statistics{
    pub owner:ComponentOwner,
    ///431c90 records each actual managed NPC once before incrementing10.
    pub recorded:Vec<EntityId>,
    pub initialized:bool,
    pub started:bool,
}
impl Statistics{
    pub fn new()->Self{Self{owner:ComponentOwner::default(),recorded:Vec::new(),initialized:false,started:false}}
    pub fn clear(&mut self){self.recorded.clear();self.recorded.shrink_to_fit();}
    pub fn record(&mut self,id:EntityId)->bool{
        if self.recorded.contains(&id){false}else{self.recorded.push(id);true}
    }
}
impl ComponentInstance for Statistics{
    fn owner(&self)->&ComponentOwner{&self.owner}
    fn owner_mut(&mut self)->&mut ComponentOwner{&mut self.owner}
}

///779010's source selector: two game clocks, manager entity, target entity,
///and the final native clock4 branch. It is not a wall-clock deadline.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Clock{Game3,Game2,Manager,Target,Game4}
impl Clock{
    pub fn from_source(value:i32)->Self{match value{0=>Self::Game3,1=>Self::Game2,2=>Self::Manager,3=>Self::Target,_=>Self::Game4}}
}
#[derive(Clone,Copy,Debug)]
pub struct Timer{pub entity:Option<EntityId>,pub id:i32,pub remaining:f32,pub clock:Clock}
pub struct Timers{
    pub owner:ComponentOwner,
    rows:Vec<Timer>,
    pub initialized:bool,
    pub started:bool,
    next:Option<usize>,
    ///Host retry acknowledgement, keyed by the actual game-clock frame.
    pub updated_frame:Option<i32>,
}
impl Timers{
    pub fn new()->Self{
        let mut owner=ComponentOwner::default();owner.request_update(1);
        Self{owner,rows:Vec::new(),initialized:false,started:false,next:None,updated_frame:None}
    }
    pub fn rows(&self)->&[Timer]{&self.rows}
    ///778f40 updates only the remaining duration of an existing key. The
    ///clock selector remains the one stored by its original insertion.
    pub fn set(&mut self,entity:Option<EntityId>,id:i32,remaining:f32,clock:Clock)->Result<bool,String>{
        if self.next.is_some(){return Err("timer insertion during its accepted update pass".into());}
        if let Some(row)=self.rows.iter_mut().find(|row|row.entity==entity&&row.id==id){row.remaining=remaining;return Ok(false);}
        let was_empty=self.rows.is_empty();self.rows.push(Timer{entity,id,remaining,clock});Ok(was_empty)
    }
    ///778ca0 prunes only missing target-clock rows when restoredbit30 is set.
    pub fn restore_start(&mut self,exists:impl Fn(EntityId)->bool){
        if self.owner.flags&(1<<30)!=0{
            for index in (0..self.rows.len()).rev(){
                let row=self.rows[index];
                if row.clock==Clock::Target&&row.entity.is_none_or(|id|!exists(id)){self.rows.swap_remove(index);}
            }
        }
    }
    ///779010 iterates in reverse, posts D1 before removing each row, and
    ///pauses only when the final row disappears. The retained cursor prevents
    ///a callback retry from decrementing already completed rows twice.
    pub fn update(&mut self,mut delta:impl FnMut(Clock,Option<EntityId>)->Result<f32,String>,
        exists:impl Fn(EntityId)->bool,mut expired:impl FnMut(EntityId,i32))->Result<bool,String>{
        let mut cursor=self.next.unwrap_or(self.rows.len());
        while cursor>0{
            let index=cursor-1;let row=self.rows[index];self.next=Some(cursor);
            let step=delta(row.clock,row.entity)?;
            self.rows[index].remaining-=step;
            if row.entity.is_none_or(|id|!exists(id))||self.rows[index].remaining<=0.{
                if let Some(entity)=row.entity.filter(|&entity|exists(entity)){expired(entity,row.id);}
                self.rows.swap_remove(index);
            }
            cursor=index;self.next=Some(cursor);
        }
        self.next=None;Ok(self.rows.is_empty())
    }
    ///778d80 frees the numeric timer list on Stop, not merely Deinit.
    pub fn stop(&mut self){self.rows.clear();self.rows.shrink_to_fit();self.next=None;self.started=false;}
}
impl ComponentInstance for Timers{
    fn owner(&self)->&ComponentOwner{&self.owner}
    fn owner_mut(&mut self)->&mut ComponentOwner{&mut self.owner}
}
