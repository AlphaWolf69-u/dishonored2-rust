Bevy Shader0.19.1, MIT OR Apache-2.0 (upstream licenses retained).

load_shader_library keeps its strong handles in an application-owned World
service instead of forgetting them. The existing missing-dependency workaround
still keeps every library alive while the application exists. Application
shutdown can now release those handles and their index allocator/channel
backing. Optional weak observers compare former/current lifetime behavior.
Shader source, embedded paths, loading settings and compilation are unchanged.
