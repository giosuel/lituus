---
title: Sound
description: Outputs, inputs, a volume per app playing or recording, and sound cards' profiles, through PipeWire's PulseAudio service.
---

The Sound page talks to PipeWire through its PulseAudio service, `pipewire-pulse`, so
`pavucontrol`, `wpctl` and your bar's volume module keep working alongside it. When the service
isn't running, the page says so and gives the command that starts it
(`systemctl --user enable --now pipewire-pulse`). Everything changes live.

## Devices and apps

The page's first view has Output and Input on the left and the apps on the right. In a narrow
window they stack: Output, Input, then the apps.

- **Output**: where sound plays, its volume and mute, the port (which of its connectors plays,
  such as speakers or headphones), and the balance. **Louder than 100 %** lets the volume sliders
  go to 150 %; sound may distort.
- **Input**: the microphone or other input apps hear, its volume and mute.
- **Apps**: each app playing sound, with its own volume and mute and the output it plays through.
  While an app records (a call, a screen recorder), it's listed under **Recording**, with its
  volume and mute and the input it records from; that can also be an output's monitor, which
  records what that output plays. Each row is one stream, so an app on a call has a row in each,
  and moving one changes only that app: the output and input in use stay as they are.

## Cards

Each sound card's profile: which outputs and inputs it offers, as pavucontrol's Configuration
tab has it. This view shows only when there are cards to choose a profile for. A Bluetooth
headset's choice between high quality and headset is also on the
[Bluetooth](/guide/bluetooth/#audio) page.

The quick popup has the output volume too, and under it the apps playing, three at most, each
with its own volume and mute ([The window](/guide/window/#the-tray-icon-and-the-quick-popup)).
