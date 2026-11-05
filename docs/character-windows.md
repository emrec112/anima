# Character windows

How the walking characters stay in sync with workspace swipes and animations,
and what another compositor backend (niri) has to provide to use the same
mechanism. Written for whoever ports it next, human or agent.

## The problem

Characters walk along a *strip*: every workspace of the monitor side by side.
The overlay mode draws them on one layer-shell surface, which belongs to the
output, not a workspace. So during a swipe or workspace animation, animad has
to know exactly where the compositor draws each workspace on every frame, and
nonlinear easing and gesture acceleration make that hard to guess.
`governor::hypr_view` replays Hyprland's gesture and bezier logic for the
plugin-less fallback, and it is never quite exact.

## Window mode: let the compositor do it

Each workspace in the row gets one transparent, screen-sized window that sits
on that workspace. The compositor slides those windows with their workspace
like any other window, so they're exact by construction.

```
every workspace side by side (logical px):
 ┌──────────── strip FBO (one wide offscreen texture) ─────────────┐
 │ ws slot 0          │ ws slot 1 (active) │ ws slot 2          │
 └─────────┬──────────┴─────────┬──────────┴─────────┬──────────┘
           │ blit slice         │ blit slice         │ blit slice
       window 0             window 1             window 2   ← app_id anima-character
       on workspace 0       on workspace 1       on workspace 2
```

The per-frame steps (`backend/charwin.rs`, `WorkspaceWindows`):

1. **`sync`** makes one window per slot of the `Row`, plus the strip FBO.
2. **`begin_frame`** binds and clears the strip and returns a `StripTarget`.
3. **`CharacterOverlay::render`** draws every character as usual. It still
   works in output pixels relative to the *active* workspace, which in window
   mode is snapped to it (no slide offset). `StripTarget::clip_xform` maps
   that clip space onto the wide strip, so characters on other workspaces
   aren't clipped.
4. **`present`** blits each window's slice of the strip and swaps it.
   - Damage hints (`swap_buffers_with_damage`) cover only the character boxes
     of this frame and the last, so the compositor only recomposites there.
   - Windows on hidden workspaces refresh every 6th frame, except while the
     row is `sliding`.
   - The input region of each window is just the character boxes, so clicks
     elsewhere pass through.
   - `place(slot)` asks the compositor backend to keep the window on its
     workspace.

The old 1x1 layer surface stays as the frame clock. `ActorTarget` (in
`render/character.rs`) is the seam: the overlay uses `SharedSurface`, window
mode uses `StripTarget`.

Don't make one window per character. That was the first version: with 188
characters Hyprland spent ~90% of the iGPU compositing them. One window per
workspace costs about what one overlay costs.

### Performance notes

- A 3D pose renders into the model's own texture, and the sprite pass reuses
  it. Re-posing runs at ≤40 Hz (`CONTENT_HZ`), ~10 Hz off screen, and the
  sprite still moves every frame.
- With a crowd, at most `POSE_BUDGET` re-poses run per frame, round robin,
  never below `MIN_POSE_HZ`.
- Render on the GPU that drives the panel. A dGPU-rendered (PRIME) client is
  copied to the compositor's iGPU on every swap.

## What a compositor backend provides

Only these are compositor-specific (Hyprland's are in `backend/governor.rs`):

| Piece | Hyprland |
|---|---|
| Is window mode possible? | `can_place_windows()`: IPC socket + plugin loaded |
| The row (`Row { count, active, step, sliding }`) | `hypr_row()`: workspaces with real windows (not ours) plus the active one, by id; `step` = monitor width + `gaps_workspaces`; active and sliding come from the plugin each frame |
| Put window `slot` on workspace `slot` at the monitor's top-left | `hypr_place_workspace(slot)`, which writes a record the plugin applies before each frame (`contrib/hyprland-plugin`) |
| Strip view for the character code | `hypr_view()` returns the active workspace's slot, without the slide offset, while `WINDOW_MODE` is set |
| Window rules | float, no animation, no focus stealing, no decorations (see the plugin README) |

Windows are identified by app_id `anima-character` and title
`anima-character-<slot>`, plus the animad pid.

Workspaces holding only character windows don't count as busy. Otherwise
the characters would keep empty workspaces alive forever.

## Porting to niri

niri has no plugins, so placement has to go through IPC. On niri the row is
the *scrolling strip of columns* on a workspace, not a row of workspaces.
Two options:

1. **Keep the overlay.** That's the current niri path, with the strip scroll
   read from IPC (`contrib/niri-ipc-tiled-window-positions.patch`) or
   estimated. It's simplest, and exact once that patch is in.
2. **Window mode with floating windows.** niri floating windows don't scroll
   with the columns; they stay put while tiled columns move. So a window per
   *workspace* (niri's workspaces are vertical) handles workspace switches
   natively, but horizontal strip scrolling still needs the overlay's view
   offset. A niri `Row` would treat each vertical workspace as a slot.
   - Placement: `{"Action":{"MoveWindowToWorkspace":{...}}}` and
     `MoveFloatingWindow` through `$NIRI_SOCKET`, plus a window rule with
     `open-floating true`.
   - These actions animate, so a window rule that disables animations for
     app_id `anima-character` is needed.

Either way, `ActorTarget`, `WorkspaceWindows` and the character code stay
as they are. A niri backend only implements the table above.
