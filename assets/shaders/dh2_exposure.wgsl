#import bevy_render::view::View
#import bevy_render::globals::Globals
@group(0) @binding(0) var<uniform> globals:Globals;
@group(0) @binding(1) var<uniform> settings:Settings;
@group(0) @binding(2) var hdr:texture_2d<f32>;
@group(0) @binding(3) var<storage,read_write> bins:array<atomic<u32>,128>;
@group(0) @binding(4) var<storage,read_write> state:State;
@group(0) @binding(5) var<storage,read_write> view:View;
var<workgroup> local_bins:array<atomic<u32>,128>;
// Original first half-resolution bilinear downsample stores linear RGBA16F.
fn half_pixel(p:vec2<i32>)->vec4<f32> {
 let dim=vec2<i32>((textureDimensions(hdr)+vec2(1u))/2u);
 let uv=(vec2<f32>(clamp(p,vec2(0),dim-vec2(1)))+vec2(0.5))/vec2<f32>(dim);
 let q=uv*vec2<f32>(textureDimensions(hdr))-vec2(0.5);let i=vec2<i32>(floor(q));let f=fract(q);
 let hi=vec2<i32>(textureDimensions(hdr))-vec2(1);
 let a=textureLoad(hdr,clamp(i,vec2(0),hi),0).rgb;
 let b=textureLoad(hdr,clamp(i+vec2(1,0),vec2(0),hi),0).rgb;
 let c=textureLoad(hdr,clamp(i+vec2(0,1),vec2(0),hi),0).rgb;
 let d=textureLoad(hdr,clamp(i+vec2(1,1),vec2(0),hi),0).rgb;
 let rgb=max(vec3(0.0),mix(mix(a,b,f.x),mix(c,d,f.x),f.y)/view.exposure);
 // Native HDR is normalized before its FP16 downsample; convert back to world
 // only at histogram insertion, preserving that intermediate rounding.
 let n=rgb/139.26;
 let xy=unpack2x16float(pack2x16float(n.xy));let zw=unpack2x16float(pack2x16float(vec2(n.z,dot(n,vec3(0.2126,0.7152,0.0722)))));
 return vec4(xy,zw);
}
fn sample_half(uv:vec2<f32>)->vec4<f32> {
 let dim=vec2<f32>((textureDimensions(hdr)+vec2(1u))/2u);
 let q=uv*dim-vec2(0.5);let i=vec2<i32>(floor(q));let f=fract(q);
 return mix(mix(half_pixel(i),half_pixel(i+vec2(1,0)),f.x),mix(half_pixel(i+vec2(0,1)),half_pixel(i+vec2(1,1)),f.x),f.y);
}
@compute @workgroup_size(16,16,1)
fn histogram(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
 if lane<128u {atomicStore(&local_bins[lane],0u);}workgroupBarrier();
 let dim=((textureDimensions(hdr)+vec2(1u))/2u+vec2(1u))/2u;
 if all(id.xy<dim) {
  let uv=(vec2<f32>(id.xy)+vec2(0.5))/vec2<f32>(dim);let d=vec2(0.5)/vec2<f32>(dim);
  let n=0.25*(sample_half(uv-d)+sample_half(uv+vec2(d.x,-d.y))+sample_half(uv+vec2(-d.x,d.y))+sample_half(uv+d));
  let native_lum=n.w;
  let l=unpack2x16float(pack2x16float(vec2(native_lum,0.0))).x*139.26;
  if l>=0.01 {atomicAdd(&local_bins[u32(clamp(floor(bucket(l)),0.0,127.0))],1u);}
 }
 workgroupBarrier();if lane<128u {atomicAdd(&bins[lane],atomicLoad(&local_bins[lane]));}
}
@compute @workgroup_size(1,1,1)
fn adapt() {
 var h:array<u32,128>;for(var i=0u;i<128u;i++){h[i]=atomicExchange(&bins[i],0u);}
 state=native_adapt(h,settings,state.luminance,clamp(globals.delta_time,0.01,1.0));
 // Bevy's HDR target already includes the fixed camera exposure. Replace it
 // with the native world-luminance exposure in the final grading stage.
 view.color_grading.exposure+=log2(1.0/(state.luminance*255.0*view.exposure));
}
