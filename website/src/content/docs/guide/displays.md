---
title: Displays
description: Arranging monitors on Hyprland, Apply and Revert, workspaces bound to monitors, and profiles that switch when you plug in.
---

The Displays page sets up your monitors through Hyprland, so it needs Hyprland running. Its three
tabs at the top are **Monitor**, **Workspaces** and **Profiles**.

## Monitors

The monitors are drawn as Hyprland places them: drag one to move it (it snaps to the others'
edges; hold <kbd>Shift</kbd> not to snap). Below, the selected monitor's settings:

- **Display**: on or off, resolution, refresh rate, scale, rotation and flip, position, and
  mirroring another monitor.
- **Colour and HDR**: colour mode (sRGB, wide gamut, HDR), bit depth (8 or 10-bit), and SDR
  brightness and saturation in HDR.
- **Advanced**: variable refresh rate (VRR), and a reserved area at each edge.

## Apply, then keep

Changes wait for **Apply**, which sets them live and asks whether to keep them. With no answer in
15 seconds the old settings come back, so a monitor that stays black fixes itself. **Keep** saves
them; **Revert** drops changes you haven't applied yet. The bar at the bottom says what's applied
and saved. When the monitors were changed some other way and differ from what Lituus saved,
**Save to config file** saves what's in use.

## Lituus's own file

Kept settings go to the Displays part of Lituus's own file, `~/.config/hypr/lituus.lua` (or
`lituus.conf` with a `hyprland.conf`), never to your config. **View config** shows what Lituus
writes there. Until your config loads that file, the page says so, and **Set up…** adds the line
that does: see [Hyprland](/guide/hyprland/#lituuss-own-file).

Monitors are named by make, model and serial (`desc:`), so a monitor keeps its settings when it
moves to another port. **Name monitors by description**, under Advanced, turns that off for port
names such as `DP-1`.

## Workspaces

Binds workspaces 1 to 10 to a monitor: a bound workspace opens there. Each monitor has a default
workspace, the one it shows when it turns on, and a workspace kept when empty stays even with no
windows on it.

## Profiles

A profile is the current layout saved under a name ("Desk + TV"), to switch to again. A profile
that switches by itself is applied when exactly its monitors are plugged in, while Lituus runs (in
the tray, or with `lituus --background`), and it can run a command afterwards, for example
`pkill -SIGUSR2 waybar`. Profiles and workspaces are kept in `~/.config/lituus/displays.toml`.
