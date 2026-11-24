---
title: Wallpaper
description: Setting wallpapers with awww or hyprpaper, per monitor, with transitions, a slideshow and a command after each change.
---

The Wallpaper page sets pictures through the program that draws them: **awww** (swww's new name,
0.12 and newer) or **hyprpaper** (0.8 and newer). With neither installed, it says so. With one
installed but not running, it offers to start it and shows the line that starts it with Hyprland.

## Pictures

The top of the page shows your monitors with the picture each one has now. Below is a folder of
pictures (`~/Pictures/Wallpapers` if you have one, else `~/Pictures`) in a grid: click one to set
it. <kbd>/</kbd> searches them. Choose another folder, go back to one you used before, and include
its subfolders' pictures.

## Settings

- **Monitors**: where a click sets the picture, on all monitors or the ones you pick.
- **Fill**: fill the screen (cropping what doesn't fit), fit inside, stretch, centre (awww) or tile
  (hyprpaper). With awww, **Bar colour** fills the rest of the screen around a picture that fits
  inside or is centred.
- **Transition** (awww): its type and how long it takes. Only awww animates GIFs; with hyprpaper
  they're dimmed in the grid.
- **Slideshow**: change the picture by itself every few minutes, in a random order or in order.
  hyprpaper runs its own slideshow; with awww, Lituus changes the picture while it runs (in the
  tray, or with `lituus --background`).
- **Program**: awww or hyprpaper, and **Run after a change**: a command run after each new
  picture, with `{file}` for the picture, such as `matugen image {file}` to make new colours from
  it.
- **Hyprland's splash text** (hyprpaper): the text drawn over the picture.

awww shows the last pictures again by itself after a restart. hyprpaper forgets what it's told
while it runs, so Lituus also writes `~/.config/hypr/hyprpaper.conf`: a block for each monitor you
set, with the rest of the file left as it is. When you have one already, Lituus asks once first and
keeps it as `hyprpaper.conf.bak`.

The page's settings are kept in the `[wallpaper]` section of `~/.config/lituus/config.toml`.
