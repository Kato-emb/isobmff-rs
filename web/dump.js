import { BoxReader, Boxes, CONTAINER_BOXES } from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";

const CUT_LENGTH = 1024 * 1024;

/**
 * The boxes `blob` is formed as, each container followed by the boxes it holds:
 * { box_type, offset, size, depth }, size undefined for a box running to the
 * end of the file.
 */
export async function dump(blob) {
  const containers = new Set(CONTAINER_BOXES().map(String));
  const records = [];
  const reader = new BoxReader();
  let container = null;
  for (let offset = 0; ; offset += CUT_LENGTH) {
    const cut = new Uint8Array(await blob.slice(offset, offset + CUT_LENGTH).arrayBuffer());
    if (cut.length === 0) {
      reader.finish();
    } else {
      reader.handle_input(cut);
    }
    for (let polled; (polled = reader.poll_event()); ) {
      const [extent, event] = polled;
      const header = event.header();
      if (header) {
        records.push(record(header, extent.start, 0));
        container = containers.has(String(header.box_type())) ? { offset: extent.end, payloads: [] } : null;
      } else if (event.end()) {
        if (container) {
          const payload = new Uint8Array(await new Blob(container.payloads).arrayBuffer());
          pushChildren(records, containers, payload, container.offset, 1);
        }
        container = null;
      } else if (container) {
        container.payloads.push(event.payload());
      }
      // Why not leave it to the FinalizationRegistry: an event holds its
      // payload in wasm memory, which never shrinks, and no collection is due
      // while the reader walks an mdat, so it grows to the size of the file.
      event.free();
    }
    if (cut.length === 0) {
      return records;
    }
  }
}

function pushChildren(records, containers, payload, offset, depth) {
  const boxes = new Boxes(payload);
  for (let child; (child = boxes.next()); ) {
    const header = child.header();
    const listed = record(header, offset, depth);
    records.push(listed);
    if (containers.has(listed.box_type)) {
      pushChildren(records, containers, child.payload(), offset + BigInt(header.encoded_len()), depth + 1);
    }
    offset += listed.size ?? 0n;
  }
}

function record(header, offset, depth) {
  return { box_type: String(header.box_type()), offset, size: header.size().total_bytes(), depth };
}
