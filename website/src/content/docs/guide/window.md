---
title: The window
description: The sidebar, Ctrl K, the keys, airplane mode, the tray icon and quick popup, notifications and the command line.
---

## The sidebar

The pages are in four groups, each with a short status beside it (the network you're on, how many
Bluetooth devices are connected, the keyboard layout):

- **Connections**: Network (Wi-Fi, Wired, VPN, Hotspot) and Bluetooth.
- **Devices**: Displays, Sound, and Input (Keyboard, Mouse & touchpad, Shortcuts).
- **Desktop**: Wallpaper, Apps and Default apps.
- **System**: System.

Network and Input fold away with their chevron. **Settings**, for Lituus itself, and the
**Airplane mode** switch are at the bottom. Pages that need Hyprland are greyed out while it isn't
running ([Hyprland](/guide/hyprland/#without-hyprland)).

## Ctrl K

<kbd>Ctrl</kbd> <kbd>K</kbd>, or the search field at the top of the sidebar, opens the command
palette over the page. It finds:

- **pages**;
- **settings**, by their name and the words they're known by, each with its value now, so a
  switch, a choice or a slider can be changed right there;
- **actions**: connect to or disconnect from a Wi-Fi network or a VPN, connect a Bluetooth device,
  update all packages.

With nothing typed it shows the quick settings (Wi-Fi, Bluetooth, airplane mode) and what you used
last.

| Key | In the palette |
|---|---|
| <kbd>↑</kbd> <kbd>↓</kbd> | Select a row |
| <kbd>Enter</kbd> | Open the page, run the action, flip the switch, or step into a choice or slider |
| <kbd>←</kbd> <kbd>→</kbd> | Change the choice or slider you stepped into, live |
| <kbd>Enter</kbd> / <kbd>Esc</kbd> | Keep the new value / put the old one back |
| <kbd>Ctrl</kbd> <kbd>Enter</kbd> | Go to the row's page, with the setting highlighted |

## Keys

The bar at the bottom of the window shows the keys that work on the page you're on.

| Key | Action |
|---|---|
| <kbd>Ctrl</kbd> <kbd>K</kbd> | Find a page, setting or action |
| <kbd>F6</kbd> | Move between the sidebar and the page |
| <kbd>/</kbd> | Search the list on the page: networks, VPNs, shortcuts, pictures |
| <kbd>↑</kbd> <kbd>↓</kbd> | Select a row |
| <kbd>Enter</kbd> | Connect to the selected network |
| <kbd>E</kbd> | Edit the selected connection |
| <kbd>S</kbd> | Look for Bluetooth devices |
| <kbd>F2</kbd> | Rename the computer (System) |
| <kbd>Esc</kbd> | Clear the search, close a dialog, leave the editor |

## Airplane mode

The switch at the bottom of the sidebar (also in the tray menu, the quick popup and
<kbd>Ctrl</kbd> <kbd>K</kbd>) turns off Wi-Fi, mobile broadband and Bluetooth. It ends when one of
them comes back on, here or elsewhere. Turning it off brings back the radios that were on before.

## The tray icon and the quick popup

While Lituus runs, its icon is in Waybar's `tray` module (or any other StatusNotifierItem tray),
showing the signal of the network you're on.

- **Right-click** for the menu: Wi-Fi, the networks in range, your VPNs, airplane mode, Open
  Lituus, Settings and Quit.
- **Left-click** for the quick popup: the Wi-Fi switch and the nearest networks, the VPNs, the
  Bluetooth devices, the volume and airplane mode. `lituus --quick` opens or hides it too.

With a tray, closing the window keeps Lituus running there. Both can be turned off in
[Settings](/guide/settings/). The window rules that place the popup are on the
[Hyprland](/guide/hyprland/) page.

Waybar's own `network` module can open the popup too:

```json
"network": {
    "on-click": "lituus --quick",
    "on-click-right": "lituus"
}
```

## Notifications

While the window isn't showing, Lituus sends a desktop notification when a connection comes up,
goes down or fails. It uses the standard notification service, which mako, swaync and dunst
provide. Turn them off in Settings.

## The command line

There's one Lituus per user: starting it again passes the request to the one that runs.

| Command | |
|---|---|
| `lituus` | Open the window (of the running one, if there is one) |
| `lituus --background` | Start without a window, for login ([why](/guide/hyprland/#why-start-it-in-the-background)) |
| `lituus --quick` | Show the quick popup, or hide it (for a bar's `on-click` or a keybind) |
| `lituus --toggle-wifi` | Turn Wi-Fi on or off |
| `lituus --version` | The version |
