---
title: Settings
description: Lituus's own settings, its colours from your wallpaper or a built-in theme, config.toml, and every file Lituus writes.
---

**Settings**, at the bottom of the sidebar, is for Lituus itself. Everything on it is saved to
`~/.config/lituus/config.toml`.

## Appearance

- **Follow wallpaper**: take the accent colour from the colours matugen makes from your wallpaper.
  Only the accent changes; the backgrounds, text and edges stay Lituus's own neutral greys.
- **Built-in theme**: Teal, Blue, Lavender, Coral or Mono, used when Follow wallpaper is off or no
  wallpaper colours are found.
- **Mode**: dark, light, or Auto, which follows the desktop.
- **Hero gradient**: the accent glow at the top of each page.

The wallpaper's colours come from the first usable **colour file** in a list. By default that's
`~/.config/lituus/colors.json`, then `~/.config/ml4w/colors/colors.json`, where ML4W's setup
writes the colours matugen makes on every wallpaper change. A colour file is a matugen-style
`colors.json`, with Material colour roles such as `primary` and `surface` as hex strings; one with
missing roles, or text too faint to read, is skipped. The page lists the files and whether each
was found; add or reorder them in `config.toml`. Lituus recolours itself when the file changes, so
it follows your wallpaper. The [Wallpaper](/guide/wallpaper/) page's **Run after a
change** can run matugen for you.

## General

- **Tray icon**: show Lituus in the system tray, when a bar with a tray runs.
- **Close to tray**: closing the window keeps Lituus running in the tray.
- **Notifications**: when a connection comes up, goes down or fails, while the window is closed.

## Updates and passwords

**Updates** has the command each **Update** button on the [System](/guide/system/) page runs, with
**Reset** for one you changed, and the terminal they run in. **Passwords** says where passwords
kept "for me only" go: which keyring, or NetworkManager when there's none.

## config.toml

Every key is optional, and a missing file means the defaults. Lituus changes one key at a time and
keeps the rest of the file, comments included. A file it can't read is reported and ignored, so a
typo never keeps Lituus from starting.

```toml
[theme]
follow_wallpaper = true   # false: always the built-in theme
builtin = "teal"          # or "blue", "lavender", "coral", "mono"
mode = "auto"             # or "dark", "light"
gradient = true           # the accent glow behind each page's hero

# Colour files, in order; the first usable one wins. These two are the defaults.
[[theme.sources]]
path = "~/.config/lituus/colors.json"

[[theme.sources]]
path = "~/.config/ml4w/colors/colors.json"

[tray]
enabled = true        # the tray icon
keep_running = true   # closing the window keeps Lituus in the tray

[notifications]
enabled = true        # connected, disconnected and failed, while the window is closed

[sound]
overamplify = false   # volume sliders go to 150 %

[bluetooth]
accept_files_from = []   # devices (by address) whose files are saved without asking

[updates]
# A changed update command, by package manager (pacman, aur, dnf, zypper, apt, xbps, flatpak, snap)
pacman = "sudo pacman -Syu"
```

The Wallpaper page keeps its settings in a `[wallpaper]` section of the same file. With
`$XDG_CONFIG_HOME` set, the folder is `$XDG_CONFIG_HOME/lituus`.

## Files Lituus writes

- `~/.config/lituus/config.toml`: these settings, and those of Sound, Bluetooth and Wallpaper.
- `~/.config/lituus/displays.toml`: display profiles and workspaces (Displays).
- `~/.config/hypr/lituus.lua` or `lituus.conf`: monitors, input settings and shortcuts (Displays,
  Keyboard, Mouse & touchpad, Shortcuts).
- `~/.config/hypr/hyprland.lua` or `hyprland.conf`: one line at the end, after a `.bak` copy, only
  from Set up… and only when you confirm ([Hyprland](/guide/hyprland/#lituuss-own-file)).
- `~/.config/hypr/hyprpaper.conf`: a block for each monitor, after a `.bak` copy (Wallpaper, with
  hyprpaper).
- `~/.local/share/applications/`: your copies of apps' desktop files, and the apps you add; the
  icons of AppImages you add go to `~/.local/share/icons/` (Apps).
- `~/.config/mimeapps.list` (or your desktop's own one) and `~/.config/xdg-terminals.list`: default
  apps and the default terminal (Default apps).
- `~/.local/share/lituus/certificates/`: certificates and keys from imported VPN files (VPN).
- Your Downloads folder: files received over Bluetooth.

Connections themselves are NetworkManager's, and passwords kept "for me only" are in your keyring.
