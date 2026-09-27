import init, { dump_boxes } from "../npm/pkg/isobmff_wasm.js";

const ready = init();

self.onmessage = async ({ data }) => {
  try {
    await ready;
    // A wasm-bindgen object does not survive postMessage: its fields are
    // getters over wasm memory, so each is copied into a plain object.
    const boxes = dump_boxes(data.file).map((record) => {
      const box = {
        boxType: record.box_type,
        offset: record.offset,
        size: record.size,
        depth: record.depth,
      };
      record.free();
      return box;
    });
    self.postMessage({ reply: "dump", boxes });
  } catch (error) {
    self.postMessage({ reply: "error", message: error.message ?? String(error) });
  }
};
