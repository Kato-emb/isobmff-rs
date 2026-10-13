import { MovieSampleExtentFsm } from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";

const CUT_LENGTH = 1024 * 1024;

/**
 * Returns the tracks of the movie `blob` carries and the samples its tables declare
 *
 * Resolves to { tracks, samples }: the tracks in the order the movie declares
 * them, the samples of the movie in the order their bytes lie in the file,
 * then those of each fragment in the order it declares them. A track is
 * { track_id, handler_type, timescale, duration, sample_entries }, duration
 * undefined where it is indeterminate; a sample is { track_id, decode_time,
 * sample_duration, sample_composition_time_offset, sync,
 * sample_description_index, offset, size }. Every 64-bit value is a BigInt.
 * Rejects with an Error stating the reason the library refuses the file, a
 * fragment ahead of the movie among them.
 *
 * @param {Blob} blob
 * @returns {Promise<{ tracks: object[], samples: object[] }>}
 */
export async function demux(blob) {
  const fsm = new MovieSampleExtentFsm();
  const samples = [];
  try {
    for (let offset = 0; ; offset += CUT_LENGTH) {
      const cut = new Uint8Array(await blob.slice(offset, offset + CUT_LENGTH).arrayBuffer());
      if (cut.length === 0) {
        fsm.finish();
      } else {
        fsm.handle_input(BigInt(offset), cut);
      }
      for (let extent; (extent = fsm.poll_sample_extent()); ) {
        samples.push(sample(extent));
      }
      if (cut.length === 0) {
        const movie = fsm.movie();
        const traks = movie.trak();
        movie.free();
        return { tracks: traks.map(track), samples };
      }
    }
  } finally {
    // Why not leave it to the FinalizationRegistry: it holds the movie and its
    // sample tables, the collector does not see wasm memory, and wasm memory
    // never shrinks, so each file read would grow it by that much.
    fsm.free();
  }
}

function track(trak) {
  const mdia = trak.mdia();
  const mdhd = mdia.mdhd();
  const minf = mdia.minf();
  const stbl = minf.stbl();
  const listed = {
    track_id: trak.tkhd().track_id(),
    handler_type: String(mdia.hdlr().handler_type()),
    timescale: mdhd.timescale(),
    duration: mdhd.duration().get(),
    sample_entries: stbl.stsd().entries().map((entry) => String(entry.box_type())),
  };
  // Why not leave these to the FinalizationRegistry: each is a copy of the
  // sample tables of the track, which the collector does not see, so each file
  // read would grow wasm memory by them.
  for (const copy of [stbl, minf, mdia, trak]) {
    copy.free();
  }
  return listed;
}

function sample(extent) {
  const properties = extent.properties();
  const flags = properties.sample_flags;
  const { start, end } = extent.extent();
  const listed = {
    track_id: properties.track_id,
    decode_time: properties.decode_time,
    sample_duration: properties.sample_duration,
    sample_composition_time_offset: properties.sample_composition_time_offset,
    sync: !flags.sample_is_non_sync_sample(),
    sample_description_index: properties.sample_description_index,
    offset: start,
    size: end - start,
  };
  // Why not leave them to the FinalizationRegistry: a file lists its samples by
  // the hundred thousand, three wasm objects each, and the collector does not
  // count wasm memory, so nothing prompts it to run.
  flags.free();
  properties.free();
  extent.free();
  return listed;
}
