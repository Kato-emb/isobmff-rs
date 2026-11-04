#!/usr/bin/env bash
# Fails where a layer of crates/isobmff (framing, structure_data, media_data,
# timeline) has an item visible across the crate outside `#[cfg(test)]`:
# `pub(crate)`, `pub(in crate)`, or a `pub(super)` at the top of the layer's
# own file, whose parent is the crate root. Fails too where the demuxer, the
# muxer or fast start imports from the crate through a private module. A
# layer's items are either public, the specification's behaviour, or private to
# the layer, so that the demuxer, the muxer and fast start are written from the
# public API alone, as another reader or writer would be.
#
#   scripts/check-layer-visibility.sh
set -euo pipefail

src=crates/isobmff/src

# Lists the files of each module named: its own file and those under it
modules() {
  for module; do
    [ -f "$src/$module.rs" ] && echo "$src/$module.rs"
    [ -d "$src/$module" ] && find "$src/$module" -name '*.rs' | sort
  done
  return 0
}
mapfile -t files < <(modules framing structure_data media_data timeline)

# An item after `#[cfg(test)]` is skipped: a line ending in `;` alone, or a
# block through the brace that closes it.
found=$(awk -v src="$src" '
  FNR == 1 { pending = 0; depth = 0; layer_root = (FILENAME ~ ("^" src "/[a-z_]+\\.rs$")) }
  depth > 0 {
    depth += gsub(/\{/, "{") - gsub(/\}/, "}")
    next
  }
  /^[[:space:]]*#\[cfg\(test\)\]/ { pending = 1; next }
  pending && /^[[:space:]]*#\[/ { next }
  pending {
    pending = 0
    opened = gsub(/\{/, "{") - gsub(/\}/, "}")
    if (opened > 0) depth = opened
    next
  }
  /pub\((crate|in crate)\)/ || (layer_root && /^pub\(super\)/) { print FILENAME ":" FNR ": " $0 }
' "${files[@]}")

status=0
if [ -n "$found" ]; then
  echo "item visible across the crate in a layer, outside #[cfg(test)]: make it pub, or private to the layer" >&2
  echo "$found" >&2
  status=1
fi

# The demuxer, the muxer and fast start import from the crate by public paths
# only: a path through a module lib.rs keeps private is refused, the items
# lib.rs re-exports being imported from the crate root.
# `convert` is excluded: it is no layer, but a utility of the crate's own, the
# conversions of numbers in memory to the numbers of a file, which the layers
# and the compositions alike use.
utilities='convert'
private_modules=$(sed -n 's/^mod \([a-z_]*\);$/\1/p' "$src/lib.rs" | grep -vxE "$utilities" | paste -sd '|' -)
mapfile -t compositions < <(modules demuxer muxer fast_start)
# Why not skip each `#[cfg(test)]` item as above: imports open a file, and a
# composition's tests come after its code. A `use super::` reaches the crate
# root from a file directly under src, and a `use super::super::` from a file
# one directory below; fewer `super::` stay inside the composition.
private_paths=$(awk -v src="$src" -v private="^(${private_modules})$" '
  FNR == 1 {
    done_file = 0; statement = ""
    nesting = split(substr(FILENAME, length(src) + 2), segments, "/")
  }
  done_file { next }
  /^#\[cfg\(test\)\]/ { done_file = 1; next }
  statement == "" && /^[[:space:]]*(pub )?use (crate|super)::/ { statement = $0; at = FNR }
  statement != "" && FNR > at { statement = statement " " $0 }
  statement != "" && /;/ {
    path = statement
    sub(/^[[:space:]]*(pub )?use /, "", path)
    sub(/;.*/, "", path)
    gsub(/[{}[:space:]]/, "", path)
    if (!sub(/^crate::/, "", path)) {
      for (up = 0; sub(/^super::/, "", path); up++) {}
      if (up < nesting) { statement = ""; next }
    }
    count = split(path, items, ",")
    for (i = 1; i <= count; i++) {
      first = items[i]
      sub(/::.*/, "", first)
      if (first ~ private) print FILENAME ":" at ": crate::" items[i]
    }
    statement = ""
  }
' "${compositions[@]}")
if [ -n "$private_paths" ]; then
  echo "import through a private module in a composition: import it from its public path" >&2
  echo "$private_paths" >&2
  status=1
fi
exit "$status"
