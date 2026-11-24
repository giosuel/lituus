---
title: Default apps
description: Which app opens links, each kind of file and a terminal, as the XDG mimeapps.list files and xdg-terminal-exec say.
---

The Default apps page sets which app opens links and files (what `xdg-open` and other apps use),
and which terminal apps open.

## Kinds of files

- **Everyday**: web browser (links and web pages), email (`mailto:` links), files (folders, and
  "Show in folder" in other apps), terminal, and text editor (plain text, code and config files).
- **Media and documents**: images, videos, music, PDFs and archives.

Choosing an app for a kind sets it for all of that kind's file types. **By file type** finds any
single type (`png`, `.md`, `video/mp4`) and sets an app for that one alone; those show as
exceptions.

## Where they come from

Apps look in several `mimeapps.list` files, first one first, and the first that names an app for a
file type wins. Lituus reads them in the same order, so each row shows what really opens. **Where
these come from** lists the files. A choice is saved to your own file: the first of yours that
exists (such as `~/.config/hyprland-mimeapps.list`), else `~/.config/mimeapps.list`.

## The terminal

The terminal is the one `xdg-terminal-exec` opens, set in its `xdg-terminals.list` in
`~/.config`. Apps that ask for a terminal and the Update buttons on the System page use it.
Choosing one here doesn't change the terminal your own keybind starts. Without
`xdg-terminal-exec`, Lituus's Update buttons use `$TERMINAL`, or the first terminal it knows that's
installed.
