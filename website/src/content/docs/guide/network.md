---
title: Network
description: Wi-Fi, wired connections, VPNs (WireGuard, OpenVPN, Cisco AnyConnect) and the hotspot, through NetworkManager, and where passwords are kept.
---

The Network pages (Wi-Fi, Wired, VPN and Hotspot) drive NetworkManager, so `nmcli`, nm-applet and
other tools keep working alongside Lituus, and what they change shows up at once. **+** at the top
of the Wi-Fi, Wired and VPN pages adds a connection of any kind.

## Wi-Fi

The networks in range, with their signal and security. <kbd>/</kbd> searches them, and the Wi-Fi
switch at the top turns Wi-Fi on or off. <kbd>Enter</kbd> connects to the selected network; a
secured one asks for its password first. **Connect to hidden network…** is below the list.

The selected network's details have **Disconnect**, **Edit**, **Share** (its name and password as
a QR code, to scan with a phone) and **Forget**, and the switches for connecting automatically and
for a metered connection.

WPA Enterprise networks (802.1X, such as eduroam) take PEAP, TTLS, TLS or PWD, with the CA
certificate and the server's domain checked, so a fake access point can't take your password.

## Wired

Your Ethernet connections, each with its settings, and 802.1X for networks that ask for a username
or a certificate on the cable. **+** › Virtual interfaces adds a **VLAN**, a **bridge** (for
virtual machines, say) or a **bond**, with the devices they're made of.

## VPN

- **WireGuard** is built into NetworkManager: start an empty connection, with a key pair made for
  you, or import a `.conf` file (the wg-quick format). **Export .conf…** writes one back out.
- **OpenVPN** needs NetworkManager's OpenVPN plugin. Set it up by hand or import an `.ovpn` file;
  the certificates and keys in it are copied to `~/.local/share/lituus/certificates`, where
  NetworkManager can read them.
- **Cisco AnyConnect** needs NetworkManager's OpenConnect plugin. The same plugin connects to
  GlobalProtect, Pulse / Ivanti, Juniper, Fortinet, F5 and Array. Signing in opens the plugin's
  own window, which also shows web sign-in pages such as Microsoft 365's. **Import…** turns the
  profiles of openconnect-saml (or openconnect-sso) into connections.

A Wi-Fi or wired connection can bring a VPN up with it: **Start VPN with this network** in its
editor.

The packages for each plugin are listed on [Installing](/install/).

## Hotspot

Shares this computer's internet connection over Wi-Fi. Set the network name, the password
(**Generate** makes one), and the band (2.4 GHz, 5 GHz or automatic). While it's on, the page
lists the devices connected to it and shows a QR code to join. It needs a Wi-Fi card.

## The editor

**Edit** (or <kbd>E</kbd>) opens every setting of a connection in tabs: General, then the
connection's own (Wi-Fi, Security, Ethernet, Peers, …), then IPv4 and IPv6, with addresses, DNS
servers, search domains and routes. Changes wait for **Apply** (**Save** for a new connection);
**Revert** puts them back.

## Passwords

When you join a network, Lituus asks where to keep its password:

- **For me only** (the default): in your keyring (GNOME Keyring, KeePassXC or anything else that
  speaks the Secret Service, or KWallet), and the connection is yours alone. No admin password
  needed. Without a keyring, NetworkManager keeps the password instead.
- **All users**: NetworkManager keeps it for everyone; changing it needs an admin password, so a
  [polkit agent](/guide/hyprland/#a-polkit-agent).
- **Ask each time**: not saved at all.

Passwords kept "for me only" are given to NetworkManager by Lituus when it connects, so start
Lituus in the background at login ([Hyprland](/guide/hyprland/#why-start-it-in-the-background)).
The Settings page says which keyring is in use.
