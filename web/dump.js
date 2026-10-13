import { BoxReader, Boxes, CONTAINER_BOXES } from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";

const CUT_LENGTH = 1024 * 1024;

/**
 * Returns the boxes `blob` is formed as, each container followed by the boxes it holds
 *
 * Resolves to records of { box_type, offset, size, depth }, offset and size
 * BigInts, size undefined for a box running to the end of the file. Rejects
 * with the library's Error, whose `kind` names the layer that refused, or
 * the exception the browser threw.
 *
 * @param {Blob} blob
 * @returns {Promise<object[]>}
 */
export async function dump(blob) {
  const containers = new Set(CONTAINER_BOXES().map(boxTypeName));
  const records = [];
  const reader = new BoxReader();
  let container = null;
  try {
    for (let offset = 0; ; offset += CUT_LENGTH) {
      const cut = new Uint8Array(await blob.slice(offset, offset + CUT_LENGTH).arrayBuffer());
      if (cut.length === 0) {
        reader.finish();
      } else {
        reader.handleInput(cut);
      }
      for (let polled; (polled = reader.pollEvent()); ) {
        const [extent, event] = polled;
        if (event.type === "header") {
          const listed = record(event.header, extent.start, 0);
          records.push(listed);
          container = containers.has(listed.box_type) ? { offset: extent.end, payloads: [] } : null;
        } else if (event.type === "payload") {
          container?.payloads.push(event.payload);
        } else {
          if (container) {
            const payload = new Uint8Array(await new Blob(container.payloads).arrayBuffer());
            pushChildren(records, containers, payload, container.offset, 1);
          }
          container = null;
        }
      }
      if (cut.length === 0) {
        return records;
      }
    }
  } finally {
    reader.free();
  }
}

/**
 * Returns the name of a box type as the library displays it: the fourcc, or `uuid` and the user type
 *
 * @param {import("../crates/isobmff-wasm/pkg/isobmff_wasm.js").BoxType} boxType
 * @returns {string}
 */
export function boxTypeName(boxType) {
  return boxType.type === "compact" ? boxType.fourCc : `uuid ${boxType.userType}`;
}

function pushChildren(records, containers, payload, offset, depth) {
  let position = 0;
  for (const child of new Boxes(payload)) {
    const listed = record(child.header, offset, depth);
    records.push(listed);
    const length = Number(listed.size ?? BigInt(payload.length - position));
    if (containers.has(listed.box_type)) {
      pushChildren(records, containers, child.payload, offset + BigInt(length - child.payload.length), depth + 1);
    }
    position += length;
    offset += BigInt(length);
  }
}

function record(header, offset, depth) {
  const { size } = header;
  return {
    box_type: boxTypeName(header.boxType),
    offset,
    size: size.type === "toEndOfFile" ? undefined : BigInt(size.size),
    depth,
  };
}
