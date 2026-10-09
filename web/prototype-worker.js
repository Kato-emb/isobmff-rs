// PROTOTYPE — throwaway harness: times the JS-driven remux against the Rust one.
import init, { prototype_rust_remux_blob } from "../crates/isobmff-wasm/pkg/isobmff_wasm.js";
import { remuxToNonFragmented } from "./prototype-js-remux.js";

const NOW = 3_000_000_000n;
const ready = init();

async function digest(blob) {
  const hash = await crypto.subtle.digest("SHA-256", await blob.arrayBuffer());
  return Array.from(new Uint8Array(hash).slice(0, 8), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function timed(label, run) {
  const start = performance.now();
  const result = await run();
  return { label, ms: Math.round(performance.now() - start), ...result };
}

self.onmessage = async ({ data: { file } }) => {
  try {
    await ready;
    const rows = [];
    for (let round = 1; round <= 2; round += 1) {
      rows.push(await timed(`rust (FileReaderSync) #${round}`, async () => {
        const bytes = new Blob([prototype_rust_remux_blob(file, NOW)]);
        return { size: bytes.size, sha: await digest(bytes) };
      }));
      rows.push(await timed(`js loop, data untouched #${round}`, async () => {
        const { bytes, samples } = await remuxToNonFragmented(file, NOW);
        return { size: bytes.size, samples, sha: await digest(bytes) };
      }));
      rows.push(await timed(`js loop, data copied to JS #${round}`, async () => {
        const { bytes, samples } = await remuxToNonFragmented(file, NOW, { touchData: true });
        return { size: bytes.size, samples, sha: await digest(bytes) };
      }));
    }
    self.postMessage({ rows });
  } catch (error) {
    self.postMessage({ error: String(error?.message ?? error) });
  }
};
