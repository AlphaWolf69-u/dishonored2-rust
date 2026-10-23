# Flash UI (Iggy) runtime

The game's UI is RAD Iggy: AS3 movies in `.iggy` files with BC7 textures in
`.raw.iggytex`. The rewrite runs the installed movies in its own player.

| Location | Responsibility |
| --- | --- |
| `src/assets/iggy.rs` | `.iggy` container: header, sections, relocation programs (`FUN_141093380`). |
| `src/assets/iggy/` | Movie root, timelines/placements/SymbolClass, shapes/fills/bitmaps/iggytex index, fonts and edit text. |
| `src/assets/lang.rs` | String tables (`strings/<language>_m.lang`) and localized library names (`lib_loc_flash_*`). |
| `src/flash/avm2*` | AVM2 interpreter, builtins and the table-driven native class installer. |
| `src/flash/native_table.rs` | Generated from Iggy's registration (`FUN_14104cc40`) by `analysis/generate_native_table.py`. |
| `src/flash/*.rs` | Flash API ports: display list, timelines and Iggy's lifecycle, events, geometry, text, graphics, filters, timers, `IggyFunctions`. |
| `src/flash/render.rs`, `tessellate.rs` | Display list to triangle batches (fill texgen, gradient ramps, colour transforms). |
| `src/flash/raster.rs`, `src/bin/iggy_preview.rs` | CPU reference rasterizer and preview tool. |
| `src/rendering/bevy_backend/flash_ui.rs` | GPU pass over the display output after post-processing. |
| `src/app/pause_menu.rs` | Escape opens the pause movie; ticks it at its frame rate. |

## Evidence rules

- Native class shapes (members, namespaces, constants, trivial handlers) come
  only from the generated table. A native Iggy implements without a Rust port
  is `MethodKind::Unimplemented` and fails with its address; the test
  `rust_natives_match_iggy_table` rejects Rust members Iggy does not declare.
- Startup and tick order follow `FUN_141028160` and `FUN_141021b40`
  (allocate-then-construct timeline children, post-order constructors,
  deferred gotos, broadcasts in reverse registration order).
- The language is Steam's `UserConfig.language` for AppID 403640.

## Open questions

1. Error subclass construction: Error has a native constructor (0x14107ef30)
   but TypeError and the other subclasses register the IllegalOperationError
   stub. Whether `new TypeError()` reaches the stub is unresolved; the rewrite
   keeps Error's constructor. Next: trace the generic path of `FUN_141044820`
   past the type-tag switch, or break on 0x1410b38e0 in the running game.
2. `m_isLightVM` default: classes gated on `VM+0x6b40` are installed on the
   assumption that the pause decl's unset bit is 0. Next: read the decl
   default in `FUN_1404f2cd0` or the VM flag at runtime.
3. Curve flattening tolerance (`player+0x1e80`), the exact GDraw blend states,
   masks/clip layers, filters, cacheAsBitmap and 3D transforms are not
   rendered yet.
4. Text layout is approximate until the engine documented in
   `artifacts/ui-pause/notes-text-engine.md` (`FUN_141030f60`) is ported.
5. The pause contract (`notes-pause-contract.md`): bindable actions, entry
   locks, saves, statistics, tutorials, options and the callbacks are not yet
   written into the movie, so `_gotoPauseScreen` fails and the menu screens stay
   hidden. Game-time pause (`idGameLocal::IsPaused`) and the `ui_pause` screen
   FX are not applied yet.
