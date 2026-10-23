// Autofocus from postfx_dof_autofocus, dispatched once as one 8x2 group.
struct FocusParameters {
 centre_resolution:vec4<f32>,
 texel_radius_far:vec4<f32>,
 speed:vec4<f32>,
 local_time:vec4<f32>,
}
@group(0)@binding(0)var<uniform> p:FocusParameters;
@group(0)@binding(1)var depth:texture_2d<f32>;
@group(0)@binding(2)var previous:texture_2d<f32>;
@group(0)@binding(3)var linear_clamp:sampler;
@group(0)@binding(4)var result:texture_storage_2d<rg32float,write>;
var<workgroup> focus_samples:array<f32,16>;
@compute @workgroup_size(8,2,1)
fn focus_update(@builtin(local_invocation_id)id:vec3<u32>){
 let radius=p.texel_radius_far.z+p.texel_radius_far.z*0.5*f32(id.y);
 let angle=f32(id.x)*(6.283185307179586476925286766559/8.0);
 let offset=vec2(cos(angle)*radius,sin(angle)*radius);
 let uv=(p.centre_resolution.xy+p.texel_radius_far.xy*offset)*p.centre_resolution.zw;
 let index=id.y*8u+id.x;
 focus_samples[index]=textureSampleLevel(depth,linear_clamp,uv,0.0).x;
 workgroupBarrier();
 if index==0u{
  let far=p.texel_radius_far.w;
  var current=textureSampleLevel(depth,linear_clamp,p.centre_resolution.xy*p.centre_resolution.zw,0.0).x;
  var average=vec2(far);
  for(var i=0u;i<8u;i++){average.x=min(average.x,focus_samples[i]);}
  for(var i=0u;i<8u;i++){average.y=min(average.y,focus_samples[8u+i]);}
  current=min(current,(average.x+average.y)*0.5);
  let previous=point_clamp_sample(previous,p.centre_resolution.xy*p.centre_resolution.zw).xy;
  let focus_target=select(previous.x,current,sign(previous.x-current)>=0.0);
  let focal_speed=select(p.speed.y,p.speed.x,focus_target>0.0);
  let fuzzy_speed=select(p.speed.x,p.speed.y,focus_target>0.0);
  let ramp=clamp((focus_target-p.speed.z)/(p.speed.w-p.speed.z),0.0,1.0);
  let sharpness=far*(ramp*ramp*(3.0-2.0*ramp));
  let frame_time=p.local_time.y*30.0;
  let last=min(far,previous.x+sharpness);
  let focus=mix(last,current,clamp(focal_speed*frame_time,0.0,1.0));
  let fuzzy_current=min(far,focus_target+sharpness);
  let fuzzy=mix(previous.y,fuzzy_current,clamp(fuzzy_speed*frame_time,0.0,1.0));
  textureStore(result,vec2(0),vec4(min(focus,fuzzy),max(focus,fuzzy),0.0,0.0));
 }
}
