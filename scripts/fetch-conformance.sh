#!/usr/bin/env bash
# Fetches the ISO base media files of MPEG's FileFormatConformance, each with
# the box structure GPAC reports for it, into the directory given as $1.
#
#   scripts/fetch-conformance.sh target/conformance
#   ISOBMFF_CONFORMANCE_DIR=target/conformance cargo nextest run --run-ignored only
set -euo pipefail

commit=767a5bcb9fa4841d1b3bb0ca112a6e7d4c6bf067
path=data/file_features/published/isobmff
# The compressed boxes of ISO/IEC 14496-12 §8.16, fetched into a directory of
# their own: they are under consideration, not published.
compressed_path=data/file_features/under_consideration/compressed_boxes
# Files above this hold media data for tens of megabytes, which framing never reads.
max_bytes=2000000
destination=${1:?usage: $0 <directory>}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The files are Git LFS objects: check out the pointers only, then fetch each
# object by URL and check it against the hash its pointer records.
lfs_off=(-c filter.lfs.smudge= -c filter.lfs.process= -c filter.lfs.required=false)
git -C "$work" init -q
git -C "$work" remote add origin https://github.com/MPEGGroup/FileFormatConformance.git
GIT_LFS_SKIP_SMUDGE=1 git -C "$work" "${lfs_off[@]}" fetch -q --depth 1 --filter=blob:none origin "$commit"
git -C "$work" sparse-checkout set --no-cone "$path/*" "$compressed_path/*"
GIT_LFS_SKIP_SMUDGE=1 git -C "$work" "${lfs_off[@]}" checkout -q FETCH_HEAD

fetch() {
  local name=$1
  local from=${2:-$path}
  local into=${3:-$destination}
  local oid
  oid=$(sed -n 's/^oid sha256://p' "$work/$from/$name")
  curl -sSfL -o "$into/$name" \
    "https://media.githubusercontent.com/media/MPEGGroup/FileFormatConformance/$commit/$from/$name"
  echo "$oid  $into/$name" | sha256sum -c --quiet -
}

mkdir -p "$destination"
for pointer in "$work/$path"/*.mp4; do
  name=$(basename "$pointer")
  size=$(sed -n 's/^size //p' "$pointer")
  [ "$size" -le "$max_bytes" ] || continue
  fetch "$name"
  cp "$work/$path/${name%.mp4}_gpac.json" "$destination/"
done
# The media data of a6_tone_multifile.mp4, which its data reference names
# relative to it. The other files a data reference names are not in the set.
fetch myData.dat
mkdir -p "$destination/compressed_boxes"
for pointer in "$work/$compressed_path"/*.mp4; do
  name=$(basename "$pointer")
  fetch "$name" "$compressed_path" "$destination/compressed_boxes"
  cp "$work/$compressed_path/${name%.mp4}_gpac.json" "$destination/compressed_boxes/"
done
