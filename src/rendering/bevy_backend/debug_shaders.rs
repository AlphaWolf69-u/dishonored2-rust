//! Opt-in source editing for the running renderer. Normal builds use embedded
//! shaders. DH2_DEBUG_SHADER_ROOT points at a rewrite checkout; only shader
//! assets already owned/registered by the application are replaced in place.
use bevy::{prelude::*, shader::Source};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

struct Watched {
    path: PathBuf,
    modified: Option<SystemTime>,
    body: Option<String>,
}
#[derive(Resource)]
struct ShaderSources {
    root: PathBuf,
    vendor: Option<PathBuf>,
    watched: HashMap<AssetId<Shader>, Watched>,
    checked: Option<Instant>,
    missing: std::collections::HashSet<String>,
}

pub(super) fn register(app: &mut App) {
    let Some(root) = std::env::var_os("DH2_DEBUG_SHADER_ROOT").map(PathBuf::from) else {
        return;
    };
    assert!(
        root.join("assets/shaders").is_dir(),
        "debug shader root does not contain assets/shaders: {}",
        root.display()
    );
    let packages = std::fs::read_dir(root.join("vendor"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("bevy_pbr-"))
        .map(|entry| entry.path().join("src"))
        .collect::<Vec<_>>();
    let vendor = (packages.len() == 1).then(|| packages[0].clone());
    info!(root=%root.display(), "Debug shader source reload enabled");
    app.insert_resource(ShaderSources {
        root,
        vendor,
        watched: HashMap::new(),
        checked: None,
        missing: Default::default(),
    })
    .add_systems(PreUpdate, reload);
}

fn source_path(root: &Path, vendor: Option<&Path>, shader: &Shader) -> Option<PathBuf> {
    let path = shader.path.replace('\\', "/");
    if let Some(start) = path.find("assets/shaders/") {
        return Some(root.join(&path[start..]));
    }
    if let Some(relative) = path.strip_prefix("embedded://bevy_pbr/") {
        return vendor.map(|root| root.join(relative));
    }
    if let Some(start) = path.find("/src/").filter(|_| path.contains("bevy_pbr-")) {
        return vendor.map(|root| root.join(&path[start + 5..]));
    }
    None
}

fn reload(mut sources: ResMut<ShaderSources>, mut shaders: ResMut<Assets<Shader>>) {
    if sources
        .checked
        .is_some_and(|time| time.elapsed() < Duration::from_secs(1))
    {
        return;
    }
    sources.checked = Some(Instant::now());
    let sources = &mut *sources;
    let mut updates = Vec::new();
    let registered = shaders
        .iter()
        .map(|(_, shader)| shader.import_path.clone())
        .collect::<std::collections::HashSet<_>>();
    for (_, shader) in shaders.iter() {
        for import in &shader.imports {
            if !registered.contains(import) {
                let missing = format!("{}: {:?}", shader.path, import);
                if sources.missing.insert(missing.clone()) {
                    warn!(%missing, "Debug shader import unavailable");
                }
            }
        }
    }
    for (id, shader) in shaders.iter() {
        let Some(path) = source_path(&sources.root, sources.vendor.as_deref(), shader) else {
            continue;
        };
        let Source::Wgsl(original) = &shader.source else {
            continue;
        };
        let watched = sources.watched.entry(id).or_insert_with(|| Watched {
            path,
            modified: None,
            body: None,
        });
        let Ok(modified) =
            std::fs::metadata(&watched.path).and_then(|metadata| metadata.modified())
        else {
            continue;
        };
        if watched.modified == Some(modified) {
            continue;
        }
        let text = match std::fs::read_to_string(&watched.path) {
            Ok(text) => text,
            Err(error) => {
                warn!(path=%watched.path.display(), %error, "Debug shader source unreadable");
                continue;
            }
        };
        watched.modified = Some(modified);
        // Some owners wrap a file in generated kernels, imports and entry
        // points. Replace only the previously observed file body: replacing
        // the entire shader silently discards that owner's generated code.
        let source = if let Some(body) = &watched.body {
            if !original.contains(body) {
                warn!(path=%watched.path.display(), "Debug shader body no longer matches its owner");
                continue;
            }
            original.replacen(body, &text, 1)
        } else {
            if !original.contains(&text) {
                warn!(path=%watched.path.display(), "Debug shader file differs from its embedded body; rebuild required");
                continue;
            }
            watched.body = Some(text);
            continue;
        };
        watched.body = Some(text);
        if source == original.as_ref() {
            continue;
        }
        let mut replacement = Shader::from_wgsl(source, shader.path.clone());
        replacement.import_path = shader.import_path.clone();
        replacement.additional_imports = shader.additional_imports.clone();
        replacement.shader_defs = shader.shader_defs.clone();
        replacement.file_dependencies = shader.file_dependencies.clone();
        replacement.validate_shader = shader.validate_shader.clone();
        updates.push((id, replacement, watched.path.clone()));
    }
    for (id, shader, path) in updates {
        // Asset events invalidate the real pipeline cache and import dependents.
        shaders.insert(id, shader).expect("live shader asset ID");
        info!(path=%path.display(), "Debug shader source reloaded");
    }
}
