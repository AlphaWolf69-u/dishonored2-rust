#import bevy_render::view::View
#import bevy_render::globals::Globals
@group(0) @binding(0) var<uniform> globals:Globals;
@group(0) @binding(1) var<uniform> settings:Settings;
@group(0) @binding(2) var hdr:texture_2d<f32>;
@group(0) @binding(3) var<storage,read_write> bins:array<atomic<u32>,128>;
@group(0) @binding(4) var<storage,read_write> state:State;
@group(0) @binding(5) var<storage,read_write> view:View;
var<workgroup> local_bins:array<atomic<u32>,128>;
@compute @workgroup_size(16,16,1)
fn histogram(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
 if lane<128u {atomicStore(&local_bins[lane],0u);}workgroupBarrier();
 let dim=textureDimensions(hdr);
 if all(id.xy<dim) {
  let l=textureLoad(hdr,vec2<i32>(id.xy),0).w*139.26;
  if l>=0.01 {atomicAdd(&local_bins[u32(clamp(floor(bucket(l)),0.0,127.0))],1u);}
 }
 workgroupBarrier();if lane<128u {atomicAdd(&bins[lane],atomicLoad(&local_bins[lane]));}
}
@compute @workgroup_size(1,1,1)
fn update_exposure() {
 var h:array<u32,128>;for(var i=0u;i<128u;i++){h[i]=atomicExchange(&bins[i],0u);}
 state=adapt(h,settings,state.luminance,clamp(globals.delta_time,0.01,1.0));
 // Bevy's HDR target already includes the fixed camera exposure. Replace it
 // with the native world-luminance exposure in the final grading stage.
 view.color_grading.exposure+=log2(1.0/(state.luminance*255.0*view.exposure));
}
