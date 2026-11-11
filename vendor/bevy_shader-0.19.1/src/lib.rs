//! Provides [`Shader`] assets for Bevy.

extern crate alloc;

mod shader;
mod shader_cache;
pub use shader::*;
pub use shader_cache::*;

/// The shader prelude.
///
/// This includes the most common types in this crate, re-exported for your convenience.
pub mod prelude {
    #[doc(hidden)]
    pub use crate::Shader;
}

#[doc(hidden)]
pub mod _macro {
    pub use bevy_asset;
}

/// Inline shader as an `embedded_asset` and retain it for the application lifetime.
///
/// This works around a limitation of the shader loader not properly loading
/// dependencies of shaders.
#[macro_export]
macro_rules! load_shader_library {
    ($asset_server_provider: expr, $path: literal $(, $settings: expr)?) => {
        $crate::_macro::bevy_asset::embedded_asset!($asset_server_provider, $path);
        let handle: $crate::_macro::bevy_asset::prelude::Handle<$crate::prelude::Shader> =
            $crate::_macro::bevy_asset::load_embedded_asset!(
                $asset_server_provider,
                $path
                $(,$settings)?
            );
        // bevy_backend: session ownership replaces a leaked strong handle.
        let world = $asset_server_provider.world_mut();
        if world.get_non_send_resource::<$crate::ShaderLibraries>().is_none() {
            world.insert_non_send_resource($crate::ShaderLibraries::default());
        }
        world.non_send_resource_mut::<$crate::ShaderLibraries>().retain(handle);
    }
}

/// bevy_backend: application-owned shader library leases. This is a non-send
/// World service because library registration already requires the App's main
/// thread; it introduces no runtime systems or extra ECS dependency.
#[doc(hidden)]
#[derive(Default)]
pub struct ShaderLibraries {
    handles: alloc::vec::Vec<bevy_asset::Handle<Shader>>,
    observed: alloc::vec::Vec<bevy_platform::sync::Weak<bevy_asset::StrongHandle>>,
}
impl ShaderLibraries {
    #[doc(hidden)]
    pub fn retain(&mut self, handle: bevy_asset::Handle<Shader>) {
        static OPTIONS: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
        let (observe, forget) = *OPTIONS.get_or_init(|| (
            std::env::var_os("DH2_DEBUG_SHADER_LIBRARY_LIFETIME").is_some_and(|value| value == "1"),
            std::env::var_os("DH2_DEBUG_FORGET_SHADER_LIBRARIES").is_some_and(|value| value == "1"),
        ));
        if observe && let bevy_asset::Handle::Strong(strong) = &handle {
            self.observed.push(bevy_platform::sync::Arc::downgrade(strong));
        }
        if forget { core::mem::forget(handle); }
        else { self.handles.push(handle); }
    }
    #[doc(hidden)]
    pub fn retirement(&self) -> ShaderLibraryRetirement {
        ShaderLibraryRetirement(self.observed.clone())
    }
}
#[doc(hidden)]
pub struct ShaderLibraryRetirement(alloc::vec::Vec<bevy_platform::sync::Weak<bevy_asset::StrongHandle>>);
impl ShaderLibraryRetirement {
    pub fn counts(&self) -> (usize, usize) {
        (self.0.len(), self.0.iter().filter(|handle| handle.strong_count() != 0).count())
    }
}
