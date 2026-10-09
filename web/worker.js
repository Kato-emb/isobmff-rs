import init, { Output, demux, dump_boxes, remux } from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";

const ready = init();
const OUTPUT_DIRECTORY = "isobmff-rs-remux";

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

async function written({ file, output, name }) {
  const root = await navigator.storage.getDirectory();
  // The directory, not the root, is emptied: the origin is shared by every
  // site published under the same account.
  await root.removeEntry(OUTPUT_DIRECTORY, { recursive: true }).catch((error) => {
    if (error.name !== "NotFoundError") {
      throw error;
    }
  });
  const outputs = await root.getDirectoryHandle(OUTPUT_DIRECTORY, { create: true });
  const fileHandle = await outputs.getFileHandle(name, { create: true });
  const handle = await fileHandle.createSyncAccessHandle();
  try {
    remux(file, Output[output], handle);
  } finally {
    handle.close();
  }
  return fileHandle.getFile();
}

self.onmessage = async ({ data }) => {
  if (data.request === "remux") {
    try {
      await ready;
      self.postMessage({ file: await written(data) });
    } catch (error) {
      self.postMessage({ error: error.message ?? String(error) });
    }
    return;
  }
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
