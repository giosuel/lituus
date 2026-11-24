<div align="center">

<img src="docs/assets/hero.svg" alt="The Lituus logo, window tiles cut in a golden spiral, beside a Lituus window where a Wi-Fi network connects" width="760">

# Lituus

**The settings app for Hyprland and other desktops that don't come with one.**<br>
<sub>Written in Rust · NetworkManager, BlueZ, PipeWire and Hyprland underneath</sub>

[![CI](https://img.shields.io/github/actions/workflow/status/giosuel/lituus/ci.yml?branch=main&style=flat-square&label=CI&logo=githubactions&logoColor=white)](https://github.com/giosuel/lituus/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/giosuel/lituus?style=flat-square&color=8e4ec6&include_prereleases)](https://github.com/giosuel/lituus/releases)
[![License: AGPL-3.0](https://img.shields.io/github/license/giosuel/lituus?style=flat-square&color=3e63dd)](LICENSE)
[![Powered by Rust](https://img.shields.io/badge/powered_by-Rust-dea584?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Slint](https://img.shields.io/badge/UI-Slint-2379f4?style=flat-square)](https://slint.dev)
[![NetworkManager](https://img.shields.io/badge/NetworkManager-D--Bus-5cc8b0?style=flat-square)](https://networkmanager.dev)
[![PipeWire](https://img.shields.io/badge/PipeWire-pulse-8e4ec6?style=flat-square)](https://pipewire.org)
[![Wayland native](https://img.shields.io/badge/Wayland-native-e5484d?style=flat-square&logo=wayland&logoColor=white)](#)
[![Hyprland ready](https://img.shields.io/badge/Hyprland-ready-58e1ff?style=flat-square&logo=hyprland&logoColor=white)](#hyprland)

[**Website**](https://lituus.giosuel.io/) · [**Install**](#install) · [**Hyprland setup**](#hyprland) · [**Keys and commands**](#keys-and-commands)

</div>

---

Running Hyprland, and reaching for `nmtui`, `bluetoothctl`, `pavucontrol` and a text editor for
`monitor =` lines? Lituus is the settings window those desktops are missing: network, Bluetooth,
displays, sound, input, wallpaper, apps and default apps, and your system, in one app. It drives
the services you already have (NetworkManager, BlueZ, PipeWire, Hyprland), so their own tools keep
working alongside it. The guide for each page is on the [website](https://lituus.giosuel.io/).

## What it does

<table>
<tr>
<td width="33%" valign="top">

**Network**<br>
Wi-Fi, wired and VPNs, all live: connect with <kbd>Enter</kbd>, a full editor for every setting (802.1X, IPv4 and IPv6, DNS, routes, VLANs, bridges, bonds), WireGuard and OpenVPN files, Cisco AnyConnect with web sign-in such as Microsoft 365's, a hotspot with a QR code. Passwords go to your keyring.

</td>
<td width="33%" valign="top">

**Bluetooth**<br>
Pairing with every kind of code, battery levels, headset audio (high quality or headset), sending and receiving files, and a phone's internet over Bluetooth.

</td>
<td width="33%" valign="top">

**Sound**<br>
The output and input in use, a volume per app and the output it plays through, ports, balance and sound cards' profiles, live with PipeWire.

</td>
</tr>
<tr>
<td valign="top">

**Displays** (Hyprland)<br>
Drag your monitors into place; set the mode, scale, rotation, mirror, HDR and VRR; Apply, then keep it or it goes back by itself, or Revert. Workspaces bound to monitors, and profiles that switch when you plug in.

</td>
<td valign="top">

**Input** (Hyprland)<br>
Keyboard layouts, Caps Lock, the Compose key and key repeat; mice and touchpads, each with its own settings if you like; and your Hyprland shortcuts in words, changed or turned off in a dialog.

</td>
<td valign="top">

**Wallpaper**<br>
Pictures from a folder, on each monitor, through awww or hyprpaper: fill, transitions, a slideshow, and a command after each change (`matugen image {file}`).

</td>
</tr>
<tr>
<td valign="top">

**Apps**<br>
What your launcher shows and how each app starts: edited as your own copy of its desktop file, hidden or restored, an AppImage or a website added as an app, icons chosen, problems found.

</td>
<td valign="top">

**Default apps**<br>
Which app opens links, each kind of file and any single file type, as the `mimeapps.list` files say, and the terminal, through `xdg-terminal-exec`.

</td>
<td valign="top">

**System**<br>
Hardware, storage and distribution; package updates for pacman, the AUR, DNF, zypper, APT, XBPS, Flatpak and Snap, run in your terminal; renaming the computer; a report for bug reports.

</td>
</tr>
<tr>
<td valign="top">

**Your wallpaper's colours**<br>
Takes its accent from the colours matugen makes from your wallpaper (ML4W's, or any `colors.json`) and follows each change. Built-in themes otherwise: Teal, Blue, Lavender, Coral and Mono, each dark and light.

</td>
<td valign="top">

**Your Hyprland config stays yours**<br>
Displays, Input and Shortcuts write a file of Lituus's own. The one line that loads it, **Set up…** adds at the end of your config after asking, with a `.bak` copy first. Without Hyprland, those pages are greyed out.

</td>
<td valign="top">

**Made for tiling desktops**<br>
A normal window with the app ID `ch.giosuel.Lituus` for your window rules, keyboard first (<kbd>Ctrl</kbd> <kbd>K</kbd> finds any page, setting or action), airplane mode, a tray icon with a quick popup, and a background mode for login.

</td>
</tr>
</table>

## Install

**Arch Linux**: from the AUR, `paru -S lituus-bin` (prebuilt) or `paru -S lituus` (from source).

**Debian, Ubuntu**: the `.deb` from the [latest release](https://github.com/giosuel/lituus/releases), `sudo apt install ./lituus_*_amd64.deb`.

**Fedora, openSUSE**: the `.rpm` from the same release, `sudo dnf install ./lituus-*.x86_64.rpm` (or `sudo zypper install`).

**AppImage (any distribution)**: `Lituus-*-x86_64.AppImage` from the same release, then:

```sh
chmod +x Lituus-*-x86_64.AppImage && ./Lituus-*-x86_64.AppImage
```

**Flatpak**: `Lituus-*-linux-x64.flatpak` from the same release (the runtime comes from Flathub):

```sh
flatpak install --user ./Lituus-*-linux-x64.flatpak && flatpak run ch.giosuel.Lituus
```

The sandbox covers Network, Bluetooth and Sound; it can't reach Hyprland, your config files or
your package managers, so the other pages need another package. Details are in
[`packaging/flatpak/`](packaging/flatpak/README.md).

**Nix**: `nix run github:giosuel/lituus`, or the flake as an input ([`packaging/nix/`](packaging/nix/README.md)).

Every release also has ARM64 builds (except the Flatpak). All of them need NetworkManager. The
rest depends on the pages you use, and most pages say when something is missing:

- **OpenVPN**: NetworkManager's OpenVPN plugin (`networkmanager-openvpn` on Arch).
- **Cisco AnyConnect**: its OpenConnect plugin (`networkmanager-openconnect`); Microsoft 365 and
  other web sign-ins need openconnect 9.00 and NetworkManager-openconnect 1.2.10 or newer.
- **Bluetooth**: BlueZ (`bluez`, with its service running); its `obexd` for files.
- **Sound** and headset audio: PipeWire's PulseAudio service (`pipewire-pulse`).
- **Displays, Keyboard, Mouse & touchpad, Shortcuts**: Hyprland.
- **Wallpaper**: awww (0.12 or newer) or hyprpaper (0.8 or newer).
- **The default terminal**: `xdg-terminal-exec`. **Websites as apps**: a Chromium-based browser.
- **Renaming the computer**, and connections for all users: systemd-hostnamed and a polkit agent.
- **Choosing files** (certificates, VPN files, files to send): an XDG desktop portal with a file
  dialog (`xdg-desktop-portal-gtk`, say).
- **Passwords "for me only"**: a keyring (GNOME Keyring, KWallet, or another Secret Service).

The website's [Installing](https://lituus.giosuel.io/install/) page has the package names for
each distribution.

**From source**: with [Rust](https://rustup.rs) and fontconfig's development files:

```sh
git clone https://github.com/giosuel/lituus && cd lituus
cargo build --release
./target/release/lituus
```

## Hyprland

Add the rules from [`extras/hyprland.lua`](extras/hyprland.lua) to `~/.config/hypr/hyprland.lua`,
and end it with the line that loads Lituus's own file:

```lua
local lituus = [[^(ch\.giosuel\.Lituus)$]]
hl.window_rule({ match = { class = lituus, title = "^(Lituus)$" }, float = true, size = "1200 760", center = true })
hl.window_rule({ match = { class = lituus, title = "^(Lituus quick)$" }, float = true, pin = true,
    size = { 380, 600 }, move = { "monitor_w-window_w-12", "52" } })   -- the quick popup, top right

hl.on("hyprland.start", function() hl.exec_cmd("lituus --background") end)
hl.bind("SUPER + N", hl.dsp.exec_cmd("lituus"))                     -- open the window
hl.bind("SUPER + SHIFT + N", hl.dsp.exec_cmd("lituus --quick"))     -- the quick popup
hl.bind("XF86WLAN", hl.dsp.exec_cmd("lituus --toggle-wifi"))        -- the Wi-Fi key

require("lituus")   -- at the very end: what Displays, Input and Shortcuts set
```

**Lituus's own file.** Displays, Keyboard, Mouse & touchpad and Shortcuts write what you change
to `~/.config/hypr/lituus.lua` (or `lituus.conf`, for a `hyprland.conf`), and never edit your
config. The `require("lituus")` line (or `source = ~/.config/hypr/lituus.conf`) loads it, after
your own settings and shortcuts. Until your config has it, those pages say so, and **Set up…**
adds it at the end of your config after asking, keeping a copy as `hyprland.lua.bak`. Displays'
Apply sets changes live and asks to keep them; with no answer in 15 seconds they go back, so a
black screen fixes itself. Profiles and workspaces are kept in `~/.config/lituus/displays.toml`.

> [!TIP]
> Still on `hyprland.conf` (Hyprland 0.54 or older, or not migrated yet)? Use [`extras/hyprland.conf`](extras/hyprland.conf) instead. It also has the rule syntax for Hyprland older than 0.53.

**The tray icon** shows up in Waybar's `tray` module (and any other StatusNotifierItem tray) while
Lituus runs: right-click for the menu, left-click for the quick popup. With a tray, closing the
window keeps Lituus running there; both can be turned off in Settings.

**Why the background mode?** Passwords you keep "for me only" live in your keyring, and
NetworkManager asks Lituus for them whenever it connects, including right after you log in.
Display profiles also switch by themselves only while Lituus runs, and so does awww's slideshow.
`lituus --background` runs without a window until then; `lituus` opens the window of the one
that's already running, and closing the window leaves it running.

When you join a network, Lituus asks where to keep its password:

- **For me only** (the default): in your keyring (GNOME Keyring, KeePassXC or anything else that
  speaks the Secret Service, or KWallet), and the connection is yours alone. No admin password
  needed. Without a keyring, NetworkManager keeps the password instead.
- **All users**: NetworkManager keeps it for everyone; changing it needs an admin password. On
  Hyprland that needs a polkit agent, such as `hyprpolkitagent`.
- **Ask each time**: not saved at all.

## Keys and commands

<details>
<summary><b>Keys and command-line options</b></summary>
<br>

| Key | Action |
|---|---|
| <kbd>Ctrl</kbd> <kbd>K</kbd> | Find a page, setting or action (the command palette) |
| <kbd>F6</kbd> | Move between the sidebar and the page |
| <kbd>/</kbd> | Search the list on the page: networks, VPNs, shortcuts, pictures |
| <kbd>↑</kbd> <kbd>↓</kbd> | Select a row |
| <kbd>Enter</kbd> | Connect |
| <kbd>E</kbd> | Edit the selected connection |
| <kbd>S</kbd> | Look for Bluetooth devices |
| <kbd>F2</kbd> | Rename the computer (System) |
| <kbd>Esc</kbd> | Clear the search · close a dialog · leave the editor |

| Command | |
|---|---|
| `lituus` | Open the window (of the running one, if there is one) |
| `lituus --background` | Start without a window, for login |
| `lituus --quick` | Show the quick popup, or hide it (for a bar's `on-click` or a keybind) |
| `lituus --toggle-wifi` | Turn Wi-Fi on or off |
| `lituus --version` | The version |

</details>

## Settings and colours

Lituus's own settings are on its Settings page (at the bottom of the sidebar), saved to
`~/.config/lituus/config.toml`. It takes its colours from the first usable source in that file,
and recolours itself when the source changes, so it follows your wallpaper. Without a config file
it uses these two, then its built-in theme:

```toml
[theme]
follow_wallpaper = true   # false: always the built-in theme
builtin = "teal"          # or "blue", "lavender", "coral", "mono"
mode = "auto"             # or "dark" / "light", for the built-in theme
gradient = true           # the accent glow behind each page's hero

[[theme.sources]]
path = "~/.config/lituus/colors.json"

[[theme.sources]]
path = "~/.config/ml4w/colors/colors.json"   # ML4W writes matugen's colours here
```

A source is a matugen-style `colors.json` (Material colour roles such as `primary` and `surface` as
hex strings). Only the accent comes from it; the backgrounds and text stay Lituus's neutral greys.
One with missing roles, or text too faint to read, is skipped.

The Settings page writes the rest of the file too, and keeps your comments in it:

```toml
[tray]
enabled = true        # the tray icon
keep_running = true   # closing the window keeps Lituus in the tray

[notifications]
enabled = true        # connected, disconnected and failed, while the window is closed

[updates]
pacman = "sudo pacman -Syu"   # a changed command for System's Update button
```

Every key, and every file Lituus writes, is on the website's
[Settings](https://lituus.giosuel.io/guide/settings/) page.

## Development

```sh
scripts/test.sh
```

runs every test, including the ones against a mock NetworkManager and BlueZ (python-dbusmock on a
private bus, so the real ones are never touched). It needs `dbus-daemon` and python-dbusmock
(`apt install python3-dbusmock`, or `pip install python-dbusmock` in a venv with
`LITUUS_PYTHON` pointing at its `python`), and fails rather than skipping them when they're
missing. With `gnome-keyring` installed, the keyring tests also run against a real one on a
private bus, and with PipeWire, WirePlumber and `pipewire-pulse`, the sound tests against a
private PipeWire.

`scripts/mock-nm.sh COMMAND` runs any command against the same mocks, filled with the networks and
Bluetooth devices from the mockups, for example the app itself, or the debug tool that prints what
Lituus reads from NetworkManager:

```sh
cargo build -p lituus -p lituus-cli
scripts/mock-nm.sh target/debug/lituus
scripts/mock-nm.sh target/debug/lituus-cli list   # or: target/debug/lituus-cli list, for the real one
```

The crates, CI and releasing are on the website's
[Development](https://lituus.giosuel.io/development/) page.

### Versions

The version comes from the git tags (`scripts/version.sh`): `Cargo.toml` always says `0.0.0-dev`,
and release builds write the tag's version in with `scripts/set-version.sh`.

### Dependency versions

Every crate version in `Cargo.lock` must have been public for at least 14 days, so a freshly
published (possibly compromised) release can't slip in. `cargo xtask dep-age` checks this, and CI
runs it. Security fixes and explicit exceptions go in `xtask/dep-age-exceptions.txt`, each with its
reason.

## License

[AGPL-3.0-or-later](LICENSE). The bundled fonts, Inter and JetBrains Mono, are under the [SIL Open Font License](crates/app/ui/fonts).
