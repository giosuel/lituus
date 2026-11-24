---
title: Bluetooth
description: Pairing and connecting devices through BlueZ, headset audio, sending and receiving files, and a phone's internet.
---

The Bluetooth page drives BlueZ, so `bluetoothctl` keeps working alongside it. It needs the
Bluetooth service running; when it isn't, the page says so and gives the command that starts it
(`sudo systemctl enable --now bluetooth`).

## Pairing and connecting

**Look for devices** (or <kbd>S</kbd>) lists the devices nearby; it stops by itself after two
minutes. Pairing says first how the device pairs: both screens show a code to compare, you type a
code on a keyboard, an older device takes a fixed PIN, or there's no code at all (put headphones
in pairing mode first).

A paired device's details have **Connect** or **Disconnect**, its battery level while it's
connected, a new name for it, and **Forget**. **Trusted** lets it connect and use its services
without asking; **Blocked** refuses every connection from it.

## Audio

A connected headset or speaker shows a choice between **High quality** (A2DP), **Headset** (HFP,
with its microphone on) and **Off**, as far as it offers them. That choice is PipeWire's, so it needs PipeWire's
PulseAudio service (`pipewire-pulse`). Volumes and outputs are on the [Sound](/guide/sound/) page.

## Files

**Send files…** on a device that takes files sends them, with their progress. When a device sends
you a file, Lituus asks whether to accept it, and saves it in your Downloads folder.
**Always accept files from** that device skips the question next time. Files go through BlueZ's
`obexd` (part of `bluez` on Arch, `bluez-obexd` on Debian and Ubuntu).

## A phone's internet

For a phone that shares its connection over Bluetooth, **Use the phone's internet** connects to it
through NetworkManager. Turn on Bluetooth tethering on the phone and connect it first.

## The adapter

The gear at the top opens the adapter's settings: its name (what other devices see when they look
for this computer), how long the **Visible** switch keeps the computer visible (3 minutes, or
always), and whether new devices can ask to pair. With airplane mode or a hardware switch blocking
the radio, the page offers to unblock it.
