// Dark Vision device and pickup layers (ARK_RENDERPASS_DARKVISION,
// 14031f960): a depth prepass of the layer's surfaces into a cleared target,
// then rendering/darkvision additively into the scene where that depth is
// equal. Rewrite-authored from the traced behaviour of the installed program.
// Positions arrive in Bevy axes and are converted to game axes here.
struct LayerFrame {
    view_projection:mat4x4<f32>, // game axes, scene depth convention
    camera_bias:vec4<f32>,       // camera position (game), depth of a point at darkVisionDepthBias
    reveal:vec4<f32>,            // revealWaveRadius (inner, outer), revealWavePower, host units
    centre_white:vec4<f32>,      // darkVisionWaveCenter, view white level
    flags:vec4<f32>,             // compressed HDR (1/0), target width, height, 0
}
struct LayerDraw {
    model:mat4x4<f32>,           // Bevy world transform
    normal:mat4x4<f32>,          // inverse transpose of model
    tint_opacity:vec4<f32>,      // layer tint, layer opacity
    clip_hack:vec4<f32>,         // darkVisionDepthClip, modeldepthhack
}
@group(0) @binding(0) var<uniform> frame:LayerFrame;
@group(0) @binding(1) var<uniform> draw:LayerDraw;
@group(0) @binding(2) var colour_in:texture_2d<f32>;
@group(0) @binding(3) var scene_depth:texture_depth_2d;
@group(0) @binding(4) var<storage,read> adaptation:array<vec4<f32>,2>;

struct Varyings {
    @builtin(position) position:vec4<f32>,
    @location(0) world:vec3<f32>,
    @location(1) normal:vec3<f32>,
}

fn game_axes(v:vec3<f32>)->vec3<f32> { return vec3(v.x,-v.z,v.y); }

fn project(position:vec3<f32>)->Varyings {
    let world=game_axes((draw.model*vec4(position,1.0)).xyz);
    var clip=frame.view_projection*vec4(world,1.0);
    clip.z=clip.z*draw.clip_hack.y;
    var out:Varyings;
    out.position=clip;
    out.world=world;
    return out;
}

@vertex fn dark_vision_depth_vertex(@location(0) position:vec3<f32>)->@builtin(position) vec4<f32> {
    return project(position).position;
}

@vertex fn dark_vision_layer_vertex(@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>)->Varyings {
    var out=project(position);
    out.normal=normalize(game_axes((draw.normal*vec4(normal,0.0)).xyz));
    return out;
}

@fragment fn dark_vision_layer_fragment(in:Varyings)->@location(0) vec4<f32> {
    // Only inside the layer's culling distance from the pulse centre.
    let from_centre=distance(frame.centre_white.xyz,in.world);
    if draw.clip_hack.x-from_centre<0.0 {discard;}
    // Fades out across the reveal wave's band.
    let band=clamp((from_centre-frame.reveal.x)/(frame.reveal.y-frame.reveal.x),0.0,1.0);
    let weight=(1.0-band*band*(3.0-2.0*band))*frame.reveal.z;
    if weight-0.001<0.0 {discard;}
    let facing=dot(normalize(in.normal),normalize(frame.camera_bias.xyz-in.world));
    let size=vec2<i32>(textureDimensions(colour_in));
    let texel=clamp(vec2<i32>(in.position.xy),vec2(0),size-vec2(1));
    let exposure=adaptation[0].x;
    // The scene target holds host units; the original reads its own units,
    // scaled back to light when the HDR target is compressed.
    var scene=textureLoad(colour_in,texel,0).rgb/frame.reveal.w;
    if frame.flags.x!=0.0 {scene=scene*exposure*frame.centre_white.w;}
    let tint=draw.tint_opacity.rgb;
    let lit=scene*facing*2.0+tint*(1.0-facing);
    // The surface glows with its tint where the scene depth plus the bias
    // term exceeds its own depth, otherwise it shows the lit scene.
    let glow=select(0.0,1.0,frame.camera_bias.w+textureLoad(scene_depth,texel,0)-in.position.z>0.0);
    let colour=(glow*tint*(1.2-facing)*3.5+lit*(1.0-glow))*weight/exposure;
    return vec4(colour*frame.reveal.w,draw.tint_opacity.w);
}
