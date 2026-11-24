---
title: Apps
description: The apps your launcher shows, from their desktop files, edited as your own copy, hidden, added, and checked for problems.
---

The Apps page lists the apps your app launcher (rofi, wofi, fuzzel and the like) shows, from their
desktop files, grouped by category. Search by name, command or file name, or show only the apps
you **changed**, **hid** or **added**, or the ones with **problems**.

## Your own copy

Lituus never edits a system file. The first change to an app copies its desktop file to
`~/.local/share/applications`, which apps read first, and every change after goes to that copy;
the rest of the file (other keys, translations, comments) stays as it is. **Restore original**
deletes the copy, and the system's file counts again.

For each app you can change:

- its name, description, command and icon (**Choose…** picks one from your icon theme, or a file);
- its category, and whether it **shows in launchers**;
- **Open in a terminal**, for a program that runs in one;
- **Runs on Wayland**, for an Electron app: it adds `--ozone-platform-hint=auto`, so the app is a
  Wayland window, sharp on scaled monitors, instead of going through Xwayland;
- **Uses the dedicated GPU**, for laptops with two graphics cards;
- the **window class** Hyprland's window rules match, and the app's quick actions.

## Hiding

Hiding an app (`NoDisplay=true` in your copy) takes it out of your launcher. It stays installed
and keeps opening its files, and the Hidden list shows it again. Select several apps to hide them
at once.

## Adding an app

**Add an app…** adds one of two kinds:

- **A program** your package manager didn't install: an AppImage, a script, a game. For an
  AppImage, Lituus reads its own name, description, categories and icon, and makes it executable.
- **A website**, in a window of its own with its own place in the launcher. That needs a
  Chromium-based browser (Chromium, Brave, Vivaldi, Chrome, Edge); Firefox has no app windows.

## Problems

When the page opens, Lituus looks for problems, each with a fix:

- **An app can't start**: its command isn't there any more. Remove it from the launcher, or, for an
  AppImage with a newer version in the same folder, use that one.
- **An app has no icon**: your icon theme has none by its name. Choose another icon.
- **A package changed its file** after you made your own copy. Lituus can put your changes on the
  new file.
