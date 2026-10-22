#define_import_path dh2_reflection
#import dh2_indirect::{probe_sh,game_vector}

@group(#{MATERIAL_BIND_GROUP}) @binding(156) var reflection_cubes:texture_cube_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(157) var reflection_sampler:sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(158) var<storage,read> reflection_planes:array<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(159) var reflection_brdf_lookup:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(160) var reflection_brdf_sampler:sampler;

// brdf_ward::AnalyticalBRDFIntegral_Order3, including scale/bias saturation.
fn integrated_ward(v_dot_n:f32,roughness:f32)->vec2<f32> {
    let x=clamp(v_dot_n,0.0,1.0);let y=clamp(roughness,0.0,1.0);
    let a=vec2(4.483120094766527,-4.268373376557555);
    let b=vec2(-6.607418162702069,5.9893822170455895);
    let c=vec2(3.1369146126741314,-2.745876982205202);
    let d=vec2(0.31965568814498413,-1.1169779635363377);
    let e=vec2(-4.171560969733041,6.2485832361009965);
    let f=vec2(10.579256413997497,-10.359824418695426);
    let g=vec2(-6.407181973132758,5.296592577623529);
    let h=vec2(1.4812238752410947,0.08005589283505338);
    let i=vec2(-5.679180293057831,-4.057878478465465);
    let j=vec2(-0.17354724317745246,10.024587584890986);
    let k=vec2(2.9060079735435256,-6.183779761950312);
    let l=vec2(-1.1483095194551398,0.3075771997708405);
    let m=vec2(5.289774653050149,0.8965992421669212);
    let n=vec2(-4.03337882134342,-3.9327120832393003);
    let o=vec2(0.496313678299834,2.8109277250321485);
    let result=x*(a+y*(e+y*(i+y*m))+x*(b+y*(f+y*(j+y*n))+x*(c+y*(g+y*(k+y*o)))))+y*(d+y*(h+y*l));
    return clamp(result+vec2(0.0,1.0),vec2(0.0),vec2(1.0));
}

fn reflection_direction(P:vec3<f32>,L:vec3<f32>,index:u32)->vec4<f32> {
    let base=4u+index*16u;
    let probe=bitcast<vec3<f32>>(vec3(reflection_planes[base+12u],reflection_planes[base+13u],reflection_planes[base+14u]));
    let local=P-probe;var distance=1e6;
    for(var plane=0u;plane<6u;plane++) {
        let packed=reflection_planes[base+plane*2u];
        let normal=vec3<f32>(vec3(packed&255u,(packed>>8u)&255u,(packed>>16u)&255u))/127.0-vec3(1.0);
        let offset=bitcast<f32>(reflection_planes[base+plane*2u+1u]);
        let t=-abs(dot(local,normal)+offset)/dot(L,normal);
        distance=min(distance,select(1e6,t,t>0.0));
    }
    let hit=P+distance*L;
    let ratio=length(hit-probe)/(1e-3+length(hit-P));
    let luminance=min(2.0,ratio*ratio);
    let direction=normalize(hit-probe);
    return vec4(direction,luminance);
}

fn reflection_radiance(P:vec3<f32>,L:vec3<f32>,index:u32,mip:f32)->vec3<f32> {
    let corrected=reflection_direction(P,L,index);
    return corrected.w*textureSampleLevel(reflection_cubes,reflection_sampler,corrected.xyz,i32(index),mip).rgb;
}

fn reflected_light(tag:u32,position:vec3<f32>,normal:vec3<f32>,view:vec3<f32>,surface_roughness:f32,f0:vec3<f32>,vegetation:bool,
    indices_gain:vec4<f32>,bary_xy:vec2<f32>)->vec3<f32> {
    let count=reflection_planes[0];
    if count==0u {return vec3(0.0);}
    let per_vertex=(tag&0x80000000u)!=0u;
    var gain=indices_gain.w;
    var index=0u;
    if !per_vertex {
        gain=probe_sh[tag][1].w;
        let encoded=probe_sh[tag][0].w;
        let requested=u32(max(encoded,0.0));
        index=select(count-1u,requested,encoded>=0.0 && requested<count);
    }
    if gain<=0.0 {return vec3(0.0);}
    let P=game_vector(position);let N=game_vector(normal);let V=game_vector(view);
    // ComputeIBL squares the artist-facing roughness before its HQ evaluator.
    let roughness=surface_roughness*surface_roughness;
    let L=normalize(mix(reflect(-V,N),N,roughness));
    let aperture=0.0003474660443456835+roughness*(1.3331290497744692-roughness*0.5040552688878546);
    let mip=6.0*sqrt(aperture/0.834);
    var radiance:vec3<f32>;
    if per_vertex {
        let bary=vec3(bary_xy,1.0-bary_xy.x-bary_xy.y);
        let ids=vec3<u32>(select(indices_gain.xyz,round(indices_gain.xyz/bary),bary>vec3(0.0)));
        radiance=reflection_radiance(P,L,ids.x,mip);
        if ids.y!=ids.x || ids.z!=ids.x {
            radiance=bary.x*radiance
                +bary.y*reflection_radiance(P,L,ids.y,mip)
                +bary.z*reflection_radiance(P,L,ids.z,mip);
        }
    } else {radiance=reflection_radiance(P,L,index,mip);}
    let bounce=mix(gain,1.0,dot(vec3(0.2126,0.7152,0.0722),f0));
    var brdf=integrated_ward(dot(V,N),roughness);
    if vegetation {brdf=textureSampleLevel(reflection_brdf_lookup,reflection_brdf_sampler,vec2(clamp(dot(V,N),0.0,1.0),roughness),0.0).rg;}
    return radiance*bounce*(f0*brdf.x+brdf.y*(1.0-roughness*roughness));
}
