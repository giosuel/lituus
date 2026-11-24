---
title: Hyprland
description: Window rules and keybinds, starting Lituus at login, the file Lituus writes to Hyprland's config folder, Set up…, and the polkit agent.
---

Lituus's window has the class (Wayland app ID) `ch.giosuel.Lituus`, so window rules can match
it. It's a normal window rather than a popup, so you decide where it goes. Ready-made files are in
the repository's [`extras/`](https://github.com/giosuel/lituus/tree/main/extras) folder.

## Lituus's own file

Displays, Keyboard, Mouse & touchpad and Shortcuts keep what you change in a file of Lituus's own,
next to your config, and never edit your config:

- With `hyprland.lua` (Hyprland 0.55 and newer), the file is `~/.config/hypr/lituus.lua`, and the
  line that loads it is `require("lituus")`.
- With `hyprland.conf`, the file is `~/.config/hypr/lituus.conf`, and the line that loads it is
  `source = ~/.config/hypr/lituus.conf`.

Each page writes only its own part of the file. The line goes at the end of your config, after
your own settings and shortcuts, so Lituus's come last: a shortcut changed on the Shortcuts page
only takes effect when Lituus's file loads after the bind it replaces.

Until your config has the line, those pages say **Your Hyprland config doesn't load Lituus's file
yet**: what you change applies at once, but is lost when Hyprland restarts. **Set up…** on that
notice adds the line for you. It shows the line and the file first, keeps a copy of your config as
`hyprland.lua.bak` (or `.bak.1` and on, if that name is taken), adds the line at the end, and has
Hyprland reload. A config that's a link into your dotfiles stays a link. This is the only time
Lituus writes to your config, and only after you confirm.

## Without Hyprland

Displays, Keyboard, Mouse & touchpad and Shortcuts work through Hyprland. When it isn't running
(on another desktop, say), they're greyed out in the sidebar, and a tooltip says why. The other
pages work on any desktop.

## The Lua config (Hyprland 0.55 and newer)

```lua
-- ~/.config/hypr/hyprland.lua
local lituus = [[^(ch\.giosuel\.Lituus)$]]

hl.window_rule({
    match = { class = lituus, title = "^(Lituus)$" },
    float = true,
    size = "1200 760",
    center = true,
})

-- The quick popup, in the top right corner under the bar.
hl.window_rule({
    match = { class = lituus, title = "^(Lituus quick)$" },
    float = true,
    pin = true,
    size = { 380, 600 },
    move = { "monitor_w-window_w-12", "52" },
})

hl.on("hyprland.start", function()
    hl.exec_cmd("lituus --background")
end)

hl.bind("SUPER + N", hl.dsp.exec_cmd("lituus"))
hl.bind("SUPER + SHIFT + N", hl.dsp.exec_cmd("lituus --quick"))
hl.bind("XF86WLAN", hl.dsp.exec_cmd("lituus --toggle-wifi"))

-- At the very end: the settings from Displays, Input and Shortcuts.
require("lituus")
```

For a bar at the bottom, or to open the popup at the pointer, use
`move = { "cursor_x-(window_w*0.5)", "cursor_y+12" }` instead.

## hyprland.conf

```ini
# ~/.config/hypr/hyprland.conf, Hyprland 0.53 and newer
windowrule = float on, match:class ^(ch\.giosuel\.Lituus)$, match:title ^(Lituus)$
windowrule = size 1200 760, match:class ^(ch\.giosuel\.Lituus)$, match:title ^(Lituus)$
windowrule = center on, match:class ^(ch\.giosuel\.Lituus)$, match:title ^(Lituus)$
windowrule = float on, match:class ^(ch\.giosuel\.Lituus)$, match:title ^(Lituus quick)$
windowrule = pin on, match:class ^(ch\.giosuel\.Lituus)$, match:title ^(Lituus quick)$
windowrule = size 380 600, match:class ^(ch\.giosuel\.Lituus)$, match:title ^(Lituus quick)$
windowrule = move (monitor_w-window_w-12) 52, match:class ^(ch\.giosuel\.Lituus)$, match:title ^(Lituus quick)$

exec-once = lituus --background
bind = SUPER, N, exec, lituus
bind = SUPER SHIFT, N, exec, lituus --quick
bind = , XF86WLAN, exec, lituus --toggle-wifi

# At the very end: the settings from Displays, Input and Shortcuts.
source = ~/.config/hypr/lituus.conf
```

On Hyprland 0.48 to 0.52 the rules are written the other way round, for example
`windowrule = float, class:^(ch\.giosuel\.Lituus)$, title:^(Lituus)$` (all of them are in `extras/hyprland.conf`); before 0.48 they're `windowrulev2`.

## Why start it in the background

`lituus --background` starts Lituus without a window, and it keeps running when you close the
window. That matters for a few things that happen while no window is open:

- Passwords you keep "for me only" are in your keyring, and NetworkManager asks Lituus for them
  whenever it connects, including right after you log in.
- Display profiles that switch by themselves do so when you plug monitors in.
- With awww, the wallpaper slideshow is run by Lituus.

Running `lituus` (from the keybind) opens the window of the one that's already running. See
[The window](/guide/window/) for the tray icon and the quick popup.

## A polkit agent

NetworkManager asks polkit before changing connections that are shared with all users, and
renaming the computer on the System page asks too. On Hyprland that needs a polkit agent running,
for example:

```ini
exec-once = systemctl --user start hyprpolkitagent
```

Connections stored only for your user don't need it. When an action does, Lituus says so
instead of failing silently.
