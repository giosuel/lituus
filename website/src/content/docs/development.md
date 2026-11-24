---
title: Development
description: Building, testing, the two-week dependency rule, versions and releasing.
---

The repository is a Cargo workspace. The app is one crate; what it talks to (NetworkManager,
BlueZ, PipeWire, Hyprland, …) is in libraries of their own, with no UI.

| Path | What it is |
|---|---|
| `crates/app` | The Slint app, binary `lituus`: the pages (`src/*.rs`) and their UI (`ui/*.slint`) |
| `crates/nm` | NetworkManager over D-Bus: the model, its change stream and the secret agent |
| `crates/bt` | BlueZ over D-Bus: adapters and devices, their change stream, the pairing agent and obexd |
| `crates/keyring` | Passwords in the Secret Service or KWallet, for "for me only" |
| `crates/sound` | PipeWire through its PulseAudio protocol: outputs, inputs, apps and cards |
| `crates/hypr` | Hyprland over its sockets: monitors, input devices, binds, and Lituus's own config file |
| `crates/wallpaper` | awww and hyprpaper, hyprpaper.conf, and the pictures in a folder |
| `crates/xdg` | Desktop files, mimeapps.list files and file types |
| `crates/system` | The computer's hardware and distribution, package updates and the default terminal |
| `crates/cli` | `lituus-cli`, which prints what Lituus reads from NetworkManager; not shipped |
| `xtask` | Repository tooling (`cargo xtask dep-age`) |
| `packaging/` | Desktop file, metainfo, icon, AppImage script, AUR, Flatpak and Nix packages |
| `extras/` | Hyprland window rules and keybinds to copy |
| `scripts/` | Tests, the mocks, versions and packaging helpers |
| `website/` | This site |

## Building and testing

```sh
cargo build
scripts/test.sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

`scripts/test.sh` runs every test, including the ones against a mock NetworkManager and BlueZ
(python-dbusmock on a private bus, so the real ones are never touched). It needs
`dbus-daemon` and python-dbusmock (`apt install python3-dbusmock`, or `pip install
python-dbusmock` in a venv with `LITUUS_PYTHON` pointing at its `python`), and fails rather than
skipping those tests when they're missing. With `gnome-keyring` installed, the keyring tests also
run against a real one; with PipeWire, WirePlumber and `pipewire-pulse`, the sound tests run
against a private PipeWire.

`scripts/mock-nm.sh COMMAND` runs any command against the same mocks, filled with the networks and
Bluetooth devices from the mockups:

```sh
cargo build -p lituus -p lituus-cli
scripts/mock-nm.sh target/debug/lituus          # the app, on mock networks
scripts/mock-nm.sh target/debug/lituus-cli list # what Lituus reads from NetworkManager
```

CI runs these on every PR, plus the dependency check below, the desktop-file and AppStream
validators, the Nix flake, and `lituus-hypr`'s tests against a real Hyprland. A ruleset only needs
to require its aggregate **CI result** job.

## Dependency versions

Every crate version in `Cargo.lock` must have been public for at least 14 days, so that a freshly
published, possibly compromised, release can't slip in:

```sh
cargo xtask dep-age
```

When it finds a version that's too new, it prints the `cargo update --precise` command that picks
the newest version that's old enough. Security fixes and explicit exceptions go in
`xtask/dep-age-exceptions.txt`, each with its reason.

## Versions

The version comes from the git tags, and nothing is bumped by hand. `Cargo.toml` always says
`0.0.0-dev`; `scripts/version.sh` prints the version of the checked-out commit:

| Checked-out commit | Version |
|---|---|
| on `v0.1.0` | `0.1.0` |
| 5 commits after `v0.1.0` | `0.1.1-dev.5+g<hash>` |
| no tags yet | `0.0.0-dev+g<hash>` |

`scripts/set-version.sh <version>` writes a version into the workspace for a release build. The
release workflow and the AUR source package run it; its changes are never committed.

## Releasing

Pushing a `v*` tag runs `.github/workflows/release.yml`:

1. It builds the binary for x86-64 and ARM64 inside Debian 12, and packs it as a tarball, `.deb`,
   `.rpm` and AppImage. It also builds the Flatpak bundle for x86-64.
2. It publishes them with a `SHA256SUMS` as a GitHub release. The notes are the tag message,
   followed by the list of merged PRs.
3. For stable tags, it pushes the `lituus` and `lituus-bin` AUR packages.

A tag with a `-` (`v0.2.0-rc.1`) is a pre-release and isn't pushed to the AUR. Tag with a message,
which becomes the release notes:

```sh
git fetch origin && git tag -a v0.1.0 origin/main && git push origin v0.1.0
```

Running the workflow by hand (Actions › Release › Run workflow), and every PR that changes the
packaging, is a dry run: everything is built and uploaded as workflow artifacts, and nothing is
published. `packaging/aur/test.sh` builds both AUR packages in a clean Arch container.

## Website

This site is built with [Astro](https://astro.build) and [Starlight](https://starlight.astro.build)
from `website/`, and deployed to GitHub Pages from `main`:

```sh
cd website
npm ci          # exact versions from package-lock.json, no install scripts (.npmrc)
npm run dev     # http://localhost:4321/
npm run build && npm run check-links
```

New npm versions follow the same two-week rule as crates (`npm install --before=<date>`).
