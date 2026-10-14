import { EditBox, EditListBox, TrackBox } from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";

const FIXED_16_16_ONE = 0x1_0000;
const FIXED_2_30_ONE = 0x4000_0000;
const ROTATIONS = new Map([
  [0, [FIXED_16_16_ONE, 0, 0, FIXED_16_16_ONE]],
  [90, [0, FIXED_16_16_ONE, -FIXED_16_16_ONE, 0]],
  [180, [-FIXED_16_16_ONE, 0, 0, -FIXED_16_16_ONE]],
  [270, [0, -FIXED_16_16_ONE, FIXED_16_16_ONE, 0]],
]);
const SECONDS = { type: "seconds", digits: 3, format: (seconds) => `${seconds.toFixed(3)} s` };
const EDIT_LIST_NOTE = "Some players ignore the edit list of a fragmented MP4, and play its media from 0.";

/**
 * The changes Remux makes to a track, each stated once for the form that offers it and for the remux that writes it
 *
 * An edit is { key, label, help, note, applies, control, read, write }:
 * `note`, where there is one, is what to know of the output once the edit is
 * changed; `applies` takes a track as `demux` lists it and says whether the
 * form offers the edit; `control` is { type: "choice", values, format } or
 * { type: "seconds", digits, format }, `format` showing a value and `digits`
 * the decimals a value in seconds is stated to; `read(trak, movieTimescale)`
 * returns the value the source track holds, or null where it holds one the
 * control cannot state; and `write(trak, values, movieTimescale)` returns the
 * track carrying `values`, an object of every edit sharing that `write`,
 * keyed by `key`.
 */
export const EDITS = [
  {
    key: "rotation",
    label: "Rotation",
    help: "Turns the picture clockwise as it is shown, by the matrix of the track header; the samples are not encoded again.",
    applies: (track) => track.handler_type === "vide",
    control: { type: "choice", values: [...ROTATIONS.keys()], format: (degrees) => `${degrees}°` },
    read: (trak) => {
      const tkhd = trak.tkhd();
      try {
        return rotationOf(tkhd.matrix());
      } finally {
        tkhd.free();
      }
    },
    write: writeRotation,
  },
  {
    key: "starting_offset",
    label: "Start delay",
    help: "How far into the movie the track starts, an empty edit ahead of its media.",
    note: EDIT_LIST_NOTE,
    applies: () => true,
    control: SECONDS,
    read: (trak, movieTimescale) => readEditList(trak, (elst) => elst.startingOffset(), movieTimescale),
    write: writeEditList,
  },
  {
    key: "media_time",
    label: "Media start",
    help: "The time in the media the track starts playing from; what lies before it is not shown.",
    note: EDIT_LIST_NOTE,
    applies: () => true,
    control: SECONDS,
    read: (trak) => readEditList(trak, (elst) => elst.mediaTime(), mediaTimescale(trak)),
    write: writeEditList,
  },
];

/**
 * Returns `trak` carrying the edits `changes` states, keyed by their `key`, or `trak` itself where no edit makes the track anew
 *
 * A track made anew is a root of its own; `trak` is left as it was but for
 * its `tkhd`, which a rotation sets in place. A rotation turns the matrix
 * with no translation. A start delay and a media start of 0 each drop the
 * `edts`, and with it every box of the track other than its `tkhd` and
 * `mdia`.
 *
 * @param {TrackBox} trak
 * @param {Record<string, number>} changes
 * @param {number} movieTimescale
 * @returns {TrackBox}
 */
export function applyEdits(trak, changes, movieTimescale) {
  const writes = new Set(EDITS.filter((edit) => edit.key in changes).map((edit) => edit.write));
  let written = trak;
  for (const write of writes) {
    const values = Object.fromEntries(
      EDITS.filter((edit) => edit.write === write).map((edit) => [
        edit.key,
        edit.key in changes ? changes[edit.key] : edit.read(written, movieTimescale),
      ]),
    );
    const next = write(written, values, movieTimescale);
    if (written !== trak && written !== next) {
      written.free();
    }
    written = next;
  }
  return written;
}

/**
 * Returns the clockwise rotation, in degrees, the matrix `matrix` turns the picture by, its translation aside, or null where it does more than turn and move it
 *
 * @param {Int32Array} matrix
 * @returns {number | null}
 */
export function rotationOf(matrix) {
  const [a, b, u, c, d, v, , , w] = matrix;
  if (u !== 0 || v !== 0 || w !== FIXED_2_30_ONE) {
    return null;
  }
  for (const [degrees, rotation] of ROTATIONS) {
    if (rotation.every((value, index) => value === [a, b, c, d][index])) {
      return degrees;
    }
  }
  return null;
}

function writeRotation(trak, { rotation }) {
  const [a, b, c, d] = ROTATIONS.get(rotation);
  const tkhd = trak.tkhd();
  const rotated = tkhd.withMatrix(Int32Array.of(a, b, 0, c, d, 0, 0, 0, FIXED_2_30_ONE));
  try {
    trak.setTkhd(rotated);
    return trak;
  } finally {
    tkhd.free();
    rotated.free();
  }
}

function readEditList(trak, read, timescale) {
  const edts = trak.edts();
  if (!edts) {
    return 0;
  }
  const elst = edts.elst();
  edts.free();
  if (!elst) {
    return 0;
  }
  try {
    const ticks = read(elst);
    return ticks === undefined ? null : Number(ticks) / timescale;
  } finally {
    elst.free();
  }
}

function writeEditList(trak, { starting_offset, media_time }, movieTimescale) {
  const startingOffset = ticks(starting_offset, movieTimescale);
  const mediaTime = ticks(media_time, mediaTimescale(trak));
  if (startingOffset === 0n && mediaTime === 0n) {
    const tkhd = trak.tkhd();
    const mdia = trak.mdia();
    try {
      return TrackBox.new(tkhd, mdia);
    } finally {
      tkhd.free();
      mdia.free();
    }
  }
  const elst = EditListBox.fromStartingOffset(startingOffset, mediaTime);
  const empty = new EditBox();
  const edts = empty.withElst(elst);
  try {
    return trak.withEdts(edts);
  } finally {
    for (const held of [elst, empty, edts]) {
      held.free();
    }
  }
}

function mediaTimescale(trak) {
  const mdia = trak.mdia();
  const mdhd = mdia.mdhd();
  try {
    return mdhd.timescale();
  } finally {
    mdia.free();
    mdhd.free();
  }
}

function ticks(seconds, timescale) {
  return BigInt(Math.round((seconds ?? 0) * timescale));
}
