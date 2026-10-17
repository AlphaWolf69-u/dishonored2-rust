//! Native static-world AABB tree and28c4e0 local-light caster selection.
//! Node overlap is deliberately coarse; individual records test the clip
//! volume. Testing each record against the query box changes native admission.
use crate::{render_model,world_mesh,tome,render_shadow};

///232e50's retained query inputs. Local precomputation uses task-pool jobs,
///not the ordinary camera's configurable query-job count.
pub struct StaticLightCasterQueries<'a> {
    pub bytes:&'a [u8],pub tree:&'a tome::SpatialTree,pub data:&'a tome::DirectTomeQueryData,
    pub gates:&'a [u32],pub world:&'a world_mesh::WorldMesh,pub job_count:u32,
    pub object_imprecision:f32,pub accurate_threshold:f32,
}

impl StaticLightCasterQueries<'_> {
    pub fn query_light(&self,position:[f32;3],clips:&[render_shadow::ShadowMatrix])->Result<Vec<Vec<u32>>,String> {
        let mut mask=vec![false;self.world.visibility.len()];let mut faces=Vec::with_capacity(clips.len());
        for &clip in clips {
            let input=tome::OrdinaryDirectQueryInput {camera_point:position,render_rows:clip,cropped_rows:clip,
                initial_tile_rect:[0,0,8,16],camera_imprecision:[self.object_imprecision;3],object_imprecision:self.object_imprecision,
                minimum_area_metric:0.,global_distance_threshold:self.accurate_threshold,object_distance_scale:f32::MIN_POSITIVE,
                winding_flip:tome::query_matrix_winding_flip(clip),gate_states:Some(self.gates),output_capacity:tome::ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY};
            let jobs=(0..self.job_count).map(|job|tome::ordinary_direct_tome_render_job(self.tree,self.bytes,self.data,&input,job,self.job_count)).collect::<Vec<_>>();
            if let Some(error)=jobs.iter().find_map(|job|job.diagnostics.error.as_ref()) {
                return Err(format!("local shadow Tome query is unresolved: {error}"));
            }
            let output=tome::merge_render_query_outputs(&jobs)?.ok_or("local shadow Tome query produced no descriptor")?;
            for &id in output.indices() {
                for &instance in self.world.tome_object_instances.get(id as usize).ok_or("local shadow Tome object has no WORLD mapping row")? {
                    *mask.get_mut(instance).ok_or("local shadow Tome mapping exceeds WORLD instances")?=true;
                }
            }
            //37ef90 overwrites spatial-query instances, while37e1d0 only
            //sets ordinary instances.232e50 preserves this mask across faces.
            for &instance in &self.world.spatial_query_instances {
                mask[instance]=tome::render_query_aabb_visible_without_depth(output.state.as_ref(),self.world.visibility[instance].bounds)?;
            }
            faces.push(mask.iter().enumerate().filter_map(|(i,&enabled)|enabled.then_some(i as u32)).collect());
        }
        Ok(faces)
    }
}

#[derive(Debug)]
struct Node {
    range:std::ops::Range<usize>,bounds:[[f32;3];2],
    left:Option<Box<Node>>,right:Option<Box<Node>>,
}

pub struct NativeLocalWorldQuery {
    bounds:Vec<[[f32;3];2]>,ids:Vec<u32>,root:Option<Box<Node>>,
}

impl NativeLocalWorldQuery {
    ///295fa0 builds293580/293800 with leaf threshold8, in visibility order.
    pub fn new(world:&world_mesh::WorldMesh)->Self {
        Self::from_bounds(world.visibility.iter().map(|record|record.bounds).collect())
    }

    fn from_bounds(bounds:Vec<[[f32;3];2]>)->Self {
        let mut ids=(0..bounds.len() as u32).collect::<Vec<_>>();
        let end=ids.len();
        let root=if end==0 {None}else{Some(Box::new(build(&bounds,&mut ids,0..end)))};
        Self {bounds,ids,root}
    }

    ///2956e0/28c760 clears its mask, then queries the retained native tree.
    pub fn query(&self,bounds:[[f32;3];2],clip:&[[f32;4];4])->Vec<u8> {
        let mut mask=vec![0;self.bounds.len()];
        let clip=std::array::from_fn(|i|clip[i/4][i%4]);
        if let Some(root)=&self.root {self.visit(root,bounds,&clip,&mut mask);}
        mask
    }

    fn visit(&self,node:&Node,bounds:[[f32;3];2],clip:&[f32;16],mask:&mut [u8]) {
        if !(0..3).all(|axis|bounds[0][axis]<=node.bounds[1][axis]+0.0 && node.bounds[0][axis]-0.0<=bounds[1][axis]) {return;}
        for &id in &self.ids[node.range.clone()] {
            let b=self.bounds[id as usize];
            if !render_model::native_transformed_aabb_clip_cull(clip,b[0],b[1]).culled {mask[id as usize]=1;}
        }
        if let Some(left)=&node.left {self.visit(left,bounds,clip,mask);}
        if let Some(right)=&node.right {self.visit(right,bounds,clip,mask);}
    }
}

fn build(bounds:&[[[f32;3];2]],ids:&mut [u32],range:std::ops::Range<usize>)->Node {
    let mut total=[[f32::from_bits(0x7149f2ca);3],[f32::from_bits(0xf149f2ca);3]];
    for &id in &ids[range.clone()] {for axis in 0..3 {
        let b=bounds[id as usize];
        if !(total[0][axis]<b[0][axis]) {total[0][axis]=b[0][axis];}
        if !(total[1][axis]>b[1][axis]) {total[1][axis]=b[1][axis];}
    }}
    let mut node=Node {range:range.clone(),bounds:total,left:None,right:None};
    if range.len()<=8 {ids[range].sort_unstable();return node;}
    let size:[f32;3]=std::array::from_fn(|axis|total[1][axis]-total[0][axis]);
    let threshold=((size[1]*size[0])*size[2])*f32::from_bits(0x3e5d2f1c);
    let mut retained=range.start;
    for at in range.clone() {
        let b=bounds[ids[at] as usize];
        let volume=if b[1][0]<=b[0][0] || b[1][1]<=b[0][1] || b[1][2]<=b[0][2] {0.0}
            else {((b[1][1]-b[0][1])*(b[1][0]-b[0][0]))*(b[1][2]-b[0][2])};
        if threshold<=volume {ids.swap(retained,at);retained+=1;}
    }
    let axis=if size[1]<=size[0] {if size[0]<size[2] {2}else{0}}else{if size[1]<size[2] {2}else{1}};
    let mut split=range.end;
    if range.end-retained>1 {
        let mut pairs=ids[retained..range.end].iter().map(|&id| {
            let b=bounds[id as usize];((b[0][axis]+b[1][axis])*0.5,id)
        }).collect::<Vec<_>>();
        sort_positions(&mut pairs);
        let center=(total[1][axis]+total[0][axis])*0.5;
        split=retained;
        for (at,(position,id)) in pairs.into_iter().enumerate() {
            ids[retained+at]=id;
            if position<center {split+=1;}
        }
        //Native promotes the first sorted record into the parent if the
        //left child would otherwise be empty, rather than forcing a split.
        if split==retained {retained+=1;split+=1;}
    }
    ids[range.start..retained].sort_unstable();node.range.end=retained;
    if retained<split {node.left=Some(Box::new(build(bounds,ids,retained..split)));}
    if split<range.end {node.right=Some(Box::new(build(bounds,ids,split..range.end)));}
    node
}

///298b80 sorts position/id pairs, including its exact equal-key permutation.
fn sort_positions(pairs:&mut [(f32,u32)]) {
    if pairs.len()<2 {return;}
    let mut work=Vec::new();let (mut low,mut high)=(0,pairs.len()-1);
    loop {
        if high-low>3 && work.len()<127 {
            pairs.swap((low+high)/2,high);
            let pivot=pairs[high].0;let (mut left,mut right)=(low,high-1);
            loop {
                while left<right && !(pivot<=pairs[left].0) {left+=1;}
                while right>left && !(pairs[right].0<=pivot) {right-=1;}
                if right<=left {break;}
                pairs.swap(left,right);left+=1;right-=1;
                if left>=right {break;}
            }
            while right>low && pivot<=pairs[right].0 {right-=1;}
            while left<high && pairs[left].0<=pivot {left+=1;}
            pairs.swap(high,left);
            if right-low<high-left {work.push((left,high));high=right;}
            else {work.push((low,right));low=left;}
            continue;
        }
        while low<high {
            let mut largest=low;
            for at in low+1..=high {if pairs[largest].0<pairs[at].0 {largest=at;}}
            pairs.swap(largest,high);high-=1;
        }
        let Some(next)=work.pop() else {break;};(low,high)=next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn serialize(node:&Node,out:&mut Vec<u32>) {
        out.extend([node.range.start as u32,node.range.end as u32]);
        for b in node.bounds {out.extend(b.map(f32::to_bits));out.push(0);}
        out.extend([u32::from(node.left.is_some()),u32::from(node.right.is_some())]);
        if let Some(left)=&node.left {serialize(left,out);}
        if let Some(right)=&node.right {serialize(right,out);}
    }
    #[test]
    fn native_world_tree_partition_and_local_masks_match_original() {
        for (case,line) in include_str!("../analysis/original_local_world_tree_cases.txt").lines().enumerate() {
            let (input,expected)=line.split_once('|').unwrap();
            let words=input.split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap()).collect::<Vec<_>>();
            let expected=expected.split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap()).collect::<Vec<_>>();
            let count=words[1] as usize;let bounds=(0..count).map(|i|std::array::from_fn(|r|std::array::from_fn(|c|f32::from_bits(words[2+i*6+r*3+c])))).collect();
            let tree=NativeLocalWorldQuery::from_bounds(bounds);let mut actual=tree.ids.clone();serialize(tree.root.as_ref().unwrap(),&mut actual);
            assert_eq!(actual,expected[..expected.len()-count],"original WORLD tree case{case}");
            let offset=2+count*6;
            let clip=std::array::from_fn(|r|std::array::from_fn(|c|f32::from_bits(words[offset+r*4+c])));
            let bounds=std::array::from_fn(|r|std::array::from_fn(|c|f32::from_bits(words[offset+16+r*4+c])));
            assert_eq!(tree.query(bounds,&clip).iter().map(|&v|u32::from(v)).collect::<Vec<_>>(),expected[expected.len()-count..],"original local WORLD mask case{case}");
        }
    }
}
