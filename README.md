# Anima

A lightweight, native live wallpaper engine for Linux (Wayland), combining
Anima-style desktop characters with Wallpaper Engine style wallpapers.

Written in Rust, rendering directly on the `wlr-layer-shell` background layer
with EGL / OpenGL ES 3.0. No Electron, no Chromium, no XWayland.

```
┌──────────────────────────────┐
│ animad   background daemon   │  layer-shell surface per output
│ anima    CLI + TUI client    │  line-delimited JSON over a Unix socket
└──────────────────────────────┘
```

## Features

| Area | Status |
| --- | --- |
| Static images (png/jpg/webp/bmp/avif/tga/tex) | ✅ native |
| Animated GIF | ✅ native |
| Video (mp4/webm/mkv/mov/avi/m4v/ogv) | ✅ libmpv + hardware decode |
| GLSL shader wallpapers (`.frag`) | ✅ native |
| Scene wallpapers (`scene.toml`, Wallpaper Engine `.pkg`) | ✅ native particle/scene renderer |
| Web wallpapers (HTML/CSS/JS) | ⚠️ approximated — see [Limitations](#limitations) |
| Desktop character / mascot | ✅ procedural presets + sprite images |
| Audio-reactive wallpapers & characters | ✅ PulseAudio/PipeWire monitor + FFT |
| Lyrics / viseme lip-sync | ✅ MPRIS + LRCLIB |
| Playlists, next/prev, shuffle | ✅ |
| Multi-monitor, per-output config | ✅ |
| Pause on fullscreen / maximized / battery | ✅ (Hyprland events) |
| Steam Workshop browsing + `.pkg` / `.tex` unpack | ✅ |
| Transitions between wallpapers | ❌ not yet (roadmap) |
| Real HTML/JS web wallpapers | ❌ not yet (roadmap) |

## Performance model

The engine is built around "do nothing when nothing changes":

- **Static images idle at 0 fps.** A static wallpaper with no character
  overlay costs ~0.5 % CPU and no continuous GPU work.
- **Character over a static wallpaper** runs on its own clock (`character.fps`,
  default 30) and skips redrawing the unchanged wallpaper surface entirely.
  Measured on a 1920×1080 output (AMD Renoir iGPU): ~2.3 % CPU in release.
- **Animated wallpapers** run at the configured `fps` (60 by default), paced by
  compositor frame callbacks — no busy loops.
- **Audio capture sleeps** (0 % CPU, stream closed) unless the active wallpaper
  or character is audio-reactive.
- **Fullscreen / maximized / battery** pause is event-driven, not polled. The
  character overlay is hidden while a fullscreen window is focused.

## Requirements

- Wayland compositor with `wlr-layer-shell` (Hyprland, Sway, …)
- EGL + OpenGL ES 3.0 driver (Mesa)
- `libmpv` (for video wallpapers)
- PulseAudio or PipeWire (for audio-reactive content; optional at build/run time)
- Rust 1.85+ (edition 2024) to build

## Build

```sh
git clone <repo> anima && cd anima
cargo build --release
```

Binaries:

- `target/release/animad` — the daemon
- `target/release/anima` — the CLI

## Run

```sh
# start the daemon (foreground; use the systemd unit for daily use)
./target/release/animad --verbose

# in another terminal
./target/release/anima status
./target/release/anima set ~/Pictures/Wallpapers/fantasy-mountain.jpg
./target/release/anima set examples/wallpapers/cyber_scene
./target/release/anima next
```

### systemd user service

A unit is provided in [`contrib/animad.service`](contrib/animad.service).
It uses `Restart=always` so the daemon recovers from compositor hotplug hiccups.

```sh
mkdir -p ~/.config/systemd/user
cp contrib/animad.service ~/.config/systemd/user/
# edit ExecStart if the checkout is not at ~/Projects/anima
systemctl --user daemon-reload
systemctl --user enable --now animad.service
```

Hyprland users: make sure the session environment is visible to systemd:

```sh
systemctl --user import-environment WAYLAND_DISPLAY XDG_RUNTIME_DIR \
  XDG_CURRENT_DESKTOP HYPRLAND_INSTANCE_SIGNATURE
```

## CLI

```
anima set [PATH|ID] [-m MONITOR]   set a wallpaper (file, folder, name, Workshop ID)
anima next / prev                  playlist / directory navigation
anima pause / resume / toggle      freeze all or one output (-m)
anima volume [0..1|0..100] [--mute true|false]  video audio
anima prop [key] [value]           list / get / set user properties
anima playlist --interval N --shuffle true
anima char [preset|file|off] [--corner ...] [--placement ...]
anima workshop [QUERY]             scan Steam Workshop items (AppID 431960)
anima unpack <file.pkg|file.tex> [out]
anima tui                          interactive control center
anima reload | status | monitors | list
```

`status --json` exposes machine-readable output, including per-monitor frames,
fps, pause state, audio levels and lyric state.

## Configuration

`~/.config/anima/config.toml` (created on first run):

```toml
[general]
fps = 60                    # animated wallpaper frame rate
mute = true
volume = 0.0
automute = true             # mute video when another app plays audio
audio_capture = true        # desktop spectrum/beat analysis
pause_on_fullscreen = true
pause_on_maximized = false
pause_on_battery = false
battery_fps = 30
low_battery_pause = 15      # pause under this %, 0 disables
hwdec = "auto-safe"         # mpv hwdec mode
playlist_interval_secs = 0
playlist_shuffle = false
playlist = []               # explicit entries; empty = wallpapers_dir

[character]
enabled = true
source = "reze"             # reze|neko|cyber|spirit, a sprite or a .glb model path
corner = "bottom-right"
placement = "window-inside" # wallpaper|screen-front|window-inside|window-outside
scale = 1.15
offset = [24.0, 0.0]
opacity = 1.0
flip_x = false
look_at_mouse = true
audio_reactive = true
always_on_top = true
fps = 30                    # character clock over a static wallpaper

[[monitor]]
name = "eDP-1"
wallpaper = "/path/to/wallpaper"
fps = 60                    # optional per-output override

[monitor.properties]
schemecolor = "0.22 0.68 0.98"
```

Runtime changes via the CLI are persisted back to this file.

### 3D characters

`source` (and `extra`) also take a rigged glTF model (`.glb` / `.gltf`). It
plays its idle clip when standing, a walk clip while walking, a reaction clip
when petted and a pick-up clip while dragged (`Cafe_Idle`, `Cafe_Walk`,
`Cafe_Reaction`, `Formation_Pickup`, else any clip named like idle / walk /
react / pickup), turned toward the walking direction. The Blue Archive chibis
from [lihaohong6/BlueArchiveModels](https://github.com/lihaohong6/BlueArchiveModels)
work as-is:

```sh
name="Airi"; dir=~/.local/share/anima/ba/$name; mkdir -p "$dir"
curl -L -o "$dir/$name.glb" \
  "https://raw.githubusercontent.com/lihaohong6/BlueArchiveModels/main/${name// /%20}.glb"
```

then `source = "~/.local/share/anima/ba/Airi/Airi.glb"`, or pick it in
`anima-gui`, which lists `~/.local/share/anima/ba/*/` next to the sprites.

## Wallpaper formats

- **Images** — any format supported by the `image` crate; `.tex` (Wallpaper
  Engine texture) is decoded natively. Static images idle completely.
- **Video** — decoded with libmpv through its OpenGL render API. Hardware
  decoding uses `hwdec=auto-safe`; Vulkan-copy / VAAPI depending on the driver.
- **Shaders** — a single fragment shader source; the engine supplies standard
  uniforms (resolution, time, mouse, spectrum, beat). Multi-pass shaders are not
  supported yet.
- **Scenes** — `scene.toml` with layers and particle presets (`stars`,
  `fireflies`, `embers`, `sakura`, …), parallax and audio reactivity.
  Wallpaper Engine `scene.pkg` archives are supported for 2D image-layer
  scenes: model/material resolution, texture decoding (RGBA, DXT1/3/5, LZ4),
  screen-space layout, blend modes, layer effects and preset-driven particles
  (rain, snow, sakura, fog, embers, stars). 3D/perspective meshes and
  `application` wallpapers are not implemented.
- **Web** — metadata-driven HUD (see below).

## Limitations

These are deliberate, documented gaps rather than bugs:

1. **Web wallpapers are not a real browser.** `src/render/web.rs` reads
   `project.json` / `index.html` metadata and synthesizes a GPU HUD (clock,
   spectrum bars, accent color). Arbitrary HTML/CSS/JS is **not** executed.
   A WebKitGTK-based helper process is planned; see
   [`docs/web-renderer-plan.md`](docs/web-renderer-plan.md).
2. **No wallpaper transitions.** Switching applies immediately.
3. **Wayland only.** X11 and Windows/macOS are out of scope.
4. **Fullscreen detection uses Hyprland's IPC socket.** On other compositors
   the wallpaper still renders, but pause-on-fullscreen is a no-op.
5. **Wallpaper Engine compatibility is partial.** Common 2D scene and video
   wallpapers work. 3D/perspective meshes, multi-pass shader materials and
   `application` wallpapers are unsupported on Linux.
6. **Audio analysis is PulseAudio/PipeWire only** (loaded dynamically; the
   daemon still runs without it).
7. **No per-wallpaper sandboxing.** Shaders and scenes are trusted input.

## Roadmap

- [ ] Real HTML/CSS/JS wallpapers via a WebKitGTK helper process
      ([plan](docs/web-renderer-plan.md))
- [ ] Cross-fade / slide transitions between wallpapers
- [ ] Full Wallpaper Engine `.pkg` scene graph coverage
- [ ] Multi-pass shader wallpapers (`.json` shader pipelines)
- [ ] Scene editor / hot-reload while editing
- [ ] Wayland `ext-session-lock` integration (pause while locked)
- [ ] Per-output power profiles and HDR output

## Project layout

```
src/
  backend/wayland.rs   layer-shell surfaces, EGL, pacing, IPC, playback
  backend/governor.rs  Hyprland events / power state
  render/              image, video, shader, scene, web, character, gl helpers
  audio.rs             PulseAudio monitor capture + FFT + beat detection
  lyrics.rs            MPRIS + LRCLIB lyrics and viseme sequencer
  pkg.rs               Wallpaper Engine .pkg / .tex reader
  wallpaper.rs         wallpaper discovery and metadata
  config.rs            TOML configuration
  ipc.rs               line-delimited JSON protocol
  bin/animad.rs        daemon entry point
  bin/anima.rs         CLI / TUI
examples/wallpapers/   aurora shader, particle scene, clock HUD project
docs/                  design notes (web renderer plan)
```

## License

MIT — see [LICENSE](LICENSE).
