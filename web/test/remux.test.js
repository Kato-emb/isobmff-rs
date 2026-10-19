import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { MovieDemuxer, initSync } from "../../crates/isobmff-wasm/pkg/isobmff_wasm.js";
import { demux } from "../demux.js";
import { dump } from "../dump.js";
import { rotationOf } from "../edits.js";
import { remuxToFragmented, remuxToNonFragmented } from "../remux.js";
import { box, fragment, fullBox, movie, u32 } from "./fixtures.js";

const wasm = initSync({ module: readFileSync(new URL("../../crates/isobmff-wasm/pkg/isobmff_wasm_bg.wasm", import.meta.url)) });

const NOW = 3_000_000_000n;

function nonFragmentedFile(sizes = [4, 6, 5, 3], syncSamples = [1, 3]) {
  const data = Uint8Array.from({ length: sizes.reduce((sum, size) => sum + size, 0) }, (_, index) => 100 + index);
  const chunkOffset = movie(0, { sizes, syncSamples }).length + 8;
  return new Blob([movie(chunkOffset, { sizes, syncSamples }), box("mdat", data)]);
}

async function remux(blob, toFragmented, changes = new Map()) {
  const parts = [];
  const write = (bytes) => parts.push(bytes.slice());
  await (toFragmented ? remuxToFragmented(blob, changes, write) : remuxToNonFragmented(blob, NOW, changes, write));
  return new Blob(parts);
}

async function samplesOf(blob) {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  const { samples } = await demux(blob);
  return samples.map(({ offset, size, ...properties }) => ({ ...properties, data: bytes.slice(Number(offset), Number(offset + size)) }));
}

test("a non-fragmented file written fragmented carries the same samples", async () => {
  const file = nonFragmentedFile();

  const written = await remux(file, true);

  assert.deepEqual(await samplesOf(written), await samplesOf(file));
});

test("a fragmented file written non-fragmented carries the same samples", async () => {
  const file = new Blob([movie(undefined), ...fragment(1, 0, [4, 6]), ...fragment(2, 6000, [5])]);

  const written = await remux(file, false);

  assert.deepEqual(await samplesOf(written), await samplesOf(file));
});

test("a fragment opens at every sync sample of a track whose sample table lists them", async () => {
  const written = await remux(nonFragmentedFile([4, 6, 5, 3], [1, 3]), true);

  const topLevel = (await dump(written)).filter(({ depth }) => depth === 0).map(({ box_type }) => box_type);
  assert.deepEqual(topLevel, ["ftyp", "moov", "moof", "mdat", "moof", "mdat"]);
});

async function boxesOf(blob, boxType) {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  return (await dump(blob))
    .filter(({ box_type }) => box_type === boxType)
    .map(({ offset, size }) => bytes.slice(Number(offset), Number(offset + size)));
}

test("a rotated track reads back as the rotation it was written with, written either way", async () => {
  for (const toFragmented of [true, false]) {
    const written = await remux(nonFragmentedFile(), toFragmented, new Map([[1, { rotation: 90 }]]));

    const { tracks } = await demux(written);
    assert.deepEqual(tracks[0].edits, { rotation: 90, starting_offset: 0, media_time: 0 });
  }
});

test("a matrix doing more than turning the picture has no rotation", () => {
  const mirrored = Int32Array.of(-0x1_0000, 0, 0, 0, 0x1_0000, 0, 0, 0, 0x4000_0000);

  assert.equal(rotationOf(mirrored), null);
});

test("a start delay and a media start are written as an empty edit and an edit lasting the rest of the media", async () => {
  const changes = new Map([[1, { starting_offset: 1 / 3, media_time: 3000 / 90_000 }]]);

  const written = await remux(nonFragmentedFile(), false, changes);

  assert.deepEqual(await boxesOf(written, "elst"), [fullBox("elst", 0, u32(2, 30_000, 0xffff_ffff, 0x1_0000, 15_000, 3000, 0x1_0000))]);
});

test("a start delay and a media start written fragmented leave the edit of the media at length 0", async () => {
  const changes = new Map([[1, { starting_offset: 1 / 3, media_time: 3000 / 90_000 }]]);

  const written = await remux(nonFragmentedFile(), true, changes);

  assert.deepEqual(await boxesOf(written, "elst"), [fullBox("elst", 0, u32(2, 30_000, 0xffff_ffff, 0x1_0000, 0, 3000, 0x1_0000))]);
});

test("a start delay and a media start both set to 0 drop the edit list", async () => {
  const delayed = await remux(nonFragmentedFile(), false, new Map([[1, { starting_offset: 1 / 3, media_time: 0 }]]));

  const written = await remux(delayed, false, new Map([[1, { starting_offset: 0 }]]));

  assert.deepEqual(await boxesOf(written, "edts"), []);
});

test("a media segment on its own is refused by the structure layer, written either way", async () => {
  const segment = new Blob([fragment(1, 0, [4])[0]]);

  for (const toFragmented of [true, false]) {
    await assert.rejects(remux(segment, toFragmented), {
      message: "file carries a moof box out of the order its structure keeps",
      kind: "structure",
    });
  }
});

test("remuxing a movie of many samples again leaves the wasm memory as it was", async () => {
  const sizes = new Array(200_000).fill(1);
  const file = nonFragmentedFile(sizes, [1, 100_000]);
  for (const toFragmented of [true, false]) {
    await remux(file, toFragmented);
  }
  const used = wasm.memory.buffer.byteLength;

  for (const toFragmented of [true, false]) {
    await remux(file, toFragmented);
  }

  assert.equal(wasm.memory.buffer.byteLength, used);
});

test("a handle following a track_ID no track declares any more is refused as a handle", () => {
  const movieOf = (trackId) => {
    const demuxer = new MovieDemuxer();
    demuxer.handleInput(0n, movie(undefined, { trackId }));
    const read = demuxer.movie();
    demuxer.free();
    return read;
  };
  const first = movieOf(1);
  const second = movieOf(2);
  const byId = first.trakById(1);

  first.trak()[0].setTkhd(second.trak()[0].tkhd());

  assert.throws(() => byId.tkhd().trackId(), { message: "the box this handle names is no longer there", kind: "handle" });
});
