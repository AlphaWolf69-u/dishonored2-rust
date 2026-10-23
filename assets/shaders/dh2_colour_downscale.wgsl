// PostFX_Downscale. The caller supplies 34f840's actual source-size offsets.
struct DownscaleParameters {
    texel:vec4<f32>,
    colour:vec4<f32>,
    limits:vec4<f32>, // adapt_max_luminance, EngineLuminanceFactor, white level
    options:vec4<u32>, // TYPE, useCompressedHdrBuffers
}
@group(0) @binding(0) var<uniform> p:DownscaleParameters;
@group(0) @binding(1) var image:texture_2d<f32>;
@group(0) @binding(2) var bilinear_clamp:sampler;
@group(0) @binding(3) var point_clamp:sampler;

fn downscale_taps(uv0:vec2<f32>,uv1:vec2<f32>,uv2:vec2<f32>,uv3:vec2<f32>)->vec4<f32> {
    let mode=p.options.x;
    if(mode==5u) {
        let clear=hdr_range_compress(p.colour.rgb,p.options.y!=0u,downscale_engine_luminance(),p.limits.z);
        var a=downscale_source(uv0,point_clamp).rgb;
        var b=downscale_source(uv1,point_clamp).rgb;
        var c=downscale_source(uv2,point_clamp).rgb;
        var e=downscale_source(uv3,point_clamp).rgb;
        let da=clear-a;let db=clear-b;let dc=clear-c;let de=clear-e;
        let va=dot(da,da)>0.0;let vb=dot(db,db)>0.0;
        let vc=dot(dc,dc)>0.0;let ve=dot(de,de)>0.0;
        // Sequential assignments are intentional: later replacements use the
        // already replaced earlier colours, as in the source shader.
        a=select(select(select(select(vec3(0.0),e,ve),c,vc),b,vb),a,va);
        b=select(select(select(select(vec3(0.0),e,ve),c,vc),a,va),b,vb);
        c=select(select(select(select(vec3(0.0),e,ve),a,va),b,vb),c,vc);
        e=select(select(select(select(vec3(0.0),a,va),c,vc),b,vb),e,ve);
        return vec4((a+b+c+e)*0.25,0.0);
    }
    if(mode==3u) {
        let a=downscale_source(uv0,point_clamp).rg;
        let b=downscale_source(uv1,point_clamp).rg;
        let c=downscale_source(uv2,point_clamp).rg;
        let e=downscale_source(uv3,point_clamp).rg;
        let result=min(min(a,c),min(b,e));
        return result.xyxy;
    }
    let a=downscale_source(uv0,bilinear_clamp);
    let b=downscale_source(uv1,bilinear_clamp);
    let c=downscale_source(uv2,bilinear_clamp);
    let e=downscale_source(uv3,bilinear_clamp);
    if(mode==1u) {return min(min(a,b),min(c,e));}
    if(mode==2u) {return max(max(a,b),max(c,e));}
    if(mode==4u) {return vec4((a.rgb+b.rgb+c.rgb+e.rgb)*0.25,0.0);}
    if(mode==6u) {return vec4(clamp((a.rgb+b.rgb+c.rgb+e.rgb)*0.25,vec3(0.0),vec3(p.limits.x)),0.0);}
    return (a+b+c+e)*0.25;
}
fn downscale_colour(uv:vec2<f32>)->vec4<f32> {
    let d=p.texel.xy;
    return downscale_taps(uv-d,uv+vec2(d.x,-d.y),uv+vec2(-d.x,d.y),uv+d);
}
struct DownscaleVertex {@builtin(position) position:vec4<f32>,@location(0) uv:vec2<f32>,}
@vertex fn downscale_vertex(@builtin(vertex_index) i:u32)->DownscaleVertex {
    let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));
    return DownscaleVertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),uv);
}
@fragment fn downscale_fragment(v:DownscaleVertex)->@location(0) vec4<f32> {return downscale_colour(v.uv);}
// Preserve the installed VP's tap offsets before interpolation when the view
// owner supplies its actual fullscreen geometry and texture coordinates.
struct DownscaleMeshVertex {
    @builtin(position)position:vec4<f32>,
    @location(0)first:vec4<f32>,
    @location(1)second:vec4<f32>,
}
@vertex fn downscale_mesh_vertex(@location(0)position:vec3<f32>,@location(1)uv:vec2<f32>)->DownscaleMeshVertex {
    return DownscaleMeshVertex(vec4(position,1.0),
        vec4(uv+vec2(-1.0,-1.0)*p.texel.xy,uv+vec2(1.0,-1.0)*p.texel.xy),
        vec4(uv+vec2(-1.0,1.0)*p.texel.xy,uv+vec2(1.0,1.0)*p.texel.xy));
}
@fragment fn downscale_mesh_fragment(v:DownscaleMeshVertex)->@location(0)vec4<f32> {
    return downscale_taps(v.first.xy,v.first.zw,v.second.xy,v.second.zw);
}
