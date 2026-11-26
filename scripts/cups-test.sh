#!/usr/bin/env bash
# lituus-print against a real cupsd: CUPS in an Arch container with virtual driverless printers
# (ippeveprinter), its socket shared with the host. Run with --userns=keep-id, so cupsd sees the
# host's uid on the socket, as PeerCred needs: the host's user is an admin while it's in wheel.
#
#   scripts/cups-test.sh            crates/print/tests/cups.rs, as an admin and as a plain user
#   scripts/cups-test.sh up DIR     start it and leave it running; then CUPS_SERVER=DIR/cups.sock
#                                   (and DBUS_SYSTEM_BUS_ADDRESS=unix:path=DIR/system_bus_socket)
#   scripts/cups-test.sh down DIR   stop it
#   scripts/cups-test.sh ci         the same tests in CI's Arch container, as root and as a plain user
#
# The image (localhost/lituus-cups-test) is made on first use; `podman rmi localhost/lituus-cups-test`
# removes it.
set -euo pipefail
cd "$(dirname "$0")/.."
image=localhost/lituus-cups-test
me=$(id -un)
# The container the commands run in; none in CI, which is the container.
c=""

inside() { # script [env assignments...]
  local script=$1
  shift
  if [ -n "$c" ]; then
    local envs=()
    for e in "$@"; do envs+=(-e "$e"); done
    podman exec "${envs[@]}" "$c" sh -euc "$script"
  else
    env "$@" sh -euc "$script"
  fi
}

build_image() {
  podman image exists "$image" && return
  podman rm -f lituus-cups-build >/dev/null 2>&1 || true
  podman run --name lituus-cups-build docker.io/library/archlinux:latest \
    pacman -Syu --noconfirm --needed cups cups-filters cups-pdf dbus >/dev/null
  podman commit -q lituus-cups-build "$image" >/dev/null
  podman rm lituus-cups-build >/dev/null
}

# cupsd listening on /lituus/cups.sock too, a system bus at /lituus/system_bus_socket, and a
# virtual printer on port 8631.
start_cups() {
  inside '
    grep -q "^Listen /lituus/cups.sock" /etc/cups/cupsd.conf || echo "Listen /lituus/cups.sock" >> /etc/cups/cupsd.conf
    mkdir -p /run/dbus
    sed -i "s|<listen>.*</listen>|<listen>unix:path=/lituus/system_bus_socket</listen>|" /usr/share/dbus-1/system.conf
    dbus-daemon --system --fork
    ln -sf /lituus/system_bus_socket /run/dbus/system_bus_socket
    cupsd
    (setsid ippeveprinter -2 -k -d /tmp/ippeve -p 8631 -n localhost -r off -M Lituus -m "Virtual Duplex" \
      -f application/pdf,image/pwg-raster,image/urf,image/jpeg -s 20,10 "Virtual Duplex" >/tmp/ippeve.log 2>&1 &)
    for _ in $(seq 100); do
      [ -S /lituus/cups.sock ] && (exec 3<>/dev/tcp/127.0.0.1/8631) 2>/dev/null && exit 0
      sleep 0.1
    done
    echo "cupsd or ippeveprinter did not start" >&2
    exit 1
  '
}

# The printers the tests expect: Virtual (driverless, the default), Held (jobs wait), PDF (cups-pdf),
# Spare (to remove).
reset_printers() {
  inside '
    for p in $(lpstat -e 2>/dev/null); do lpadmin -x "$p"; done
    lpadmin -p Virtual -E -v ipp://localhost:8631/ipp/print -m everywhere -D "Virtual Duplex" -L "Lab"
    lpadmin -p Held -E -v ipp://localhost:8631/ipp/print -m everywhere -D "Held printer"
    cupsdisable Held
    lpadmin -p PDF -E -v cups-pdf:/ -m CUPS-PDF_opt.ppd -D "Print to PDF" 2>/dev/null
    lpadmin -p Spare -E -v ipp://localhost:8631/ipp/print -m everywhere
    lpadmin -d Virtual
    cancel -a -x 2>/dev/null || true
  '
}

# Puts the user in wheel (CUPS's SystemGroup on Arch) or takes them out of it.
admin() { # user on|off
  inside '
    awk -F: -v u="$U" -v on="$ON" "BEGIN { OFS = \":\" }
      \$1 == \"wheel\" { n = split(\$4, a, \",\"); s = \"\"
        for (i = 1; i <= n; i++) if (a[i] != u && a[i] != \"\") s = s == \"\" ? a[i] : s \",\" a[i]
        if (on == \"on\") s = s == \"\" ? u : s \",\" u
        \$4 = s }
      { print }" /etc/group > /tmp/group
    cat /tmp/group > /etc/group
  ' U="$1" ON="$2"
}

up() {
  local dir=$1
  c="lituus-cups-$(basename "$dir")"
  build_image
  mkdir -p "$dir"
  chmod 755 "$dir"
  podman run -d --rm --name "$c" --memory 1500m --userns=keep-id --user 0 -v "$dir:/lituus" "$image" sleep infinity >/dev/null
  start_cups
  admin "$me" on
  reset_printers
  echo "cupsd is up: CUPS_SERVER=$dir/cups.sock DBUS_SYSTEM_BUS_ADDRESS=unix:path=$dir/system_bus_socket"
}

down() {
  local dir=$1
  podman rm -f "lituus-cups-$(basename "$dir")" >/dev/null 2>&1 || true
  # The sockets belong to the container's root: removed as that user.
  podman unshare rm -rf "$dir"
}

case "${1:-test}" in
  up) up "$2" ;;
  down) down "$2" ;;
  test)
    shift || true
    dir=$(mktemp -d /tmp/lituus-cups.XXXXXX)
    trap 'down "$dir"' EXIT
    up "$dir"
    export LITUUS_CUPS_SOCKET="$dir/cups.sock"
    LITUUS_CUPS_ADMIN=1 cargo test -p lituus-print --test cups "$@" -- --test-threads 1
    reset_printers
    admin "$me" off
    LITUUS_CUPS_ADMIN=0 cargo test -p lituus-print --test cups "$@" -- --test-threads 1
    ;;
  ci)
    # As root in CI's container: root is one of CUPS's admins; "plain" isn't.
    mkdir -p /lituus
    start_cups
    reset_printers
    export LITUUS_CUPS_SOCKET=/lituus/cups.sock
    LITUUS_CUPS_ADMIN=1 cargo test --locked -p lituus-print --test cups -- --test-threads 1
    reset_printers
    id plain >/dev/null 2>&1 || useradd -m plain
    bin=$(cargo test --locked -p lituus-print --test cups --no-run --message-format=json |
      sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' | tail -1)
    install -m 755 "$bin" /tmp/cups-tests
    runuser -u plain -- env LITUUS_CUPS_SOCKET=/lituus/cups.sock LITUUS_CUPS_ADMIN=0 /tmp/cups-tests --test-threads 1
    ;;
  *) echo "usage: $0 [test [cargo args] | up DIR | down DIR | ci]" >&2; exit 2 ;;
esac
