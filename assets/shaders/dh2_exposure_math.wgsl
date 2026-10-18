struct Settings {range:vec4<f32>,adaptation:vec4<f32>,camera:vec4<f32>}
struct State {engine:f32,luminance:f32,min_ldr:f32,max_ldr:f32,middle:f32,ev:f32,fstop:f32,peak:u32}
fn bucket(l:f32)->f32 {return (8.685889638065036*log(l)+40.0)*(128.0/140.0);}
fn luminance(b:f32)->f32 {return pow(10.0,0.05*(-40.0+b*(140.0/128.0)));}
fn native_adapt(h:array<u32,128>,p:Settings,last:f32,dt:f32)->State {
 var result:State;
 let monitor=(8.685889638065036*log(255.0*p.adaptation.y))/(140.0/128.0);
 let clip=vec2<u32>(clamp(p.range.zw,vec2(0.0),vec2(1.0))*monitor);
 var integral=0u;var peak=0u;for(var i=clip.x;i<clip.y;i++){integral+=h[i];peak=max(peak,h[i]);}
 var best=integral;var best_tail=clip.x;var tail=clip.x;var head=clip.y;
 let end=128u-u32(monitor-f32(clip.y));
 loop {if head>=end {break;}integral-=h[tail];tail++;integral+=h[head];peak=max(peak,h[head]);head++;
  if integral>=best {best=integral;best_tail=tail;}}
 if best==0u {best_tail=u32((128.0-monitor)/2.0);}
 let lo=bucket(p.range.x);let hi=bucket(p.range.y);
 let b=clamp(f32(best_tail),min(lo,hi-monitor-1.0),hi-monitor);
 let target_luminance=luminance(b)*exp2(-(p.adaptation.x+p.camera.z));

 var adapted=0.0;
 if last<target_luminance {adapted=mix(target_luminance,last,pow(abs(1.0-p.adaptation.z),dt));}
 else {adapted=1.0/mix(1.0/max(1e-4,target_luminance),1.0/max(1e-4,last),pow(abs(1.0-p.adaptation.w),dt));}
 result.luminance=adapted;result.min_ldr=adapted;result.max_ldr=adapted*p.adaptation.y*255.0;
 result.middle=adapted*55.497598410127914;let pow_ev=result.middle/0.15;
 result.ev=log2(pow_ev);result.fstop=sqrt(pow_ev/p.camera.x)+p.camera.y;result.peak=peak;
 result.engine=139.26/(adapted*255.0);
 return result;
}
