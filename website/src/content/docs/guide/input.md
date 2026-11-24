---
title: Input
description: Keyboard layouts and typing, mice and touchpads, and Hyprland's keyboard shortcuts.
---

Input has three pages: **Keyboard**, **Mouse & touchpad** and **Shortcuts**. They set Hyprland's
own settings, so they need Hyprland running.

A change on Keyboard or Mouse & touchpad applies at once and is saved to the Input part of
Lituus's own file (see [Hyprland](/guide/hyprland/#lituuss-own-file)). **Revert** puts the last
change back. When Hyprland's settings differ from the file (changed with `hyprctl`, say), **Save
to config file** saves the ones in use.

## Keyboard

- **Layouts**: up to four, which every keyboard uses. **Add a layout…** searches them ("German",
  "dvorak"); drag a layout, or press <kbd>Alt</kbd> <kbd>↑</kbd> <kbd>↓</kbd>, to reorder them. The
  first is the one in use at login, and **Switch now** goes to the next.
- **Switch layouts with**: the keys that, pressed together, go to the next layout.
- **Caps Lock acts as** and **Compose key**: what Caps Lock does, and a key that starts a sequence
  (<kbd>"</kbd> then <kbd>a</kbd> types ä).
- **Num Lock on at login**.
- **Repeat delay** and **repeat rate**, with a field to hold a key in and test them.

Your other xkb options stay as they are.

## Mouse & touchpad

Settings for every mouse (speed, acceleration, natural scrolling, scroll speed, left-handed) and
every touchpad (the same, plus tap to click, disable while typing, scrolling with two fingers or the
edge, and clicking with button areas or fingers). **Give a device its own settings…** sets one
mouse or touchpad apart from the others. Connected devices are listed with how they're connected
(USB, Bluetooth, built in).

## Shortcuts

The keyboard shortcuts in your Hyprland config, in groups and in words ("Close window", "Go to
workspace 3"), with their keys, their options, and the file and line each comes from.
<kbd>/</kbd> searches them by key or action.

- **Change…** (in a row's **⋯** menu, or by opening the row) shows the shortcut in a dialog: new
  keys (press them; Hyprland's own shortcuts are paused while Lituus listens), the action (run a
  command, go to a workspace, move or focus a window, and so on), and options such as repeating
  while held or working on the lock screen.
- The switch on each row turns a shortcut off or on.
- **Add a shortcut…** makes a new one; **Delete…** in its menu removes it again.

Lituus doesn't edit your config for this. It writes the change to the Shortcuts part of its own
file, which unbinds your shortcut and binds the new one, and applies it at once. That only lasts
when your config loads Lituus's file after its own binds, at the end (the page says when it
doesn't). **Back to your config's version** undoes a change. For a shortcut your config makes with
Lua code Lituus can't read, the dialog shows the code, and **Replace with a new shortcut…** turns
it off and makes a new one in its place.
