import init, { Mp4EpochSeconds } from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";
import { demux } from "./demux.js";
import { dump } from "./dump.js";
import { remuxToFragmented, remuxToNonFragmented } from "./remux.js";

const ready = init();
const OUTPUT_DIRECTORY = "isobmff-rs-remux";
const WRITE_LENGTH = 1024 * 1024;

async function attempt(read) {
  try {
    return { value: await read() };
  } catch (error) {
    return { error: error.message ?? String(error) };
  }
}

async function remuxToStorage({ file, output, name, changes }) {
  const root = await navigator.storage.getDirectory();
  // The directory, not the root, is emptied: the origin is shared by every
  // site published under the same account.
  await root.removeEntry(OUTPUT_DIRECTORY, { recursive: true }).catch((error) => {
    if (error.name !== "NotFoundError") {
      throw error;
    }
  });
  const outputs = await root.getDirectoryHandle(OUTPUT_DIRECTORY, { create: true });
  const fileHandle = await outputs.getFileHandle(name, { create: true });
  const handle = await fileHandle.createSyncAccessHandle();
  try {
    // Why not hand each output to the handle: the non-fragmented muxer yields
    // the bytes of each sample apart, and a synchronous write per sample is
    // slow on large files.
    const buffer = new Uint8Array(WRITE_LENGTH);
    let buffered = 0;
    const flush = () => {
      handle.write(buffer.subarray(0, buffered));
      buffered = 0;
    };
    const write = (bytes) => {
      if (buffered + bytes.length > buffer.length) {
        flush();
      }
      if (bytes.length >= buffer.length) {
        handle.write(bytes);
      } else {
        buffer.set(bytes, buffered);
        buffered += bytes.length;
      }
    };
    if (output === "Fragmented") {
      await remuxToFragmented(file, changes, write);
    } else {
      const now = Mp4EpochSeconds.fromUnixSeconds(BigInt(Math.floor(Date.now() / 1000)));
      if (now === undefined) {
        throw new Error("the clock is past what a header states");
      }
      await remuxToNonFragmented(file, now, changes, write);
    }
    flush();
    handle.flush();
  } finally {
    handle.close();
  }
  return fileHandle.getFile();
}

self.onmessage = async ({ data }) => {
  if (data.request === "remux") {
    try {
      await ready;
      self.postMessage({ file: await remuxToStorage(data) });
    } catch (error) {
      self.postMessage({ error: error.message ?? String(error) });
    }
    return;
  }
  try {
    await ready;
  } catch (error) {
    const failed = { error: error.message ?? String(error) };
    self.postMessage({ boxes: failed, movie: failed });
    return;
  }
  const boxes = await attempt(() => dump(data.file));
  const movie = await attempt(() => demux(data.file));
  self.postMessage({ boxes, movie });
};
