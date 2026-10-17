//! Native model-owner caster bucket state, not file-type or visibility flags.

/// FUN_140233e50 writes 8 at its +0x5c (idRenderModel +0x18bc).
/// This is only the constructor's flags byte, not a complete model owner.
/// The dynamic-caster bit starts clear for BML and default/rest-pose BMD6;
/// callers must preserve subsequent actual runtime setter events.
pub const INITIAL_MODEL_CONTEXT_FLAGS_18BC: u8 = 8;

#[derive(Clone,Copy,Debug,Default,PartialEq,Eq)]
pub struct LocalShadowCacheControls {
    pub discard_models:bool,
    pub clear_all:bool,
}

impl LocalShadowCacheControls {
    pub fn actor_caster_changed(&mut self,changed:bool,dynamic:bool) {
        if changed {
            if dynamic {self.clear_all=true;}else{self.discard_models=true;}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeModelCasterInfo {
    pub flags: u8,
    /// The native producer leaves the preceding sphere unchanged when bit2
    /// is absent. No consumer may read a newly invented sphere in that case.
    pub sphere: Option<[f32;4]>,
}

/// Caster publication shared by FUN_140237660/140268de0 and the skinned
/// bounds update. Surface entries are the native vertex count and material
/// +0x48 byte, before forward visibility or shader-slot filtering.
pub fn prepare_model_caster_info(
    bounds: [[f32;3];2], flags_1c30: u8, flags_18bc: u8, flags_18bd: u8,
    skinned: bool, surfaces: impl IntoIterator<Item=(i32,u8)>,
) -> NativeModelCasterInfo {
    let mut flags=if skinned {9} else {1};
    let admitted=(flags_1c30&0x20!=0 && flags_18bc&8!=0)
        || surfaces.into_iter().any(|(vertices,material)|vertices>0 && material&1!=0);
    if !admitted {return NativeModelCasterInfo{flags,sphere:None};}
    let mut extent=bounds[1][1]-bounds[0][1];
    let z=bounds[1][2]-bounds[0][2];
    if extent<=z {extent=z;}
    let mut size=bounds[1][0]-bounds[0][0];
    if size<=extent {size=extent;}
    size*=0.5;
    flags|=2;
    // DAT_1422b3658 is 1.0, referenced only by the four bound producers.
    if size<1.0 && flags_18bc&2==0 && flags_18bd&8==0 {flags|=4;}
    NativeModelCasterInfo {flags,sphere:Some([
        (bounds[0][0]+bounds[1][0])*0.5,
        (bounds[0][1]+bounds[1][1])*0.5,
        (bounds[0][2]+bounds[1][2])*0.5,size,
    ])}
}

/// FUN_140330020's collection gate and surface-sort distance. Its camera
/// inverse view is applied to DAT_14228a9a0=[0,0,0,1], in native sum order.
pub fn model_caster_sort_distance(
    flags: u8, sphere: [f32;4], inverse_view: &[[f32;4];4], cull_distance:f32,
) -> Option<f32> {
    if flags&2==0 {return None;}
    let camera: [f32;3]=std::array::from_fn(|r|
        ((0.0*inverse_view[r][0]+0.0*inverse_view[r][1])+0.0*inverse_view[r][2])+inverse_view[r][3]);
    let d: [f32;3]=std::array::from_fn(|axis|sphere[axis]-camera[axis]);
    let squared=(d[1]*d[1]+d[0]*d[0])+d[2]*d[2];
    if flags&4!=0 && !(squared<=cull_distance*cull_distance) {return None;}
    Some(squared.sqrt()-sphere[3])
}

/// FUN_14032bf60: farthest AABB support value along the normalized radial
/// direction from the supplied light point. The collector sorts its raw
/// float bits as UNSIGNED integers, including ties and negative values.
pub fn model_shadow_radial_sort_key(bounds:[[f32;3];2],point:[f32;3])->u32 {
    let center:[f32;3]=std::array::from_fn(|i|(bounds[0][i]+bounds[1][i])*0.5);
    let extent:[f32;3]=std::array::from_fn(|i|(bounds[1][i]-bounds[0][i])*0.5);
    let delta:[f32;3]=std::array::from_fn(|i|center[i]-point[i]);
    let squared=(delta[1]*delta[1]+delta[0]*delta[0])+delta[2]*delta[2];
    let denominator=if squared>f32::MIN_POSITIVE {squared} else {f32::MIN_POSITIVE};
    let reciprocal=1.0/denominator.sqrt();
    let direction=delta.map(|value|value*reciprocal);
    let absolute=|value:f32|f32::from_bits(value.to_bits()&0x7fff_ffff);
    let support=(absolute(extent[1]*direction[1])+absolute(extent[0]*direction[0]))
        +absolute(extent[2]*direction[2]);
    let projected=(direction[1]*center[1]+direction[0]*center[0])+direction[2]*center[2];
    (support+projected).to_bits()
}

///10a350 mode0: strict point half-space mask. Row0 preserves its native
///Y+X summation, while rows1..3 use X+Y. Unordered comparisons stay false.
pub fn native_point_clip_mask(matrix:&[[f32;4];4],point:[f32;3])->u8 {
    let [x,y,z]=point;
    let p:[f32;4]=std::array::from_fn(|r| {
        let a=matrix[r];
        let xy=if r==0 {y*a[1]+x*a[0]}else{x*a[0]+y*a[1]};
        (xy+z*a[2])+a[3]
    });
    let inside=[-p[3]<p[0],p[0]<p[3],-p[3]<p[1],p[1]<p[3],0.0<p[2],p[2]<p[3]];
    inside.into_iter().enumerate().fold(0x3f,|mask,(bit,inside)|if inside {mask^(1<<bit)}else{mask})
}

#[derive(Clone,Copy)]
pub struct LocalModelCasterRecord {
    pub bounds:[[f32;3];2],pub camera_clip_mask:u8,
}

///3285a0 static /328900 dynamic: whole-light unit-volume admission and
///radial surface order. Dynamic records also compare the camera clip mask
///against the actual light point's mask, before touching light bounds.
pub fn collect_local_model_casters(records:&[LocalModelCasterRecord],light_clip:&[[f32;4];4],
    camera_clip:&[[f32;4];4],point:[f32;3],dynamic:bool,view_flags:u16,skip_sort:bool)->Vec<usize> {
    if view_flags&2!=0 {return Vec::new();}
    let point_mask=native_point_clip_mask(camera_clip,point);
    let clip=std::array::from_fn(|i|light_clip[i/4][i%4]);
    let mut selected=Vec::new();let mut keys=Vec::new();
    for (index,record) in records.iter().enumerate() {
        if dynamic && record.camera_clip_mask&(record.camera_clip_mask^point_mask)!=0 {continue;}
        if super::native_unit_aabb_clip_cull(&clip,record.bounds[0],record.bounds[1]).culled {continue;}
        selected.push(index);keys.push(model_shadow_radial_sort_key(record.bounds,point));
    }
    if !skip_sort {sort_model_shadow_records(&mut selected,&mut keys).expect("paired native collector keys");}
    selected
}

/// FUN_1403345c0's paired key/record sort. In particular, equal-key swaps
/// follow the original partition and short-range selection order; a host
/// stable/unstable sort is not an equivalent tie policy.
pub fn sort_model_shadow_records<T>(records:&mut [T],keys:&mut [u32])->Result<(),String> {
    if records.len()!=keys.len() {return Err("native model shadow records and keys differ in length".into());}
    if records.is_empty() {return Ok(());}
    let mut pending=Vec::new();
    let (mut low,mut high)=(0isize,records.len() as isize-1);
    loop {
        if high-low<4 || pending.len()>126 {
            while low<high {
                let mut maximum=low;
                for at in low+1..=high {
                    if keys[maximum as usize]<keys[at as usize] {maximum=at;}
                }
                records.swap(maximum as usize,high as usize);
                keys.swap(maximum as usize,high as usize);
                high-=1;
            }
            let Some((next_low,next_high))=pending.pop() else {return Ok(());};
            low=next_low;high=next_high;
            continue;
        }
        let pivot=keys[((low+high)/2) as usize];
        let (mut left,mut right)=(low,high);
        loop {
            while keys[left as usize]<pivot {left+=1;}
            while pivot<keys[right as usize] {right-=1;}
            if right<left {break;}
            records.swap(left as usize,right as usize);
            keys.swap(left as usize,right as usize);
            left+=1;right-=1;
            if left>=right {break;}
        }
        while keys[right as usize]==pivot && low<right {right-=1;}
        while keys[left as usize]==pivot && left<high {left+=1;}
        pending.push((left,high));
        high=right;
    }
}

/// FUN_140330020's owner-bucket predicate, separate from model-query flags.
pub fn model_is_dynamic_caster(model_flags_18bc: u8) -> bool {
    model_flags_18bc & 2 != 0
}

/// Complete FUN_1401be960 mutation, preserving seven bits and using value&1.
pub fn set_model_dynamic_caster_flag(model_flags_18bc: &mut u8, value: u8) {
    *model_flags_18bc = (*model_flags_18bc & !2) | ((value & 1) << 1);
}

/// Owned equivalents of235880's +16830/+f018 pending-ID lists. The native
/// capacity-growth helpers are represented by Vec's normal owned growth.
#[derive(Default)]
pub struct ModelOwnerDirtyQueues {
    pub flag_40:Vec<u16>,
    pub flag_20:Vec<u16>,
}

pub fn mark_model_owner_dirty(model_flags:&mut u8,owner_flags:&mut u16,
    requested:u16,id:u16,queues:&mut ModelOwnerDirtyQueues) {
    *model_flags&=!1;
    let mut flags=requested|*owner_flags;
    if flags&8!=0 && flags&0x40==0 {flags|=0x40;queues.flag_40.push(id);}
    if flags&0x20==0 {flags|=0x20;queues.flag_20.push(id);}
    *owner_flags=flags;
}

/// Final bounds flag in1fc110 after235880. Preserve IEEE comparisons,
/// including unordered values and signed zero; do not replace with a length.
pub fn model_owner_bounds_flags(flags:u8,min:[f32;4],max:[f32;4])->u8 {
    let empty=min.iter().zip(max).all(|(&a,b)|a==b) || (max[0]<=min[0] && min[0]!=max[0]);
    (flags&!1)|u8::from(empty)
}

/// FUN_1401be8d0: update this owner, then call every child's actual +0x68
/// virtual setter in source-list order, even if its bit already matches.
/// `children` is the real owner's decoded low24-count list. The supplied
/// dispatcher retains each child's actual override (leaf or recursive).
/// Actor traversal (87ddd0), bucket registration/unlink and pickup lifetime
/// are NOT replaced by this helper; those require their own owned inputs.
pub fn set_model_dynamic_caster_with_children(
    model_flags_18bc: &mut u8,
    value: u8,
    children: &[usize],
    mut dispatch_child: impl FnMut(usize, u8),
) {
    set_model_dynamic_caster_flag(model_flags_18bc, value);
    for &child in children {
        dispatch_child(child, value);
    }
}

/// Owned access to the exact actor/model relationships consumed by 87ddd0.
/// Counts are already decoded low24 lengths. Virtual calls retain the actual
/// model override; this interface does not infer graph membership or unlinking.
pub trait ActorCasterAccess {
    fn actor_flags_100(&self, actor: usize) -> u8;
    fn primary_model(&self, actor: usize) -> Option<usize>;
    fn additional_model_count(&self, actor: usize) -> usize;
    fn additional_model(&self, actor: usize, index: usize) -> usize;
    fn child_actor_count(&self, actor: usize) -> usize;
    fn child_actor(&self, actor: usize, index: usize) -> usize;
    fn model_flags_18bc(&self, model: usize) -> u8;
    fn model_flags_1c30(&self, model: usize) -> u8;
    fn model_slot68(&mut self, model: usize, value: u8);
    fn model_slot28(&mut self, model: usize, value: u32);
}

/// Complete FUN_14087ddd0 traversal/dispatch, not its virtual implementations.
/// Machine code 14087de14..1e confirms the primary setter also receives DL;
/// the decompiler omits that argument. Compare the whole incoming byte, not
/// value&1: normalization occurs in each actual model setter, not this caller.
pub fn set_actor_dynamic_caster(
    access: &mut impl ActorCasterAccess,
    actor: usize,
    value: u8,
) -> bool {
    let mut changed = false;
    if access.actor_flags_100(actor) & 8 != 0 {
        if let Some(model) = access.primary_model(actor) {
            if (access.model_flags_18bc(model) >> 1) & 1 != value {
                changed = true;
                access.model_slot68(model, value);
                if access.model_flags_1c30(model) & 1 == 0 {
                    access.model_slot28(model, 0);
                }
            }
        }
        let count = access.additional_model_count(actor);
        for index in 0..count {
            let model = access.additional_model(actor, index);
            if (access.model_flags_18bc(model) >> 1) & 1 != value {
                changed = true;
                access.model_slot68(model, value);
                // The original reloads this list entry AFTER the virtual call.
                let model = access.additional_model(actor, index);
                if access.model_flags_1c30(model) & 1 == 0 {
                    access.model_slot28(model, 0);
                }
            }
        }
    }
    // Actor children are visited even when this actor's model flag is clear.
    let count = access.child_actor_count(actor);
    for index in 0..count {
        let child = access.child_actor(actor, index);
        changed |= set_actor_dynamic_caster(access, child, value);
    }
    changed
}

/// 523af0's actor dispatch and view invalidation, after its actor-selection
/// list has been resolved. Missing actors are skipped; changes are accumulated
/// before publishing one view signal. Midnight uses the same transition rule.
pub fn set_actors_dynamic_caster_for_view(
    access:&mut impl ActorCasterAccess,
    actors:impl IntoIterator<Item=Option<usize>>,
    dynamic:bool,
    controls:&mut LocalShadowCacheControls,
)->bool {
    let mut changed=false;
    for actor in actors.into_iter().flatten() {
        changed|=set_actor_dynamic_caster(access,actor,u8::from(dynamic));
    }
    controls.actor_caster_changed(changed,dynamic);
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_owner_dirty_and_bounds_match_original_instructions() {
        for (case,line) in include_str!("../../analysis/original_model_owner_dirty_cases.txt").lines().enumerate() {
            let (input,output)=line.split_once('|').unwrap();
            let words=|s:&str|s.split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap()).collect::<Vec<_>>();
            let w=words(input);let expected=words(output);
            let mut model=w[1] as u8;let mut owner=w[2] as u16;let mut queues=ModelOwnerDirtyQueues::default();
            mark_model_owner_dirty(&mut model,&mut owner,w[3] as u16,w[4] as u16,&mut queues);
            if w[0]!=0 {model=model_owner_bounds_flags(model,std::array::from_fn(|i|f32::from_bits(w[5+i])),std::array::from_fn(|i|f32::from_bits(w[9+i])));}
            let mut actual=vec![u32::from(model),u32::from(owner),(queues.flag_40.len()+queues.flag_20.len()) as u32];
            actual.extend(queues.flag_40.iter().map(|_|0x40).chain(queues.flag_20.iter().map(|_|0x20)));
            actual.extend([queues.flag_40.first().copied().unwrap_or(u16::MAX) as u32,queues.flag_20.first().copied().unwrap_or(u16::MAX) as u32]);
            assert_eq!(actual,expected,"native model owner update case{case}");
        }
    }

    #[test]
    fn local_model_collectors_match_original_unit_clip_masks_and_sorting() {
        let mut cases=0;
        for line in include_str!("../../analysis/original_local_model_caster_cases.txt").lines() {
            let w=line.split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap()).collect::<Vec<_>>();
            let value=|at|f32::from_bits(w[at]);let count=w[4] as usize;
            let point=std::array::from_fn(|i|value(5+i));
            let light=std::array::from_fn(|r|std::array::from_fn(|c|value(8+r*4+c)));
            let camera=std::array::from_fn(|r|std::array::from_fn(|c|value(24+r*4+c)));
            let records=(0..count).map(|i|LocalModelCasterRecord {
                bounds:std::array::from_fn(|r|std::array::from_fn(|c|value(40+i*7+r*3+c))),camera_clip_mask:w[40+i*7+6] as u8,
            }).collect::<Vec<_>>();
            let expected=40+count*7;assert_eq!(w[expected] as usize,w.len()-expected-1);
            let actual=collect_local_model_casters(&records,&light,&camera,point,w[1]!=0,w[2] as u16,w[3]!=0);
            assert_eq!(actual.iter().map(|&v|v as u32).collect::<Vec<_>>(),w[expected+1..],"native local model collector case{}",w[0]);cases+=1;
        }
        assert_eq!(cases,384);
    }

    #[test]
    fn model_shadow_margin_clip_matches_original() {
        let mut count=0;
        for line in include_str!("../../analysis/original_shadow_model_clip_cases.txt").lines() {
            let w:Vec<u32>=line.split_whitespace().map(|s|u32::from_str_radix(s,16).unwrap()).collect();
            let matrix=std::array::from_fn(|i|f32::from_bits(w[i]));
            let minimum=std::array::from_fn(|i|f32::from_bits(w[16+i]));
            let maximum=std::array::from_fn(|i|f32::from_bits(w[20+i]));
            let actual=super::super::native_transformed_aabb_clip_cull_at_depth(&matrix,minimum,maximum,f32::from_bits(w[24]));
            assert_eq!(actual.output_byte,w[25] as u8,"native margin clip mask {count}");
            assert_eq!(actual.culled,w[26]!=0,"native margin clip result {count}");
            count+=1;
        }
        assert_eq!(count,512);
    }

    #[test]
    fn model_shadow_radial_keys_and_tie_order_match_original() {
        let (mut radial,mut sorting)=(0,0);
        for line in include_str!("../../analysis/original_shadow_model_order_cases.txt").lines() {
            let (kind,words)=line.split_once(' ').unwrap();
            let w:Vec<u32>=words.split_whitespace().map(|s|u32::from_str_radix(s,16).unwrap()).collect();
            if kind=="K" {
                let bounds=std::array::from_fn(|r|std::array::from_fn(|c|f32::from_bits(w[r*3+c])));
                let point=std::array::from_fn(|i|f32::from_bits(w[6+i]));
                assert_eq!(model_shadow_radial_sort_key(bounds,point),w[9],"native radial key {radial}");
                radial+=1;
            } else {
                let count=w[0] as usize;
                let mut keys=w[1..1+count].to_vec();
                let mut records=(0..count as u32).collect::<Vec<_>>();
                sort_model_shadow_records(&mut records,&mut keys).unwrap();
                assert_eq!(keys,w[1+count..1+count*2],"native paired sort keys {sorting}");
                assert_eq!(records,w[1+count*2..],"native tied-record order {sorting}");
                sorting+=1;
            }
        }
        assert_eq!((radial,sorting),(512,512));
    }

    #[test]
    fn model_caster_publication_matches_original_bounds_producer() {
        let mut count=0;
        for line in include_str!("../../analysis/original_model_caster_info_cases.txt").lines() {
            let w:Vec<u32>=line.split_whitespace().map(|s|u32::from_str_radix(s,16).unwrap()).collect();
            let bounds=std::array::from_fn(|r|std::array::from_fn(|c|f32::from_bits(w[11+r*3+c])));
            let surfaces=(0..4).map(|i|(w[3+i*2] as i32,w[4+i*2] as u8));
            let actual=prepare_model_caster_info(bounds,w[0] as u8,w[1] as u8,w[2] as u8,false,surfaces);
            assert_eq!(actual.flags,u8::try_from(w[17]).unwrap(),"native publication {count}");
            if let Some(sphere)=actual.sphere {
                assert_eq!(sphere.map(f32::to_bits),w[18..22],"native sphere {count}");
            } else { assert_eq!(w[18..22],[(-777.0f32).to_bits();4]); }
            count+=1;
        }
        assert_eq!(count,512);
    }

    #[test]
    fn collector_surface_distances_match_full_game_capture() {
        let mut count=0;
        for line in include_str!("../../analysis/original_model_caster_distance_cases.txt").lines() {
            let w:Vec<u32>=line.split_whitespace().map(|s|u32::from_str_radix(s,16).unwrap()).collect();
            let sphere=std::array::from_fn(|i|f32::from_bits(w[1+i]));
            let inverse=std::array::from_fn(|r|std::array::from_fn(|c|f32::from_bits(w[5+r*4+c])));
            let actual=model_caster_sort_distance(w[0] as u8,sphere,&inverse,f32::from_bits(w[21]));
            assert_eq!(actual.map(f32::to_bits),Some(w[22]),"native collected distance {count}");
            count+=1;
        }
        assert_eq!(count,104);
    }

    #[test]
    fn actor_caster_dispatch_matches_complete_original_traversal() {
        #[derive(Clone)]
        struct Actor {
            flags: u8, primary: Option<usize>, extra: Vec<usize>, children: Vec<usize>,
        }
        #[derive(Clone)]
        struct Graph {
            actors: Vec<Actor>, flags: Vec<u8>, other: Vec<u8>, calls: Vec<(u32, usize, u32)>,
        }
        impl ActorCasterAccess for Graph {
            fn actor_flags_100(&self, a: usize) -> u8 { self.actors[a].flags }
            fn primary_model(&self, a: usize) -> Option<usize> { self.actors[a].primary }
            fn additional_model_count(&self, a: usize) -> usize { self.actors[a].extra.len() }
            fn additional_model(&self, a: usize, i: usize) -> usize { self.actors[a].extra[i] }
            fn child_actor_count(&self, a: usize) -> usize { self.actors[a].children.len() }
            fn child_actor(&self, a: usize, i: usize) -> usize { self.actors[a].children[i] }
            fn model_flags_18bc(&self, m: usize) -> u8 { self.flags[m] }
            fn model_flags_1c30(&self, m: usize) -> u8 { self.other[m] }
            fn model_slot68(&mut self, m: usize, v: u8) {
                self.calls.push((0, m, u32::from(v)));
                set_model_dynamic_caster_flag(&mut self.flags[m], v);
            }
            fn model_slot28(&mut self, m: usize, v: u32) { self.calls.push((1, m, v)); }
        }
        let mut count = 0;
        for (case, line) in include_str!("../../analysis/original_actor_caster_flags_cases.txt").lines().enumerate() {
            let mut words = line.split_whitespace().map(|s| u32::from_str_radix(s, 16).unwrap());
            let mut next = || words.next().unwrap();
            let value = next() as u8;
            let nmodels = next() as usize;
            let nactors = next() as usize;
            let flags = (0..nmodels).map(|_| next() as u8).collect();
            let other = (0..nmodels).map(|_| next() as u8).collect();
            let actors = (0..nactors).map(|_| {
                let flags = next() as u8;
                let primary = next();
                let n = next() as usize;
                let extra = (0..n).map(|_| next() as usize).collect();
                let n = next() as usize;
                let children = (0..n).map(|_| next() as usize).collect();
                Actor { flags, primary: (primary != u32::MAX).then_some(primary as usize), extra, children }
            }).collect();
            let expected_changed = next() != 0;
            let expected_flags: Vec<_> = (0..nmodels).map(|_| next() as u8).collect();
            let n = next() as usize;
            let expected_calls: Vec<_> = (0..n).map(|_| (next(), next() as usize, next())).collect();
            assert!(words.next().is_none());
            let mut graph = Graph { actors, flags, other, calls: Vec::new() };
            if value<=1 {
                let mut owned=graph.clone();let mut controls=LocalShadowCacheControls::default();
                assert_eq!(set_actors_dynamic_caster_for_view(&mut owned,[None,Some(0)],value!=0,&mut controls),expected_changed);
                assert_eq!(owned.flags,expected_flags);
                assert_eq!(owned.calls,expected_calls);
                assert_eq!(controls,LocalShadowCacheControls {
                    discard_models:expected_changed && value==0,clear_all:expected_changed && value==1,
                });
            }
            assert_eq!(set_actor_dynamic_caster(&mut graph, 0, value), expected_changed, "case {case}");
            assert_eq!(graph.flags, expected_flags, "case {case}");
            assert_eq!(graph.calls, expected_calls, "case {case}");
            count += 1;
        }
        assert_eq!(count, 256);
    }

    #[test]
    fn model_caster_owner_flags_match_original_setters_and_child_dispatch() {
        assert!(!model_is_dynamic_caster(INITIAL_MODEL_CONTEXT_FLAGS_18BC));
        let mut leaf_count = 0;
        let mut parent_count = 0;
        for line in include_str!("../../analysis/original_model_caster_flags_cases.txt").lines() {
            let mut fields = line.split_whitespace();
            let kind = fields.next().unwrap();
            let mut data = fields.map(|value| u8::from_str_radix(value, 16).unwrap());
            let mut owner = data.next().unwrap();
            let value = data.next().unwrap();
            let expected_owner = data.next().unwrap();
            if kind == "L" {
                set_model_dynamic_caster_flag(&mut owner, value);
                assert!(data.next().is_none());
                leaf_count += 1;
            } else {
                assert_eq!(kind, "C");
                let count = data.next().unwrap() as usize;
                let mut child_flags: Vec<_> = data.by_ref().take(count).collect();
                let expected_children: Vec<_> = data.by_ref().take(count).collect();
                let call_count = data.next().unwrap() as usize;
                let expected_calls: Vec<_> = (0..call_count)
                    .map(|_| (data.next().unwrap() as usize, data.next().unwrap())).collect();
                assert!(data.next().is_none());
                let children: Vec<_> = (0..count).collect();
                let mut calls = Vec::new();
                set_model_dynamic_caster_with_children(&mut owner, value, &children, |child, argument| {
                    calls.push((child, argument));
                    set_model_dynamic_caster_flag(&mut child_flags[child], argument);
                });
                assert_eq!(calls, expected_calls);
                assert_eq!(child_flags, expected_children);
                parent_count += 1;
            }
            assert_eq!(owner, expected_owner);
            assert_eq!(model_is_dynamic_caster(owner), value & 1 != 0);
        }
        assert_eq!((leaf_count, parent_count), (2048, 256));
    }
}
