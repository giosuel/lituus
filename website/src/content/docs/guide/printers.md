---
title: Printers
description: The printers CUPS has, their queues and what they print by default; adding printers from the network or USB, driverless ones with nothing installed.
---

The Printers page talks to CUPS, the printing service, through its local socket
(`/run/cups/cups.sock`), so `lpstat`, CUPS's web interface and your apps' print dialogs keep
working alongside it. It follows CUPS as things change: a job sent from an app shows up in the
queue at once.

When CUPS isn't running, the page says so and gives the command that starts it
(`sudo systemctl enable --now cups.socket`); when it isn't installed, the command that installs it.
Lituus never installs drivers or services itself.

## The printers

The list has every printer CUPS has, with what it's doing ("Ready", "Printing", "Paused", "Out of
paper"). A printer on the network that doesn't answer when the page opens says **Not found on the
network**; its settings can still be changed, and jobs wait in its queue until it's back.

The selected printer's details:

- **Queue**: the jobs waiting or printing, with the time they were sent, and who sent them when it
  wasn't you. **Cancel** stops one; other people's jobs can be cancelled only by an administrator.
- **What it prints by default**: paper, two-sided, colour and quality, used when an app doesn't
  choose. Lituus writes them where both kinds of apps read them (CUPS's own settings and the
  printer's PPD), so GTK, Qt and LibreOffice follow too.
- **Printer**: its address and driver. "Driverless (IPP Everywhere)" means nothing is installed for
  it.
- **Make default** makes it your default printer, for you only (in `~/.cups/lpoptions`): other
  users on the computer keep theirs. Your default printer has a **Your default** badge instead.
- **Print a test page** sends CUPS's test page, with the printer's details on it.
- **Remove** removes the printer from this computer after asking.

## Adding a printer

**Add a printer…** lists the printers on your network that it finds, as they answer, through Avahi
(mDNS). Most printers made in the last ten years are driverless (IPP Everywhere or AirPrint): those
are added with one click and need nothing installed. A USB printer that needs a driver says which
package has it, such as `splix` or `hplip`; install that, and it can be added too.

A printer that isn't found can be added by its address: an IP address (`192.168.1.40`), a name
(`printer.local`) or a whole address (`ipp://printer.local/ipp/print`). Lituus checks something
answers there before asking CUPS to add it.

New printers aren't shared with other computers, and their paper is the one your language and
region use (A4, or Letter in North America).

Printers on the network are found only while Avahi runs. On Arch it comes with CUPS but isn't
started; the dialog gives the command (`sudo systemctl enable --now avahi-daemon`).

## Who can change printers

Changing printers needs CUPS's administrator rights. Members of the `wheel` group (Arch, Fedora) or
`lpadmin` (Debian, Ubuntu) have them, and Lituus asks for no password. Anyone else changes printers
through [cups-pk-helper](https://www.freedesktop.org/software/cups-pk-helper/releases/), which asks
for the password through your polkit agent (`hyprpolkitagent` on Hyprland). Without either, the
page says what to install, and only the changes that need no rights stay available: printing a
test page, making a printer your default, and cancelling your own jobs.
