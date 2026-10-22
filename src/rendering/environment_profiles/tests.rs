use super::*;
use crate::assets::installation;
use crate::rendering::fog::settings::FogSettings;
use bevy_math::Vec3;
use std::collections::HashSet;

#[test]
#[ignore = "requires installed environment declarations and image headers"]
fn installed_profiles_preserve_presence_prepare_images_and_reuse_identity() {
    let catalog = Catalog::open(&installation::game_base().unwrap()).unwrap();
    let read = |name: &str| -> Result<Option<String>, String> {
        catalog
            .read(&format!("generated/decls/env/{name}.decl"))
            .map(|bytes| bytes.map(|bytes| entities::declaration_text(&bytes).into_owned()))
            .map_err(|e| e.to_string())
    };
    let mut world = None;
    for path in catalog.paths_with_suffix(".entities") {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let doc = entities::parse(&entities::declaration_text(&bytes)).unwrap();
        if let Some(name) = entities::world_environment(&doc) {
            world = Some(name);
            break;
        }
    }
    let world = world.unwrap();
    let text = read(&world).unwrap().unwrap();
    let parameters =
        entities::environment_render_parameters_with_inheritance(&world, &text, read).unwrap();
    let lighting = entities::environment_lighting_with_inheritance(&text, read).unwrap();
    let sun = lighting
        .sun_theta_degrees
        .zip(lighting.sun_phi_degrees)
        .zip(lighting.sun_color)
        .map(|((theta, phi), color)| {
            (
                Vec3::from_array(crate::render_shadow::environment_sun_direction(theta, phi)),
                Vec3::from_array(crate::render_light::sun_ground_color(color)),
            )
        });
    let schema = EnvironmentParameters::capture(&catalog, &parameters, |read| {
        FogSettings::resolve(read, sun, 7000.)
    })
    .unwrap();
    let mut cache = EnvironmentProfiles::new(&schema);
    let mut image_resources = HashSet::new();
    let mut image_requests = 0;
    let mut count = 0;
    let mut supplied = 0;
    let mut absent = 0;
    let mut image_profile = None;
    for path in catalog
        .paths_with_suffix(".decl")
        .into_iter()
        .filter(|p| p.starts_with("generated/decls/env/"))
    {
        let name = path
            .strip_prefix("generated/decls/env/")
            .unwrap()
            .strip_suffix(".decl")
            .unwrap();
        let profile = cache
            .load(&catalog, name, |parameter, source| {
                image_requests += 1;
                let resource = crate::material::image_load_resource(&catalog, source)
                    .ok_or_else(|| format!("{name}/{parameter}: unresolved image {source}"))?;
                if image_resources.insert(resource.clone())
                    && crate::material::intrinsic_image_pixels(&resource).is_none()
                    && crate::material::intrinsic_cube_image(&resource).is_none()
                {
                    let bytes = catalog
                        .read_prefix(&resource, 64)
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| format!("missing profile image {resource}"))?;
                    crate::texture::BimageInfo::parse(&bytes)?;
                }
                Ok(resource)
            })
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let repeated = cache
            .load(
                &catalog,
                &name.to_ascii_uppercase(),
                |_, _| -> Result<String, String> {
                    panic!("cached profile unexpectedly prepares images again")
                },
            )
            .unwrap();
        assert!(Arc::ptr_eq(&profile, &repeated));
        let authored = entities::environment_render_parameters_with_inheritance(
            name,
            &read(name).unwrap().unwrap(),
            read,
        )
        .unwrap();
        for key in schema.values.keys() {
            let present = authored.parameter(key).is_some();
            assert_eq!(
                profile.parameters.values.contains_key(key),
                present,
                "{name}/{key} presence"
            );
            supplied += usize::from(present);
            absent += usize::from(!present);
        }
        assert_eq!(
            profile.images.len(),
            profile
                .parameters
                .values
                .values()
                .filter(|v| matches!(v, EnvironmentValue::Image(_)))
                .count()
        );
        if !profile.images.is_empty() {
            image_profile = Some(name.to_owned());
        }
        count += 1;
    }
    assert!(count > 0 && supplied > 0 && absent > 0 && image_requests > 0);
    let name = image_profile.unwrap();
    let mut retry = EnvironmentProfiles::<String>::new(&schema);
    assert!(retry
        .load(&catalog, &name, |_, _| Err(
            "injected image preparation failure".into()
        ))
        .is_err());
    assert!(retry.profiles.is_empty());
    assert!(retry
        .load(&catalog, &name, |_, source| Ok(source.to_owned()))
        .is_ok());
    println!("Prepared {count} environment profiles against {world} fog schema: {supplied} supplied, {absent} absent fields, {image_requests} image fields, {} unique resolved image resources",image_resources.len());
}
