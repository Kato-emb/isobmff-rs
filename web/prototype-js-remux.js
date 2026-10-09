// PROTOTYPE — throwaway. The non-fragmented remux written in JavaScript over the
// prototype binding: the read loop and every policy live here, the wasm only
// wraps the library. Mirrors crates/isobmff-wasm/src/remux.rs::remux_to_non_fragmented.
import { MovieBox, MovieDemuxFsm, NonFragmentedMuxFsm } from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";

const CUT_LENGTH = 1024 * 1024;

export async function remuxToNonFragmented(file, nowSeconds, { touchData = false } = {}) {
  const demux = new MovieDemuxFsm();
  let refused = null;
  const handWanted = async (offset) => {
    const start = Number(offset);
    const cut = new Uint8Array(await file.slice(start, start + CUT_LENGTH).arrayBuffer());
    try {
      if (cut.length === 0) demux.finish();
      else demux.handle_input(offset, cut);
    } catch (error) {
      refused = error;
    }
  };
  let source = demux.movie();
  while (!source) {
    if (refused) throw refused;
    const offset = demux.wanted_offset();
    if (offset === undefined) throw new Error("the file carries no movie");
    await handWanted(offset);
    source = demux.movie();
  }
  const tracks = source.trak().map((track) => {
    track.set_tkhd(track.tkhd().with_modification_time(nowSeconds));
    const mdia = track.mdia();
    mdia.set_mdhd(mdia.mdhd().with_modification_time(nowSeconds));
    track.set_mdia(mdia);
    return track;
  });
  const movie = MovieBox.create(source.mvhd().with_modification_time(nowSeconds), tracks);
  const mux = new NonFragmentedMuxFsm();
  const fileType = demux.file_type();
  if (fileType) mux.handle_file_type(fileType);
  mux.handle_movie(movie);
  const written = [];
  const drain = () => {
    for (let bytes; (bytes = mux.poll_output()) !== undefined; ) written.push(bytes);
  };
  let chunk = null;
  let samples = 0;
  for (;;) {
    for (let sample; (sample = demux.poll_sample()) !== undefined; ) {
      samples += 1;
      const describedBy = `${sample.track_id}:${sample.sample_description_index}`;
      if (touchData) sample.data();
      if (chunk !== describedBy) {
        if (chunk !== null) {
          mux.finish_chunk();
          drain();
        }
        chunk = describedBy;
        mux.begin_chunk();
      }
      mux.handle_sample(sample);
    }
    if (refused) throw refused;
    const offset = demux.wanted_offset();
    if (offset === undefined) break;
    await handWanted(offset);
  }
  if (chunk !== null) mux.finish_chunk();
  mux.finish();
  drain();
  demux.free();
  mux.free();
  return { bytes: new Blob(written), samples };
}
