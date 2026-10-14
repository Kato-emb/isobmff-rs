import {
  ChunkOffsets,
  FragmentedMuxFsm,
  MovieBox,
  MovieDemuxFsm,
  NonFragmentedMuxFsm,
  SampleSizeBox,
  SampleSizes,
  SampleTableBox,
  SampleToChunkBox,
  TimeToSampleBox,
} from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";
import { applyEdits } from "./edits.js";

const CUT_LENGTH = 1024 * 1024;

/**
 * Writes the movie file `blob` carries, fragmented or not, through `write` as a fragmented one
 *
 * Each track keeps its boxes but for the sample tables, which are emptied to
 * its sample description, so its samples lie in the fragments alone; the
 * movie header is made anew from the source's timescale, and the boxes of the
 * movie other than its tracks are dropped. The samples are taken in decode
 * time across the tracks, so each fragment carries the same stretch of time
 * of every track, and are written track by track within it. A fragment opens
 * at every sync sample of a track whose `stbl` carries an `stss`, so a source
 * with no such track, a fragmented one among them, is written as one
 * fragment. The source's `ftyp` is carried over, unless it lists a brand the
 * `default-base-is-moof` of the fragments shall not be used under, where the
 * one the muxer lays down takes its place. Each track carries the edits
 * `changes` states for its `track_ID`, as `applyEdits` makes them, which may
 * drop boxes of the track.
 *
 * Rejects with the library's Error, whose `kind` names the layer that
 * refused what it read or what it was to write, a file carrying no `moov` or
 * none ahead of its first `moof` among them; with an Error for a file that
 * ends with no `moov`, a movie declaring no track or two tracks of the same
 * `track_ID`, a sample of a track it does not declare, or more fragments than
 * a sequence number counts; or with the exception the browser or `write`
 * threw.
 *
 * @param {Blob} blob
 * @param {Map<number, Record<string, number>>} changes
 * @param {(bytes: Uint8Array) => void} write
 * @returns {Promise<void>}
 */
export async function remuxToFragmented(blob, changes, write) {
  const demux = new MovieDemuxFsm();
  const mux = new FragmentedMuxFsm();
  const queues = [];
  const fragment = [];
  const cutTracks = new Set();
  const held = [];
  const hold = (handle) => (held.push(handle), handle);
  try {
    let refused;
    try {
      const read = await readMovie(demux, blob);
      refused = read.refused;
      const source = hold(read.source);
      const timescale = hold(source.mvhd()).timescale();
      const movie = MovieBox.newFragmented(timescale, editedTracks(source.trak(), timescale, changes));
      if (!movie) {
        throw unwritableMovie();
      }
      hold(movie);
      for (const trak of source.trak().map(hold)) {
        const trackId = hold(trak.tkhd()).trackId();
        const mdia = hold(trak.mdia());
        const stbl = hold(hold(mdia.minf()).stbl());
        const stss = stbl.stss();
        if (stss) {
          hold(stss);
          cutTracks.add(trackId);
        }
        queues.push({ trackId, timescale: BigInt(hold(mdia.mdhd()).timescale()), samples: [], next: 0 });
        const target = movie.trakById(trackId);
        if (!target) {
          throw unwritableMovie();
        }
        const emptied = hold(
          SampleTableBox.new(
            hold(stbl.stsd()),
            hold(TimeToSampleBox.fromDeltas([])),
            hold(SampleToChunkBox.fromChunks([])),
            hold(SampleSizes.stsz(hold(SampleSizeBox.fromSizes([])))),
            hold(ChunkOffsets.fromOffsets([])),
          ),
        );
        hold(hold(hold(target).mdia()).minf()).setStbl(emptied);
      }
      const fileType = demux.fileType();
      if (fileType) {
        hold(fileType);
        if (!fileType.forbidsDefaultBaseIsMoof()) {
          mux.handleFileType(fileType);
        }
      }
      mux.handleMovie(movie);
    } finally {
      free(held.splice(0));
    }

    let sequenceNumber = 0;
    const writeFragment = () => {
      if (sequenceNumber === 0xffff_ffff) {
        throw new Error("more fragments than a sequence number counts");
      }
      sequenceNumber += 1;
      mux.beginFragment(sequenceNumber);
      fragment.sort((sample, other) => sample.properties.trackId - other.properties.trackId);
      for (const { sample } of fragment.splice(0)) {
        mux.handleSample(sample);
      }
      mux.finishFragment();
      writeOutput(mux, write);
    };

    let carriesCutTrack = false;
    for (;;) {
      for (;;) {
        for (let sample; (sample = demux.pollSample()); ) {
          const properties = sample.properties();
          const queue = queues.find((queue) => queue.trackId === properties.trackId);
          if (!queue) {
            sample.free();
            throw new Error("a sample of a track the movie does not declare");
          }
          queue.samples.push({ sample, properties });
        }
        if (refused) {
          throw refused;
        }
        if (queues.every((queue) => queue.next < queue.samples.length)) {
          break;
        }
        const wanted = demux.wantedInput();
        if (!wanted) {
          break;
        }
        refused = await handWanted(demux, wanted, blob);
      }
      let earliest;
      for (const queue of queues) {
        if (queue.next < queue.samples.length && (!earliest || earlier(queue, earliest))) {
          earliest = queue;
        }
      }
      if (!earliest) {
        break;
      }
      // Why not shift(): a track read in one chunk queues all its samples at
      // once, and moving the rest of the queue for each one taken is quadratic.
      const taken = earliest.samples[earliest.next];
      earliest.next += 1;
      if (earliest.next * 2 >= earliest.samples.length) {
        earliest.samples.splice(0, earliest.next);
        earliest.next = 0;
      }
      const isCutTrack = cutTracks.has(taken.properties.trackId);
      if (isCutTrack && carriesCutTrack && !taken.properties.sampleFlags.sampleIsNonSyncSample) {
        writeFragment();
        carriesCutTrack = false;
      }
      carriesCutTrack ||= isCutTrack;
      fragment.push(taken);
    }
    if (fragment.length > 0) {
      writeFragment();
    }
    mux.finish();
    writeOutput(mux, write);
  } finally {
    free([...queues.flatMap((queue) => queue.samples.slice(queue.next)), ...fragment].map(({ sample }) => sample));
    // Why not leave them to the FinalizationRegistry: the FSMs hold the movie
    // and the samples read ahead, the collector does not see wasm memory, and
    // wasm memory never shrinks.
    demux.free();
    mux.free();
  }
}

/**
 * Writes the movie file `blob` carries, fragmented or not, through `write` as a non-fragmented one
 *
 * The movie header and the tracks of the source are kept, the modification
 * time of the `mvhd` and of each `tkhd` and `mdhd` set to `now`, its `mvex`
 * and every other box of the movie dropped, and the sample tables of each
 * track are filled in from the samples and the durations updated from them.
 * A chunk opens wherever the samples pass to another track or another sample
 * description. The source's `ftyp` is carried over. Each track carries the
 * edits `changes` states for its `track_ID`, as `applyEdits` makes them,
 * which may drop boxes of the track.
 *
 * Rejects with the library's Error, whose `kind` names the layer that
 * refused what it read or what it was to write, a file carrying no `moov` or
 * none ahead of its first `moof` among them; with an Error for a file that
 * ends with no `moov`, or a movie declaring no track or two tracks of the
 * same `track_ID`; or with the exception the browser or `write` threw.
 *
 * @param {Blob} blob
 * @param {bigint} now the seconds since 1904, as `Mp4EpochSeconds.fromUnixSeconds` states them
 * @param {Map<number, Record<string, number>>} changes
 * @param {(bytes: Uint8Array) => void} write
 * @returns {Promise<void>}
 */
export async function remuxToNonFragmented(blob, now, changes, write) {
  const demux = new MovieDemuxFsm();
  const mux = new NonFragmentedMuxFsm();
  const held = [];
  const hold = (handle) => (held.push(handle), handle);
  let traks = [];
  try {
    let refused;
    try {
      const read = await readMovie(demux, blob);
      refused = read.refused;
      const source = hold(read.source);
      const mvhd = hold(source.mvhd());
      traks = source.trak();
      for (const trak of traks) {
        trak.setTkhd(hold(hold(trak.tkhd()).withModificationTime(now)));
        const mdia = hold(trak.mdia());
        mdia.setMdhd(hold(hold(mdia.mdhd()).withModificationTime(now)));
      }
      traks = editedTracks(traks, mvhd.timescale(), changes);
      const movie = MovieBox.new(hold(mvhd.withModificationTime(now)), traks.splice(0));
      if (!movie) {
        throw unwritableMovie();
      }
      hold(movie);
      const fileType = demux.fileType();
      if (fileType) {
        hold(fileType);
        mux.handleFileType(fileType);
      }
      mux.handleMovie(movie);
    } finally {
      free([...held.splice(0), ...traks.splice(0)]);
    }

    let chunk;
    for (;;) {
      for (let sample; (sample = demux.pollSample()); ) {
        const { trackId, sampleDescriptionIndex } = sample.properties();
        if (chunk?.trackId !== trackId || chunk.sampleDescriptionIndex !== sampleDescriptionIndex) {
          if (chunk !== undefined) {
            mux.finishChunk();
            writeOutput(mux, write);
          }
          chunk = { trackId, sampleDescriptionIndex };
          mux.beginChunk();
        }
        mux.handleSample(sample);
      }
      if (refused) {
        throw refused;
      }
      const wanted = demux.wantedInput();
      if (!wanted) {
        break;
      }
      refused = await handWanted(demux, wanted, blob);
    }
    if (chunk !== undefined) {
      mux.finishChunk();
    }
    mux.finish();
    writeOutput(mux, write);
  } finally {
    demux.free();
    mux.free();
  }
}

function editedTracks(traks, movieTimescale, changes) {
  return traks.map((trak) => {
    const tkhd = trak.tkhd();
    const edited = applyEdits(trak, changes.get(tkhd.trackId()) ?? {}, movieTimescale);
    tkhd.free();
    if (edited !== trak) {
      trak.free();
    }
    return edited;
  });
}

async function readMovie(demux, blob) {
  let refused;
  for (;;) {
    const source = demux.movie();
    if (source) {
      return { source, refused };
    }
    if (refused) {
      throw refused;
    }
    const wanted = demux.wantedInput();
    if (!wanted) {
      throw new Error(
        "the file carries no movie (moov) ahead of its samples; a media segment on its own, without the movie it continues, is not supported",
      );
    }
    refused = await handWanted(demux, wanted, blob);
  }
}

async function handWanted(demux, wanted, blob) {
  const start = Number(wanted.offset);
  const length = wanted.length === undefined ? CUT_LENGTH : Math.min(CUT_LENGTH, Number(wanted.length));
  const cut = new Uint8Array(await blob.slice(start, start + length).arrayBuffer());
  try {
    if (cut.length === 0) {
      demux.finish();
    } else {
      demux.handleInput(wanted.offset, cut);
    }
  } catch (error) {
    // Why not throw: the samples the demux FSM completed before the refusal
    // are taken first, and the caller throws it once they are.
    return error;
  }
}

function writeOutput(mux, write) {
  for (let bytes; (bytes = mux.pollOutput()); ) {
    write(bytes);
  }
}

function earlier(queue, other) {
  const time = queue.samples[queue.next].properties.decodeTime;
  const otherTime = other.samples[other.next].properties.decodeTime;
  return time * other.timescale < otherTime * queue.timescale;
}

function unwritableMovie() {
  return new Error("the movie declares no track, or two tracks of the same track_ID");
}

function free(classes) {
  for (const held of classes) {
    held.free();
  }
}
