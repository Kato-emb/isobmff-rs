const PAGE_LENGTH = 500;
const HEX_LENGTH = 4096;

const input = document.querySelector("#file");
const operation = document.querySelector("#operation");
const status = document.querySelector("#status");
const dumpSection = document.querySelector("#dump");
const demuxSection = document.querySelector("#demux");
const boxRows = document.querySelector("#boxes tbody");
const trackRows = document.querySelector("#tracks tbody");
const sampleRows = document.querySelector("#samples tbody");
const trackChoice = document.querySelector("#track");
const previous = document.querySelector("#previous");
const next = document.querySelector("#next");
const page = document.querySelector("#page");
const hex = document.querySelector("#hex");

let worker;
let file;
let samples = [];
let chosenSamples = [];
let pageStart = 0;

function run() {
  [file] = input.files;
  if (!file) {
    return;
  }
  const request = operation.querySelector("input:checked").value;
  dumpSection.hidden = true;
  demuxSection.hidden = true;
  status.textContent = `Reading ${file.name}…`;

  // Terminated so that a reply to an earlier choice never lands over this one.
  worker?.terminate();
  worker = new Worker("worker.js", { type: "module" });
  worker.onmessage = ({ data }) => {
    worker.terminate();
    if (data.reply === "error") {
      status.textContent = data.message;
    } else if (data.reply === "dump") {
      showBoxes(data.boxes);
    } else {
      showTracks(data.tracks, data.samples);
    }
  };
  worker.onerror = (event) => {
    worker.terminate();
    status.textContent = event.message;
  };
  worker.postMessage({ request, file });
}

function row(cells) {
  const rowElement = document.createElement("tr");
  for (const text of cells) {
    const cell = document.createElement("td");
    cell.textContent = text;
    rowElement.append(cell);
  }
  return rowElement;
}

function rows(tbody, records, toRow) {
  const fragment = document.createDocumentFragment();
  for (const record of records) {
    fragment.append(toRow(record));
  }
  tbody.replaceChildren(fragment);
}

function showBoxes(boxes) {
  status.textContent = `${boxes.length} boxes in ${file.name}`;
  rows(boxRows, boxes, ({ box_type, offset, size, depth }) => {
    const rowElement = row([box_type, offset, size ?? "to end of file"]);
    rowElement.style.setProperty("--depth", depth);
    return rowElement;
  });
  dumpSection.hidden = false;
}

function showTracks(tracks, resolved) {
  const counts = new Map();
  for (const sample of resolved) {
    counts.set(sample.track_id, (counts.get(sample.track_id) ?? 0) + 1);
  }
  samples = resolved;

  status.textContent = `${tracks.length} tracks and ${samples.length} samples in ${file.name}`;
  rows(trackRows, tracks, (track) =>
    row([
      track.track_id,
      track.handler_type,
      track.timescale,
      track.duration ?? "indeterminate",
      track.sample_entries.join(", "),
      counts.get(track.track_id) ?? 0,
    ]),
  );
  trackChoice.replaceChildren(
    new Option("all", ""),
    ...tracks.map((track) => new Option(`${track.track_id} (${track.handler_type})`, track.track_id)),
  );
  hex.textContent = "";
  chooseTrack();
  demuxSection.hidden = false;
}

function chooseTrack() {
  const trackId = Number(trackChoice.value);
  chosenSamples = trackChoice.value === "" ? samples : samples.filter((sample) => sample.track_id === trackId);
  showPage(0);
}

function showPage(start) {
  pageStart = start;
  const end = Math.min(start + PAGE_LENGTH, chosenSamples.length);
  page.textContent = `${chosenSamples.length === 0 ? 0 : start + 1}–${end} of ${chosenSamples.length}`;
  previous.disabled = start === 0;
  next.disabled = end === chosenSamples.length;
  rows(sampleRows, chosenSamples.slice(start, end), (sample) =>
    row([
      sample.track_id,
      sample.decode_time,
      sample.sample_duration,
      sample.sample_composition_time_offset,
      sample.sync ? "yes" : "no",
      sample.sample_description_index,
      sample.offset,
      sample.size,
    ]),
  );
}

function chooseSample(event) {
  const rowElement = event.target.closest("tr");
  if (!rowElement) {
    return;
  }
  sampleRows.querySelector(".chosen")?.classList.remove("chosen");
  rowElement.classList.add("chosen");
  showHex(chosenSamples[pageStart + rowElement.sectionRowIndex]);
}

async function showHex({ offset, size }) {
  const start = Number(offset);
  const length = Math.min(Number(size), HEX_LENGTH);
  const bytes = new Uint8Array(await file.slice(start, start + length).arrayBuffer());
  const lines = [];
  for (let at = 0; at < bytes.length; at += 16) {
    const line = bytes.subarray(at, at + 16);
    const hexText = Array.from(line, (byte) => byte.toString(16).padStart(2, "0")).join(" ");
    const text = Array.from(line, (byte) => (byte >= 0x20 && byte < 0x7f ? String.fromCharCode(byte) : ".")).join("");
    lines.push(`${(start + at).toString(16).padStart(8, "0")}  ${hexText.padEnd(47)}  ${text}`);
  }
  if (length < Number(size)) {
    lines.push(`… the first ${length} of ${size} bytes`);
  }
  hex.textContent = lines.join("\n");
}

input.addEventListener("change", run);
operation.addEventListener("change", run);
trackChoice.addEventListener("change", chooseTrack);
sampleRows.addEventListener("click", chooseSample);
previous.addEventListener("click", () => showPage(pageStart - PAGE_LENGTH));
next.addEventListener("click", () => showPage(pageStart + PAGE_LENGTH));
