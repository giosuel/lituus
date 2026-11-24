---
title: System
description: The computer's names, hardware and storage, its distribution, package updates through pacman, the AUR, DNF, zypper, APT, XBPS, Flatpak and Snap, and a report for bug reports.
---

The System page shows what this computer is and runs, as far as Lituus can read it without root.

## Names

The top of the page shows the computer's two names: its **name**, any text you like ("Giosuel's
desk"), which Bluetooth devices see too, and its **network name** (a–z, 0–9 and hyphens), which
the router and other computers see. **Rename this computer** (or <kbd>F2</kbd>) changes them
through systemd-hostnamed; saving asks for an admin password, so it needs a
[polkit agent](/guide/hyprland/#a-polkit-agent).

## Hardware and storage

The processor, graphics, memory, network adapters, firmware, battery and displays, and each drive
with how full its partitions are.

## Software

- **Distribution**: its name and version, what it's based on, and how long it's supported.
- **Updates**: how many package updates each package manager has, counted without a password.
  Lituus knows pacman (with `checkupdates` from pacman-contrib, or as of the last sync without it),
  the AUR (through paru or yay), DNF, zypper, APT (as of the last package lists), XBPS, Flatpak and
  Snap; only the ones installed are listed. They're counted when the page opens, then every 30
  minutes while it's open.
- **This computer runs**: the kernel, the desktop and Lituus's own version.
- **What Lituus's pages use**: the versions of NetworkManager, BlueZ, PipeWire and the wallpaper
  program, or that one isn't installed or running.

**Update** runs that package manager's update command in your terminal, and **Update all** runs
them one after another. The commands (such as `sudo pacman -Syu` or `paru -Sua`) can be changed in
[Settings](/guide/settings/), and the terminal is the one set on
[Default apps](/guide/default-apps/#the-terminal).

## For a bug report

The button at the top right copies the whole page as plain text, without the computer's names and
serial numbers, to paste into an issue or a forum post.
