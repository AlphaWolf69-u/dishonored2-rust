#define_import_path dh2_material_samplers

// Shared material linear/clamp sampler, independent of surface equations.
@group(#{MATERIAL_BIND_GROUP}) @binding(149) var linear_clamp:sampler;
