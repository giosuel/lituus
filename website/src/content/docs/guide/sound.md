---
title: Sound
description: Outputs, inputs, a volume per app and sound cards' profiles, through PipeWire's PulseAudio service.
---

The Sound page talks to PipeWire through its PulseAudio service, `pipewire-pulse`, so
`pavucontrol`, `wpctl` and your bar's volume module keep working alongside it. When the service
isn't running, the page says so and gives the command that starts it
(`systemctl --user enable --now pipewire-pulse`). Everything changes live.

## Devices

- **Output**: where sound plays, its volume and mute, the port (which of its connectors plays,
  such as speakers or headphones), and the balance. **Louder than 100 %** lets the volume sliders
  go to 150 %; sound may distort.
- **Input**: the microphone or other input apps hear, its volume and mute.

## Apps

Each app playing sound, with its own volume and mute, and the output it plays through.

## Cards

Each sound card's profile: which outputs and inputs it offers. The tab shows only when there are
cards to choose a profile for. A Bluetooth headset's choice between high quality and headset is
also on the [Bluetooth](/guide/bluetooth/#audio) page.

The quick popup has the output volume too ([The window](/guide/window/#the-tray-icon-and-the-quick-popup)).
