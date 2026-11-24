#!/usr/bin/env bash
# Writes a version into the workspace before a release build:   scripts/set-version.sh 0.4.0
# Cargo.toml keeps the placeholder 0.0.0-dev in git; the release workflow and the AUR package
# run this with the version from the tag, and nothing they change is committed.
# For a release (no "-" in the version), the AppStream metainfo also gets a <release> entry if it
# has none: its description comes from $RELEASE_NOTES (the tag message, in Markdown), or stays
# empty. AppStream has paragraphs, lists, <em> and <code> only, so the Markdown is turned into
# those: **bold** and *italic* become <em>, a link keeps its text, a heading is a paragraph, and
# nested lists are flattened. scripts/set-version-test.sh checks it.
set -euo pipefail
cd "$(dirname "$0")/.."
version="${1:?usage: set-version.sh VERSION}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; then
  echo "set-version.sh: $version is not a semantic version" >&2
  exit 1
fi
placeholder='version = "0.0.0-dev"'
grep -qx "$placeholder" Cargo.toml || { echo "set-version.sh: Cargo.toml has no $placeholder" >&2; exit 1; }
# The workspace members are the only packages at 0.0.0-dev, in Cargo.toml and Cargo.lock alike.
# (-i.orig: macOS's sed needs a suffix.)
sed -i.orig "s/^$placeholder\$/version = \"$version\"/" Cargo.toml Cargo.lock
rm Cargo.toml.orig Cargo.lock.orig

# The app's AppStream file, if it has one.
metainfo=$(ls packaging/linux/*.metainfo.xml 2>/dev/null | head -1)
[ -n "$metainfo" ] || exit 0
if [[ "$version" == *-* ]] || grep -q "<release version=\"$version\"" "$metainfo"; then
  exit 0
fi
entry=$(printf '%s\n' "${RELEASE_NOTES:-}" | awk -v version="$version" -v date="$(date -u +%F)" '
  function esc(s) { gsub(/&/, "\\&amp;", s); gsub(/</, "\\&lt;", s); gsub(/>/, "\\&gt;", s); return s }
  # A line of Markdown as AppStream text: `code` as <code>, and the rest through prose().
  function inline(s,   out, at, n) {
    out = ""
    while (match(s, /`[^`]+`/)) {
      # prose() matches too, which moves RSTART and RLENGTH.
      at = RSTART; n = RLENGTH
      out = out prose(substr(s, 1, at - 1)) "<code>" esc(substr(s, at + 1, n - 2)) "</code>"
      s = substr(s, at + n)
    }
    return out prose(s)
  }
  function prose(s,   text) {
    # [text](url) keeps its text, <url> its URL.
    while (match(s, /\[[^]]*\]\([^)]*\)/)) {
      text = substr(s, RSTART + 1, RLENGTH - 1)
      s = substr(s, 1, RSTART - 1) substr(text, 1, index(text, "](") - 1) substr(s, RSTART + RLENGTH)
    }
    while (match(s, /<https?:[^> ]*>/)) s = substr(s, 1, RSTART - 1) substr(s, RSTART + 1, RLENGTH - 2) substr(s, RSTART + RLENGTH)
    s = esc(s)
    # **bold** and *italic*, both <em>: a mark next to a space (as in 2 * 3 * 4) is no emphasis.
    while (match(s, /\*\*[^ *]([^*]*[^ *])?\*\*/)) s = em(s, 2)
    while (match(s, /\*[^ *]([^*]*[^ *])?\*/)) s = em(s, 1)
    return s
  }
  function em(s, n) { return substr(s, 1, RSTART - 1) "<em>" substr(s, RSTART + n, RLENGTH - 2 * n) "</em>" substr(s, RSTART + RLENGTH) }
  function close_item() { if (item != "") { body = body "          <li>" inline(item) "</li>\n"; item = "" } }
  function close_block() {
    if (para != "") { body = body "        <p>" inline(para) "</p>\n"; para = "" }
    close_item()
    if (list != "") { body = body "        </" list ">\n"; list = "" }
  }
  { sub(/[ \t\r]+$/, "") }
  # Rules and code fences carry no text.
  /^[ \t]*[-*_][ \t]*[-*_][ \t]*[-*_][-*_ \t]*$/ || /^[ \t]*```/ { close_block(); next }
  /^[ \t]*#+[ \t]/ {
    close_block()
    sub(/^[ \t]*#+[ \t]+/, ""); sub(/[ \t]+#+$/, "")
    para = $0
    close_block()
    next
  }
  /^[ \t]*([-*+]|[0-9]+[.)])[ \t]/ {
    kind = ($0 ~ /^[ \t]*[-*+]/) ? "ul" : "ol"
    if (para != "" || list != kind) close_block()
    close_item()
    if (list == "") { body = body "        <" kind ">\n"; list = kind }
    sub(/^[ \t]*([-*+]|[0-9]+[.)])[ \t]+/, "")
    item = $0
    next
  }
  /^$/ { close_block(); next }
  # A line right under an item continues it.
  {
    sub(/^[ \t]+/, "")
    if (item != "") item = item " " $0
    else para = (para == "" ? $0 : para " " $0)
  }
  END {
    close_block()
    if (body == "") print "    <release version=\"" version "\" date=\"" date "\"/>"
    else printf "    <release version=\"%s\" date=\"%s\">\n      <description>\n%s      </description>\n    </release>\n", version, date, body
  }')
# The newest release goes first.
ENTRY="$entry" awk '{ print } /<releases>/ && !done { print ENVIRON["ENTRY"]; done = 1 }' "$metainfo" >"$metainfo.new"
mv "$metainfo.new" "$metainfo"
