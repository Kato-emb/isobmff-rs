import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { MovieDemuxFsm, initSync } from "../../crates/isobmff-wasm/pkg/isobmff_wasm.js";
import { demux } from "../demux.js";
import { dump } from "../dump.js";
import { remuxToFragmented, remuxToNonFragmented } from "../remux.js";
import { box, fragment, movie } from "./fixtures.js";

const wasm = initSync({ module: readFileSync(new URL("../../crates/isobmff-wasm/pkg/isobmff_wasm_bg.wasm", import.meta.url)) });

const NOW = 3_000_000_000n;

function nonFragmentedFile(sizes = [4, 6, 5, 3], syncSamples = [1, 3]) {
  const data = Uint8Array.from({ length: sizes.reduce((sum, size) => sum + size, 0) }, (_, index) => 100 + index);
  const chunkOffset = movie(0, { sizes, syncSamples }).length + 8;
  return new Blob([movie(chunkOffset, { sizes, syncSamples }), box("mdat", data)]);
}

async function remux(blob, toFragmented) {
  const parts = [];
  const write = (bytes) => parts.push(bytes.slice());
  await (toFragmented ? remuxToFragmented(blob, write) : remuxToNonFragmented(blob, NOW, write));
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
    const fsm = new MovieDemuxFsm();
    fsm.handleInput(0n, movie(undefined, { trackId }));
    const read = fsm.movie();
    fsm.free();
    return read;
  };
  const first = movieOf(1);
  const second = movieOf(2);
  const byId = first.trakById(1);

  first.trak()[0].setTkhd(second.trak()[0].tkhd());

  assert.throws(() => byId.tkhd().trackId(), { message: "the box this handle names is no longer there", kind: "handle" });
});
