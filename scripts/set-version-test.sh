#!/usr/bin/env bash
# Checks the release entry scripts/set-version.sh writes into the AppStream metainfo from a tag
# message in Markdown, with every awk installed of mawk (Debian's, which the release builds use),
# gawk and BusyBox's, and validates the result with appstreamcli if it's there.
#   scripts/set-version-test.sh
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

notes='Lituus 1.2.3: *one* look

- **One look on every page**: the same hero, `Ctrl K`, and <tags> & more.
- **Set up…**: adds the line at the end
  (with a `.bak`) of [your config](https://example.com/a_b).
  - a nested item
1. first
2) second

## Fixes

A paragraph with 2 * 3 * 4 and a <https://lituus.giosuel.io> link
on two lines.

---'

expected='    <release version="1.2.3" date="DATE">
      <description>
        <p>Lituus 1.2.3: <em>one</em> look</p>
        <ul>
          <li><em>One look on every page</em>: the same hero, <code>Ctrl K</code>, and &lt;tags&gt; &amp; more.</li>
          <li><em>Set up…</em>: adds the line at the end (with a <code>.bak</code>) of your config.</li>
          <li>a nested item</li>
        </ul>
        <ol>
          <li>first</li>
          <li>second</li>
        </ol>
        <p>Fixes</p>
        <p>A paragraph with 2 * 3 * 4 and a https://lituus.giosuel.io link on two lines.</p>
      </description>
    </release>'

failed=0
tested=0
for awk in mawk gawk busybox; do
  command -v "$awk" >/dev/null || continue
  tested=$((tested + 1))
  # set-version.sh calls `awk`: this one, first on the PATH (BusyBox picks its applet by name).
  mkdir -p "$tmp/$awk/bin" "$tmp/$awk/tree/scripts" "$tmp/$awk/tree/packaging/linux"
  ln -s "$(command -v "$awk")" "$tmp/$awk/bin/awk"
  cp "$root/Cargo.toml" "$root/Cargo.lock" "$tmp/$awk/tree/"
  cp "$root/scripts/set-version.sh" "$tmp/$awk/tree/scripts/"
  cp "$root"/packaging/linux/*.metainfo.xml "$tmp/$awk/tree/packaging/linux/"
  metainfo=$(ls "$tmp/$awk"/tree/packaging/linux/*.metainfo.xml)
  PATH="$tmp/$awk/bin:$PATH" RELEASE_NOTES="$notes" "$tmp/$awk/tree/scripts/set-version.sh" 1.2.3
  got=$(sed -n '/<release version="1.2.3"/,/<\/release>/p' "$metainfo" | sed 's/date="[0-9-]*"/date="DATE"/')
  if [ "$got" != "$expected" ]; then
    echo "FAIL ($awk): the release entry differs:" >&2
    diff <(printf '%s\n' "$expected") <(printf '%s\n' "$got") >&2 || true
    failed=1
  # Any error or warning but one: a URL in the tag message stays, as text, which AppStream warns
  # about (and doesn't let --override make less).
  elif command -v appstreamcli >/dev/null &&
    issues=$(appstreamcli validate --no-net "$metainfo" 2>&1 | grep -E '^[EW]: ' | grep -v ' description-has-plaintext-url ') ; then
    echo "FAIL ($awk): appstreamcli doesn't accept it:" >&2
    printf '%s\n' "$issues" >&2
    failed=1
  else
    echo "ok ($awk)"
  fi
done
[ "$tested" -gt 0 ] || { echo "FAIL: none of mawk, gawk and busybox is installed" >&2; exit 1; }
exit "$failed"
