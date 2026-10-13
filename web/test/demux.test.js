import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { initSync } from "../../crates/isobmff-wasm/pkg/isobmff_wasm.js";
import { demux } from "../demux.js";

const wasm = initSync({ module: readFileSync(new URL("../../crates/isobmff-wasm/pkg/isobmff_wasm_bg.wasm", import.meta.url)) });

function box(type, ...payload) {
  const bytes = new Uint8Array(payload.reduce((length, part) => length + part.length, 8));
  new DataView(bytes.buffer).setUint32(0, bytes.length);
  bytes.set(new TextEncoder().encode(type), 4);
  payload.reduce((at, part) => (bytes.set(part, at), at + part.length), 8);
  return bytes;
}

function fullBox(type, flags, ...payload) {
  return box(type, u32(flags), ...payload);
}

function u32(...values) {
  const flat = values.flat();
  const bytes = new Uint8Array(flat.length * 4);
  const view = new DataView(bytes.buffer);
  flat.forEach((value, index) => view.setUint32(index * 4, value));
  return bytes;
}

function movie(chunkOffset, sizes = [4, 6]) {
  return box(
    "moov",
    fullBox("mvhd", 0, u32(0, 0, 90_000, 0), new Uint8Array(76), u32(2)),
    box(
      "trak",
      fullBox("tkhd", 1, u32(0, 0, 1, 0, 0), new Uint8Array(60)),
      box(
        "mdia",
        fullBox("mdhd", 0, u32(0, 0, 90_000, 0, 0x55c4_0000)),
        fullBox("hdlr", 0, u32(0), new TextEncoder().encode("vide"), new Uint8Array(13)),
        box(
          "minf",
          fullBox("vmhd", 1, new Uint8Array(8)),
          box("dinf", fullBox("dref", 0, u32(1), fullBox("url ", 1))),
          box(
            "stbl",
            fullBox("stsd", 0, u32(1), box("avc1", new Uint8Array([0, 0, 0, 0, 0, 0, 0, 1]))),
            fullBox("stts", 0, u32(1, sizes.length, 3000)),
            fullBox("ctts", 0, u32(2, 1, 0, sizes.length - 1, 6000)),
            fullBox("stss", 0, u32(1, 1)),
            fullBox("stsc", 0, u32(1, 1, sizes.length, 1)),
            fullBox("stsz", 0, u32(0, sizes.length, sizes)),
            fullBox("stco", 0, u32(1, chunkOffset)),
          ),
        ),
      ),
    ),
  );
}

const TRACKS = [{ track_id: 1, handler_type: "vide", timescale: 90_000, duration: 0n, sample_entries: ["avc1"] }];

function samples(chunkOffset) {
  const sample = { track_id: 1, sample_duration: 3000, sample_description_index: 1 };
  return [
    { ...sample, decode_time: 0n, sample_composition_time_offset: 0n, sync: true, offset: BigInt(chunkOffset), size: 4n },
    { ...sample, decode_time: 3000n, sample_composition_time_offset: 6000n, sync: false, offset: BigInt(chunkOffset + 4), size: 6n },
  ];
}

test("a movie lists its tracks and the samples its tables declare", async () => {
  const chunkOffset = movie(0).length + 8;

  const demuxed = await demux(new Blob([movie(chunkOffset), box("mdat", new Uint8Array(10))]));

  assert.deepEqual(demuxed, { tracks: TRACKS, samples: samples(chunkOffset) });
});

test("a movie lying after media data longer than a cut lists its samples where they lie", async () => {
  const mediaData = box("mdat", new Uint8Array(3 * 1024 * 1024));

  const demuxed = await demux(new Blob([mediaData, movie(8)]));

  assert.deepEqual(demuxed, { tracks: TRACKS, samples: samples(8) });
});

test("a fragment ahead of any movie is refused with the library's reason", async () => {
  await assert.rejects(demux(new Blob([box("moof")])), { message: "file carries a moof box out of the order its structure keeps" });
});

test("reading a movie of many samples again leaves the wasm memory as it was", async () => {
  const sizes = new Array(200_000).fill(1);
  const chunkOffset = movie(0, sizes).length + 8;
  const file = new Blob([movie(chunkOffset, sizes), box("mdat", new Uint8Array(sizes.length))]);
  await demux(file);
  const used = wasm.memory.buffer.byteLength;

  const demuxed = await demux(file);

  assert.equal(demuxed.samples.length, sizes.length);
  assert.equal(wasm.memory.buffer.byteLength, used);
});
