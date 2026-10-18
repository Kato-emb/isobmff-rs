import { MovieSampleExtentFsm } from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";
import { boxTypeName } from "./dump.js";
import { EDITS } from "./edits.js";

const CUT_LENGTH = 1024 * 1024;

/**
 * Returns the tracks of the movie `blob` carries and the samples its tables declare
 *
 * Resolves to { tracks, samples }: the tracks in the order the movie declares
 * them, the samples of the movie in the order their bytes lie in the file,
 * then those of each fragment in the order it declares them. A track is
 * { track_id, handler_type, timescale, duration, sample_entries, edits },
 * duration undefined where it is indeterminate and edits the value each of
 * `EDITS` reads from the track, keyed by its `key`; a sample is { track_id,
 * decode_time, sample_duration, sample_composition_time_offset, sync,
 * sample_description_index, offset, size }. Every 64-bit value is a BigInt.
 * Rejects with the library's Error, whose `kind` names the layer that
 * refused, a fragment ahead of the movie among them, or the exception the
 * browser threw.
 *
 * @param {Blob} blob
 * @returns {Promise<{ tracks: object[], samples: object[] }>}
 */
export async function demux(blob) {
  const fsm = new MovieSampleExtentFsm();
  try {
    const samples = [];
    for (let wanted; (wanted = fsm.wantedInput()); ) {
      const start = Number(wanted.offset);
      const length = wanted.length === undefined ? CUT_LENGTH : Math.min(CUT_LENGTH, Number(wanted.length));
      const cut = new Uint8Array(await blob.slice(start, start + length).arrayBuffer());
      if (cut.length === 0) {
        fsm.finish();
      } else {
        fsm.handleInput(wanted.offset, cut);
      }
      for (let extent; (extent = fsm.pollOutput()); ) {
        samples.push(sample(extent));
      }
    }
    const movie = fsm.movie();
    const mvhd = movie.mvhd();
    const movieTimescale = mvhd.timescale();
    const traks = movie.trak();
    mvhd.free();
    movie.free();
    return { tracks: traks.map((trak) => track(trak, movieTimescale)), samples };
  } finally {
    // Why not leave it to the FinalizationRegistry: it holds the movie and its
    // sample tables, the collector does not see wasm memory, and wasm memory
    // never shrinks, so each file read would grow it by that much.
    fsm.free();
  }
}

function track(trak, movieTimescale) {
  const tkhd = trak.tkhd();
  const mdia = trak.mdia();
  const mdhd = mdia.mdhd();
  const hdlr = mdia.hdlr();
  const minf = mdia.minf();
  const stbl = minf.stbl();
  const stsd = stbl.stsd();
  const entries = stsd.entries();
  try {
    return {
      track_id: tkhd.trackId(),
      handler_type: hdlr.handlerType(),
      timescale: mdhd.timescale(),
      duration: mdhd.duration(),
      sample_entries: entries.map((entry) => boxTypeName(entry.boxType())),
      edits: Object.fromEntries(EDITS.map((edit) => [edit.key, edit.read(trak, movieTimescale)])),
    };
  } finally {
    for (const handle of [trak, tkhd, mdia, mdhd, hdlr, minf, stbl, stsd, ...entries]) {
      handle.free();
    }
  }
}

function sample({ properties, extent }) {
  return {
    track_id: properties.trackId,
    decode_time: properties.decodeTime,
    sample_duration: properties.sampleDuration,
    sample_composition_time_offset: properties.sampleCompositionTimeOffset,
    sync: !properties.sampleFlags.sampleIsNonSyncSample,
    sample_description_index: properties.sampleDescriptionIndex,
    offset: extent.start,
    size: extent.end - extent.start,
  };
}
