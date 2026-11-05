# anima-hypr: Hyprland plugin for anima

Shares workspace state with `animad` through shared memory
(`$XDG_RUNTIME_DIR/anima-hyprland.shm`) and keeps animad's windows out of the way:

- **Publishes** each monitor's active workspace and the offset Hyprland draws
  it at. That offset is the value swipes and workspace animations move.
- **Places animad's character windows.** Wayland clients can't position their
  own windows, so animad writes where each one goes and the plugin moves it
  there before every frame, without animation. It only touches windows with
  app_id `anima-character` belonging to the animad process named in a record.
- **Lets clicks through** everywhere except on the characters. Hyprland
  hit-tests floating windows by their box, not their input region, so before
  each pointer move the screen-sized character windows are made unfocusable
  unless a character is under the cursor.

Without the plugin, animad falls back to an overlay that estimates swipes and
workspace animations from Hyprland's IPC and the touchpad.

## Build

The plugin must be built against the exact Hyprland you run. Rebuild it after
every Hyprland update:

```sh
make            # needs the hyprland headers (pkg-config hyprland)
install -Dm644 anima-hypr.so ~/.local/lib/anima/anima-hypr.so
```

## Load

Hyprland caches a plugin by its path. A rebuilt `.so` loads on the next
Hyprland start, or right away from a different path:
`hyprctl plugin load /some/new/path.so`.

`hyprland.lua` (Hyprland 0.57+):

```lua
hl.plugin.load("/home/you/.local/lib/anima/anima-hypr.so")

-- the character windows: transparent decorations that only take clicks on
-- the characters. Don't add no_focus: Hyprland drops no_focus windows from
-- click hit-testing. The plugin hands keyboard focus back instead.
hl.window_rule({
  match = { class = "^anima-character$" },
  float = true, no_anim = true, no_initial_focus = true, no_follow_mouse = true,
  border_size = 0, rounding = 0, no_shadow = true, no_blur = true, no_dim = true, decorate = false,
})
```

The plugin uses Hyprland 0.57's plugin API (event bus, `Config::Actions`) and
won't build against older versions.

See `docs/character-windows.md` for how the window mode works.
