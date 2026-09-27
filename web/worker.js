import init, { demux, dump_boxes } from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";

const ready = init();

// A wasm-bindgen object does not survive postMessage: its fields are getters
// over wasm memory, so each record is copied out into a plain object.
function plain(record) {
  const copy = record.toJSON();
  record.free();
  return copy;
}

self.onmessage = async ({ data }) => {
  try {
    await ready;
    if (data.request === "dump") {
      self.postMessage({ reply: "dump", boxes: dump_boxes(data.file).map(plain) });
    } else {
      const demuxed = demux(data.file);
      const tracks = demuxed.tracks.map(plain);
      const samples = demuxed.samples.map(plain);
      demuxed.free();
      self.postMessage({ reply: "demux", tracks, samples });
    }
  } catch (error) {
    self.postMessage({ reply: "error", message: error.message ?? String(error) });
  }
};
