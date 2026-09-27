import init, { demux, dump_boxes } from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";

const ready = init();

// A wasm-bindgen object does not survive postMessage: its fields are getters
// over wasm memory, so each record is copied out into a plain object.
function plain(record) {
  const copy = record.toJSON();
  record.free();
  return copy;
}

function attempt(read) {
  try {
    return { value: read() };
  } catch (error) {
    return { error: error.message ?? String(error) };
  }
}

self.onmessage = async ({ data }) => {
  try {
    await ready;
  } catch (error) {
    const failed = { error: error.message ?? String(error) };
    self.postMessage({ boxes: failed, movie: failed });
    return;
  }
  const boxes = attempt(() => dump_boxes(data.file).map(plain));
  const movie = attempt(() => {
    const demuxed = demux(data.file);
    const value = { tracks: demuxed.tracks.map(plain), samples: demuxed.samples.map(plain) };
    demuxed.free();
    return value;
  });
  self.postMessage({ boxes, movie });
};
