export function box(type, ...payload) {
  const bytes = new Uint8Array(payload.reduce((length, part) => length + part.length, 8));
  new DataView(bytes.buffer).setUint32(0, bytes.length);
  bytes.set(new TextEncoder().encode(type), 4);
  payload.reduce((at, part) => (bytes.set(part, at), at + part.length), 8);
  return bytes;
}

export function fullBox(type, flags, ...payload) {
  return box(type, u32(flags), ...payload);
}

export function u32(...values) {
  const flat = values.flat();
  const bytes = new Uint8Array(flat.length * 4);
  const view = new DataView(bytes.buffer);
  flat.forEach((value, index) => view.setUint32(index * 4, value));
  return bytes;
}

/**
 * Returns the `moov` of one video track `trackId` whose samples of `sizes`, 3000 ticks each, lie in one chunk at `chunkOffset`
 *
 * Every sample but the first has a composition offset of 6000, and the sync
 * samples are those numbered in `syncSamples`. Without `chunkOffset` the tables
 * are empty and an `mvex` continues the movie in fragments.
 */
export function movie(chunkOffset, { sizes = [4, 6], syncSamples = [1], trackId = 1 } = {}) {
  const tables =
    chunkOffset === undefined
      ? [fullBox("stts", 0, u32(0)), fullBox("stsc", 0, u32(0)), fullBox("stsz", 0, u32(0, 0)), fullBox("stco", 0, u32(0))]
      : [
          fullBox("stts", 0, u32(1, sizes.length, 3000)),
          fullBox("ctts", 0, u32(2, 1, 0, sizes.length - 1, 6000)),
          fullBox("stss", 0, u32(syncSamples.length, syncSamples)),
          fullBox("stsc", 0, u32(1, 1, sizes.length, 1)),
          fullBox("stsz", 0, u32(0, sizes.length, sizes)),
          fullBox("stco", 0, u32(1, chunkOffset)),
        ];
  return box(
    "moov",
    fullBox("mvhd", 0, u32(0, 0, 90_000, 0), new Uint8Array(76), u32(trackId + 1)),
    box(
      "trak",
      fullBox("tkhd", 1, u32(0, 0, trackId, 0, 0), new Uint8Array(60)),
      box(
        "mdia",
        fullBox("mdhd", 0, u32(0, 0, 90_000, 0, 0x55c4_0000)),
        fullBox("hdlr", 0, u32(0), new TextEncoder().encode("vide"), new Uint8Array(13)),
        box(
          "minf",
          fullBox("vmhd", 1, new Uint8Array(8)),
          box("dinf", fullBox("dref", 0, u32(1), fullBox("url ", 1))),
          box("stbl", fullBox("stsd", 0, u32(1), box("avc1", new Uint8Array([0, 0, 0, 0, 0, 0, 0, 1]))), ...tables),
        ),
      ),
    ),
    ...(chunkOffset === undefined ? [box("mvex", fullBox("trex", 0, u32(trackId, 1, 0, 0, 0)))] : []),
  );
}

/**
 * Returns a `moof` and its `mdat` of samples of track 1, of 3000 ticks each from `baseDecodeTime`, all sync samples
 */
export function fragment(sequenceNumber, baseDecodeTime, sizes) {
  const traf = (dataOffset) =>
    box(
      "traf",
      fullBox("tfhd", 0x02_0000, u32(1)),
      fullBox("tfdt", 0x0100_0000, u32(0, baseDecodeTime)),
      fullBox("trun", 0x00_0301, u32(sizes.length, dataOffset, sizes.flatMap((size) => [3000, size]))),
    );
  const length = box("moof", fullBox("mfhd", 0, u32(sequenceNumber)), traf(0)).length;
  const moof = box("moof", fullBox("mfhd", 0, u32(sequenceNumber)), traf(length + 8));
  const data = Uint8Array.from({ length: sizes.reduce((sum, size) => sum + size, 0) }, (_, index) => index);
  return [moof, box("mdat", data)];
}
