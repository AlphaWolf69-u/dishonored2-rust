//! Opt-in replay of the actual F8 wall/feet/aim from collision-1795084974980.
//! Only setup restores position. Attack/charge/aim/query callbacks are real.
use super::*;

const FEET: Vec3 = Vec3::new(13.866425,87.776184,21.560005);
const FORWARD: Vec3 = Vec3::new(0.5445311,-0.82580554,0.14673439);
#[derive(Clone, Copy, Debug)]
enum Stage { Equip, LookAway, Charge, Aim, Contact }
pub(super) struct Observation {
    stage: Stage,
    since: Instant,
    held: Option<Input>,
    serial: u64,
    windows: u64,
}
impl Observation {
    pub(super) fn begin(world:&mut World)->Result<Self,String>{
        {
            let mut state=world.resource_mut::<PlayerState>();
            let p=state.player.as_mut().ok_or("wall replay Player absent")?;
            p.teleport(FEET,FORWARD.y.atan2(FORWARD.x));
        }
        eprintln!("ACTION_SMOKE_WALL_SETUP source=collision-1795084974980 level=eotw_p_lowchaos feet={FEET:?} forward={FORWARD:?} setup_restore_only=true no_collision_or_state_override=true");
        let input=input_for(19)?;send(world,input,true)?;send(world,input,false)?;
        Ok(Self{stage:Stage::Equip,since:Instant::now(),held:None,serial:0,windows:0})
    }
    fn steer(world:&mut World,forward:Vec3)->Result<bool,String>{
        let (_,axis)=player(world)?.published_camera_frame();
        let current=axis.x_axis;
        let yaw=forward.y.atan2(forward.x)-current.y.atan2(current.x);
        let yaw=yaw.sin().atan2(yaw.cos());
        let pitch=-forward.z.atan2(forward.truncate().length())+current.z.atan2(current.truncate().length());
        world.write_message(bevy::input::mouse::MouseMotion{delta:Vec2::new((-yaw*100.).clamp(-8.,8.),(pitch*100.).clamp(-8.,8.))});
        // Diagnostic setup tolerance only; no production collision predicate.
        Ok(yaw.abs()<0.01&&pitch.abs()<0.01)
    }
    fn fixture(world:&World)->Result<crate::gameplay::collision_hit::ShapeHitInfo,String>{
        let p=player(world)?;
        let (identity,_,_)=p.equipment_action_status(1).ok_or("wall fixture sword absent")?;
        let decl=world.resource::<super::super::player_actions::PlayerActions>().equipment_metadata(identity)
            .and_then(|metadata|metadata.melee.as_ref()).ok_or("wall fixture sword declaration absent")?;
        let equipment=super::super::equipment_identity::id(world,identity);
        let scene=world.resource::<super::super::physics_scene::PhysicsScene>();
        let live=world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
        let (eye,axis)=p.published_camera_frame();
        let mut queries=super::super::physics_scene::melee::MeleeQueries::default();
        let contacts=scene.melee_ray_contacts(live,&mut queries,&super::super::physics_scene::melee::Ray{
            origin:eye.to_array(),direction:(axis.x_axis*decl.dimensions.x).to_array(),maximum_fraction:1.,layers:0x443e})?;
        for contact in contacts{
            if contact.entity.is_some()&&(contact.entity==p.managed_entity()||contact.entity==equipment){continue}
            if !scene.melee_contact_world(live,contact,3)?{return Err("captured wall has a different actual non-WORLD occluder".into());}
            let super::super::physics_scene::melee::Hit::Ray(hit)=contact.hit else{return Err("wall ray produced a non-ray witness".into());};
            let point=Vec3::from_slice(&hit.position);
            let normal=Vec3::from_slice(&hit.normal);
            // Match the saved collision surface, allowing camera animation
            // while steering; these bounds gate only diagnostic setup.
            // F8's actual center-ray triangle (report line13), not the
            // unrelated off-axis last melee packet from report line367.
            let a=Vec3::new(14.715664,87.30931,21.549978);
            let b=Vec3::new(4.488506,80.475746,26.049978);
            let c=Vec3::new(14.715664,87.30931,26.049978);
            let expected_normal=(b-a).cross(c-a).normalize();
            if point.distance(Vec3::new(14.33991,87.05824,23.376156))>0.15
                ||normal.dot(expected_normal)<0.99{
                return Err(format!("captured wall geometry differs: actual={hit:?}"));
            }
            if !decl.world_focus.contains(p.physics.position(),eye,axis,decl.focus_step_back,point){
                return Err("captured wall center ray is outside the actual authored focus".into());
            }
            return Ok(hit.target);
        }
        Err("captured wall has no actual ray collision".into())
    }
    pub(super) fn poll(&mut self,world:&mut World)->Result<bool,String>{
        if self.since.elapsed()>Duration::from_secs(45){
            let mut report=String::new();
            if let Some(owner)=world.get_resource::<super::super::melee::Melee>(){owner.write_diagnostics(&mut report);}
            eprintln!("ACTION_SMOKE_WALL_FAILURE stage={:?}{report}",self.stage);
            if let Some(input)=self.held.take(){send(world,input,false)?;}
            return Err(format!("captured wall thrust incomplete at {:?}; retained full forward-window trace",self.stage));
        }
        match self.stage {
            Stage::Equip if player(world)?.equipment_action_status(1).is_some_and(|(_,equipped,transitioning)|equipped&&!transitioning)
                &&!player(world)?.machine.is_active(StateId::SLASH)=>{self.stage=Stage::LookAway;}
            Stage::LookAway=>{
                // Charge away from the wall so the initial ordinary swing
                // cannot terminate the real held Slash before its thrust.
                if Self::steer(world,Vec3::new(FORWARD.x*0.1,FORWARD.y*0.1,1.).normalize())?{
                    let input=input_for(19)?;send(world,input,true)?;self.held=Some(input);self.stage=Stage::Charge;
                }
            }
            Stage::Charge=>{
                if player(world)?.machine.melee_focus_charge().is_some_and(|(elapsed,required)|elapsed>=required){self.stage=Stage::Aim;}
            }
            Stage::Aim=>{
                if Self::steer(world,FORWARD)?{
                    let hit=Self::fixture(world)?;
                    eprintln!("ACTION_SMOKE_WALL_ACTUAL_RAY hit={hit:?} player_feet={:?} published_camera={:?}",player(world)?.feet(),player(world)?.published_camera_frame());
                    self.serial=super::super::melee::world_action_receipt(world).map_or(0,|receipt|receipt.0);
                    self.windows=super::super::melee::active_windows(world);
                    send(world,self.held.take().ok_or("wall charge physical hold absent")?,false)?;
                    self.stage=Stage::Contact;self.since=Instant::now();
                }
            }
            Stage::Contact=>{
                if let Some((serial,hit,focused,forward,recoil,ticket))=super::super::melee::world_action_receipt(world){
                    if serial>self.serial&&focused&&forward&&ticket&&super::super::melee::active_windows(world)>self.windows{
                        let mut report=String::new();
                        world.resource::<super::super::melee::Melee>().write_diagnostics(&mut report);
                        eprintln!("ACTION_SMOKE_PASS check=captured_wall_forward actual_hit={hit:?} recoil={recoil} actual_weak_FX_ticket=true{report}");
                        return Ok(true);
                    }
                }
            }
            _=>{}
        }
        Ok(false)
    }
}
