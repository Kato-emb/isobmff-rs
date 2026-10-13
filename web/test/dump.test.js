import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { BoxReader, Boxes, initSync } from "../../crates/isobmff-wasm/pkg/isobmff_wasm.js";
import { dump } from "../dump.js";

const wasm = initSync({ module: readFileSync(new URL("../../crates/isobmff-wasm/pkg/isobmff_wasm_bg.wasm", import.meta.url)) });

function box(type, ...payload) {
  const bytes = new Uint8Array(payload.reduce((length, part) => length + part.length, 8));
  new DataView(bytes.buffer).setUint32(0, bytes.length);
  bytes.set(new TextEncoder().encode(type), 4);
  payload.reduce((at, part) => (bytes.set(part, at), at + part.length), 8);
  return bytes;
}

function listed(box_type, offset, size, depth) {
  return { box_type, offset: BigInt(offset), size: size === undefined ? undefined : BigInt(size), depth };
}

test("a container is followed by the boxes it holds, each a level deeper than it", async () => {
  const records = await dump(new Blob([box("ftyp", new Uint8Array(8)), box("moov", box("mvhd", new Uint8Array(4)), box("trak", box("tkhd"))), box("mdat", new Uint8Array(4))]),
  );

  assert.deepEqual(records, [
    listed("ftyp", 0, 16, 0),
    listed("moov", 16, 36, 0),
    listed("mvhd", 24, 12, 1),
    listed("trak", 36, 16, 1),
    listed("tkhd", 44, 8, 2),
    listed("mdat", 52, 12, 0),
  ]);
});

test("a container is read whole however far its payload runs", async () => {
  const records = await dump(new Blob([box("moov", box("free", new Uint8Array(3 * 1024 * 1024)), box("trak"))]));

  assert.deepEqual(records, [
    listed("moov", 0, 3 * 1024 * 1024 + 24, 0),
    listed("free", 8, 3 * 1024 * 1024 + 8, 1),
    listed("trak", 3 * 1024 * 1024 + 16, 8, 1),
  ]);
});

test("a box running to the end of the file has no size, and a uuid box names its user type", async () => {
  const toEndOfFile = box("mdat", new Uint8Array(4));
  toEndOfFile.set([0, 0, 0, 0]);
  const userType = Uint8Array.from({ length: 16 }, (_, index) => index);

  const records = await dump(new Blob([box("uuid", userType), toEndOfFile]));

  assert.deepEqual(records, [
    listed("uuid 00010203-0405-0607-0809-0a0b0c0d0e0f", 0, 24, 0),
    listed("mdat", 24, undefined, 0),
  ]);
});

test("a container whose payload does not divide into boxes is refused by the box layer", async () => {
  const cutShort = box("moov", box("mvhd", new Uint8Array(4)));
  new DataView(cutShort.buffer).setUint32(8, 64);

  await assert.rejects(dump(new Blob([cutShort])), { message: "box of 64 bytes cut short by an input of 12", kind: "box" });
});

test("a box far longer than the wasm memory in use is read without the memory growing to its size", async () => {
  const length = 256 * 1024 * 1024;
  const header = box("mdat");
  new DataView(header.buffer).setUint32(0, 8 + length);

  const records = await dump(new Blob([header, new Uint8Array(length)]));

  assert.deepEqual(records, [listed("mdat", 0, 8 + length, 0)]);
  assert.ok(wasm.memory.buffer.byteLength < length / 8);
});

test("a file ending inside a box is refused by the sequence layer", async () => {
  const truncated = box("free", new Uint8Array(8)).subarray(0, 12);

  await assert.rejects(dump(new Blob([truncated])), { message: "box of 16 bytes closed off at 12", kind: "sequence" });
});

test("the boxes come as plain objects of the shape their interfaces state", () => {
  const extended = new Uint8Array([0, 0, 0, 1, ...new TextEncoder().encode("free"), 0, 0, 0, 0, 0, 0, 0, 17, 9]);
  const userType = Uint8Array.from({ length: 16 }, (_, index) => index);
  const toEndOfFile = box("mdat", new Uint8Array([7]));
  toEndOfFile.set([0, 0, 0, 0]);

  const boxes = new Boxes(new Uint8Array([...box("free", new Uint8Array([1])), ...extended, ...box("uuid", userType), ...toEndOfFile]));

  assert.deepEqual(
    [boxes.next(), boxes.next(), boxes.next(), boxes.next(), boxes.next()],
    [
      { done: false, value: { header: { boxType: { type: "compact", fourCc: "free" }, size: { type: "compact", size: 9 } }, payload: new Uint8Array([1]) } },
      { done: false, value: { header: { boxType: { type: "compact", fourCc: "free" }, size: { type: "extended", size: 17n } }, payload: new Uint8Array([9]) } },
      {
        done: false,
        value: { header: { boxType: { type: "extended", userType: "00010203-0405-0607-0809-0a0b0c0d0e0f" }, size: { type: "compact", size: 24 } }, payload: new Uint8Array() },
      },
      { done: false, value: { header: { boxType: { type: "compact", fourCc: "mdat" }, size: { type: "toEndOfFile" } }, payload: new Uint8Array([7]) } },
      { done: true, value: undefined },
    ],
  );
});

test("the events come as plain objects of the shape their interfaces state", () => {
  const reader = new BoxReader();
  reader.handleInput(box("free", new Uint8Array([1])));
  reader.finish();

  const events = [reader.pollEvent(), reader.pollEvent(), reader.pollEvent(), reader.pollEvent()];
  reader.free();

  assert.deepEqual(events, [
    [{ start: 0n, end: 8n }, { type: "header", header: { boxType: { type: "compact", fourCc: "free" }, size: { type: "compact", size: 9 } } }],
    [{ start: 8n, end: 9n }, { type: "payload", payload: new Uint8Array([1]) }],
    [{ start: 9n, end: 9n }, { type: "end" }],
    undefined,
  ]);
});
