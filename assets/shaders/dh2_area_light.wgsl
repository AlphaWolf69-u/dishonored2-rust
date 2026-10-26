// areaLight's rectangular portal clipping, projected lobes and Ward integral.
// Coordinates and TiledLightRecord fields remain in game world units.
struct AreaLobe { bounds:vec4<f32>, solid_angle:f32, }

fn area_rectangle_solid_angle(position:vec3<f32>,bounds:vec4<f32>)->f32 {
    let closest=clamp(position.xy,bounds.xy,bounds.zw);
    let ax=normalize(vec3(bounds.z,closest.y,-1e-2)-position).x;
    let bx=normalize(position-vec3(bounds.x,closest.y,-1e-2)).x;
    let ay=normalize(vec3(closest.x,bounds.w,-1e-2)-position).y;
    let by=normalize(position-vec3(closest.x,bounds.y,-1e-2)).y;
    let cx=sqrt(0.25*(1.0-ax)*(1.0+bx))+sqrt(0.25*(1.0+ax)*(1.0-bx));
    let cy=sqrt(0.25*(1.0-ay)*(1.0+by))+sqrt(0.25*(1.0+ay)*(1.0-by));
    return 3.14159265358979323846*(2.0-cx-cy);
}
fn area_clip_segment(first:vec3<f32>,last:vec3<f32>,position:vec3<f32>,normal:vec3<f32>)->vec3<f32> {
    let v=last-first;
    let d=dot(position-first,normal);
    let t=d/dot(v,normal);
    return mix(last,first+shadow_saturate(t-1e-3)*v,select(0.0,1.0,d>0.0 || t>=0.0));
}
fn area_clip_bounds(position:vec3<f32>,normal:vec3<f32>,size:vec2<f32>)->vec4<f32> {
    let x=size.x*select(1.0,-1.0,normal.x<0.0);
    let y=size.y*select(1.0,-1.0,normal.y<0.0);
    let corners=array(vec3(x,y,0.0),vec3(-x,y,0.0),vec3(-x,-y,0.0),vec3(x,-y,0.0));
    var low=vec2(100.0);var high=vec2(-100.0);
    for(var winding=0u;winding<2u;winding++) {
        var p=corners[0];
        for(var side=1u;side<=4u;side++) {
            let index=select(side%4u,(4u-side)%4u,winding==1u);
            p=area_clip_segment(p,corners[index],position,normal);
            low=min(low,p.xy);high=max(high,p.xy);
        }
    }
    return vec4(low,high);
}
fn area_diffuse_lobe(position:vec3<f32>,normal:vec3<f32>,projection:vec3<f32>,bounds:vec4<f32>,solid:f32)->AreaLobe {
    var intersection:array<vec2<f32>,2>;
    let portal=array(vec3(bounds.xy,0.0),vec3(bounds.zw,0.0));
    for(var corner=0u;corner<2u;corner++) {
        let direction=portal[corner]-projection-position;
        intersection[corner]=(position+(-position.z/direction.z)*direction).xy;
    }
    let perpendicular=normalize(vec3(normal.xy,1e-4));
    let line=vec3(-perpendicular.y,perpendicular.x,perpendicular.z);
    let along=vec2(dot(line.xy,intersection[0]),dot(line.xy,intersection[1]));
    let x=clamp(dot(line.xy,position.xy),min(along.x,along.y),max(along.x,along.y));
    let y=0.5*(dot(perpendicular.xy,intersection[0])+dot(perpendicular.xy,intersection[1]));
    let center=x*line+y*perpendicular;
    var incoming=center-position;
    let distance=length(incoming);
    incoming*=select(0.0,1.0/distance,distance>1e-6);
    return AreaLobe(vec4(intersection[0],intersection[1]),shadow_saturate(dot(normal,incoming))*solid);
}
fn area_specular_lobe(position:vec3<f32>,normal:vec3<f32>,view:vec3<f32>,roughness:f32,solid:f32)->AreaLobe {
    if !(view.z<0.0) { return AreaLobe(vec4(-100.0),0.0); }
    let angle=0.0003474660443456835+roughness*(1.3331290497744692-roughness*0.5040552688878546);
    let distance=-position.z/view.z;
    let hit=position+distance*view;
    let radius=vec2((sin(angle)/cos(angle))*distance);
    let grazing=shadow_saturate(1.0+view.z);
    let squared=grazing*grazing;
    return AreaLobe(vec4(hit.xy-radius,hit.xy+radius),
        shadow_saturate(dot(normal,view))*solid*(1.0-squared*squared));
}
fn area_sample(pool:texture_2d_array<f32>,trilinear:sampler,bounds:vec4<f32>,size:vec2<f32>,slice:u32)->vec3<f32> {
    let a=vec2(bounds.x,bounds.w)/size;
    let b=vec2(bounds.z,bounds.y)/size;
    let uv0=0.5*vec2(1.0+a.x,1.0-a.y);
    let uv1=0.5*vec2(1.0+b.x,1.0-b.y);
    let delta=0.5*(uv1-uv0);
    let mip=log2(1e-4+256.0*0.5*(delta.x+delta.y));
    let center=0.5*(uv0+uv1);
    let clamped=clamp(center,vec2(0.0),vec2(1.0));
    let attenuation=smoothstep(1.0,0.0,shadow_saturate(length((center-clamped)/(vec2(1e-3)+delta))));
    var colour=vec3(1.0);
    // Slice255 is the original untextured branch, not a missing-image fallback.
    if slice<255u {colour=textureSampleLevel(pool,trilinear,clamped,i32(slice),mip).rgb;}
    return attenuation*colour;
}
fn area_ward(surface:WardSurface,position:vec3<f32>,record:TiledLightRecord,visibility:f32,
    pool:texture_2d_array<f32>,trilinear:sampler)->DirectLighting {
    var result=DirectLighting(vec3(0.0),vec3(0.0));
    let delta=position-record.position;
    if !(dot(delta,record.direction)>0.0 && visibility>0.0) {return result;}
    let x=record.axis;let z=record.direction;let y=cross(z,x);
    let local=vec3(dot(delta,x),dot(delta,y),dot(delta,z));
    let n=vec3(dot(surface.normal,x),dot(surface.normal,y),dot(surface.normal,z));
    let reflection=reflect(-surface.view,surface.normal);
    let v=vec3(dot(reflection,x),dot(reflection,y),dot(reflection,z));
    let size=vec2(record.decay,record.origin_offset);
    let bounds=area_clip_bounds(local,n,size);
    if !all(bounds.zw-bounds.xy>vec2(1e-4)) {return result;}
    let solid=area_rectangle_solid_angle(local,bounds);
    let center=0.5*(bounds.xy+bounds.zw);
    let light=normalize(center.x*x+center.y*y-delta);
    let roughness=max(0.001,surface.roughness*surface.roughness);
    let diffuse=area_diffuse_lobe(local,n,record.spot,bounds,solid);
    let specular=area_specular_lobe(local,n,v,roughness,solid);
    let slice=(record.flags>>8u)&255u;
    let diffuse_irradiance=area_sample(pool,trilinear,diffuse.bounds,size,slice);
    let specular_irradiance=area_sample(pool,trilinear,specular.bounds,size,slice);
    let colour=smoothstep(record.cutoff,record.falloff,length(delta))*visibility*record.color;
    let halfway=normalize(surface.view+light);
    let fresnel=ward_fresnel(surface.ior,shadow_saturate(dot(halfway,surface.normal)),surface.fresnel_strength);
    result.diffuse=colour*diffuse_irradiance*((1.0/3.14159265358979323846)*surface.diffuse*(vec3(1.0)-fresnel)*diffuse.solid_angle*3.0);
    let ratio=(surface.ior-vec3(1.0))/(surface.ior+vec3(1.0));
    let brdf=integrated_ward(shadow_saturate(dot(surface.view,surface.normal)),roughness);
    let integral=ratio*ratio*brdf.x*specular.solid_angle+(1.0-roughness*roughness)*brdf.y;
    result.specular=colour*specular_irradiance*integral*0.08;
    return result;
}
