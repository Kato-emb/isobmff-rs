import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { MovieSampleExtentFsm, initSync } from "../../crates/isobmff-wasm/pkg/isobmff_wasm.js";
import { demux } from "../demux.js";
import { box, movie } from "./fixtures.js";

const wasm = initSync({ module: readFileSync(new URL("../../crates/isobmff-wasm/pkg/isobmff_wasm_bg.wasm", import.meta.url)) });

const TRACKS = [
  {
    track_id: 1,
    handler_type: "vide",
    timescale: 90_000,
    duration: 0n,
    sample_entries: ["avc1"],
    edits: { rotation: null, starting_offset: 0, media_time: 0 },
  },
];

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

test("a fragment ahead of any movie is refused by the structure layer", async () => {
  await assert.rejects(demux(new Blob([box("moof")])), {
    message: "file carries a moof box out of the order its structure keeps",
    kind: "structure",
  });
});

test("reading a movie of many samples again leaves the wasm memory as it was", async () => {
  const sizes = new Array(200_000).fill(1);
  const chunkOffset = movie(0, { sizes }).length + 8;
  const file = new Blob([movie(chunkOffset, { sizes }), box("mdat", new Uint8Array(sizes.length))]);
  await demux(file);
  const used = wasm.memory.buffer.byteLength;

  const demuxed = await demux(file);

  assert.equal(demuxed.samples.length, sizes.length);
  assert.equal(wasm.memory.buffer.byteLength, used);
});

test("the wanted input and the sample extents come as plain objects of the shape their interfaces state", () => {
  const chunkOffset = movie(0).length + 8;
  const fsm = new MovieSampleExtentFsm();
  const wanted = fsm.wantedInput();
  fsm.handleInput(0n, movie(chunkOffset));

  const extent = fsm.pollOutput();
  const duration = fsm.movie().trak()[0].mdia().mdhd().duration();
  fsm.free();

  assert.deepEqual(wanted, { offset: 0n, length: undefined });
  assert.deepEqual(extent, {
    properties: {
      trackId: 1,
      decodeTime: 0n,
      sampleDuration: 3000,
      sampleCompositionTimeOffset: 0n,
      sampleFlags: {
        sampleDependencyType: { isLeading: "unknown", sampleDependsOn: "unknown", sampleIsDependedOn: "unknown", sampleHasRedundancy: "unknown" },
        paddingBits: 0,
        sampleIsNonSyncSample: false,
        degradationPriority: 0,
      },
      sampleDescriptionIndex: 1,
    },
    dataReferenceIndex: 1,
    extent: { start: BigInt(chunkOffset), end: BigInt(chunkOffset + 4) },
  });
  assert.equal(duration, 0n);
});
