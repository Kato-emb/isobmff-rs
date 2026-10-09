const PAGE_LENGTH = 500;
const HEX_LENGTH = 4096;
const BOX_HEX_LENGTH = 512;
const NARROW_HEX_WIDTH = 600;
const OUTPUTS = {
  Fragmented: { label: "Fragmented MP4", suffix: "frag" },
  NonFragmented: { label: "Non-fragmented MP4", suffix: "nonfrag" },
};
const HANDLERS = { vide: "Video", soun: "Audio", hint: "Hint", meta: "Metadata", text: "Text", subt: "Subtitles", sbtl: "Subtitles" };
const BOX_NAMES = {
  ftyp: "File type", styp: "Segment type", moov: "Movie", mvhd: "Movie header", trak: "Track",
  tkhd: "Track header", edts: "Edit", elst: "Edit list", mdia: "Media", mdhd: "Media header",
  hdlr: "Handler", minf: "Media information", vmhd: "Video media header", smhd: "Sound media header",
  dinf: "Data information", dref: "Data reference", stbl: "Sample table", stsd: "Sample description",
  stts: "Decoding time to sample", ctts: "Composition offset", stss: "Sync sample", stsc: "Sample to chunk",
  stsz: "Sample size", stz2: "Compact sample size", stco: "Chunk offset", co64: "Chunk large offset",
  sdtp: "Sample dependency type", mvex: "Movie extends", mehd: "Movie extends header", trex: "Track extends",
  moof: "Movie fragment", mfhd: "Movie fragment header", traf: "Track fragment", tfhd: "Track fragment header",
  tfdt: "Track fragment decode time", trun: "Track run", mdat: "Media data", free: "Free space",
  skip: "Free space", udta: "User data", meta: "Metadata", sidx: "Segment index",
  mfra: "Movie fragment random access", tfra: "Track fragment random access",
  mfro: "Movie fragment random access offset", sgpd: "Sample group description", sbgp: "Sample to group",
};

const chip = document.querySelector("#chip");
const tabs = document.querySelector("#tabs");
const input = document.querySelector("#file");
const main = document.querySelector("#main");
const list = document.querySelector("#list");
const inspector = document.querySelector("#inspector");
const numbers = new Intl.NumberFormat("en-US");
const secondNumbers = new Intl.NumberFormat("en-US", { minimumFractionDigits: 3, maximumFractionDigits: 3 });

const VIEWS = {
  overview: { render: renderOverview },
  boxes: { noun: "box", count: (data) => data.boxes.length, error: (data) => data.boxesError, render: renderBoxes, inspect: inspectBox },
  tracks: { noun: "track", count: (data) => data.tracks.length, error: (data) => data.samplesError, render: renderTracks, inspect: inspectTrack },
  samples: { noun: "sample", count: (data) => data.samples.length, error: (data) => data.samplesError, render: renderSamples, inspect: inspectSample },
  remux: { render: renderRemux },
};

let worker;
let remuxWorker;
const state = {
  tab: "overview",
  data: null,
  chosen: null,
  collapsed: new Set(),
  track: null,
  page: 0,
  remux: null,
};

function element(tag, attributes = {}, ...children) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attributes)) {
    if (name === "class") {
      node.className = value ?? "";
    } else if (name === "style") {
      node.style.cssText = value;
    } else if (value === true) {
      node.setAttribute(name, "");
    } else if (value !== false && value != null) {
      node.setAttribute(name, value);
    }
  }
  for (const child of children.flat()) {
    if (child != null && child !== false) {
      node.append(child instanceof Node ? child : String(child));
    }
  }
  return node;
}

const grouped = (value) => numbers.format(value);
const located = (value) => `${grouped(value)} (0x${value.toString(16)})`;
const seconds = (value) => `${secondNumbers.format(value)} s`;
const kind = (handlerType) => HANDLERS[handlerType] ?? handlerType;

function bytes(value) {
  const number = Number(value);
  if (number < 1024) {
    return `${number} B`;
  }
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let scaled = number / 1024;
  let unit = 0;
  while (scaled >= 1024 && unit < units.length - 1) {
    scaled /= 1024;
    unit += 1;
  }
  return `${scaled.toFixed(1)} ${units[unit]}`;
}

function open(file) {
  if (!file) {
    return;
  }
  // Terminated so that a reply about an earlier file never lands over this one.
  worker?.terminate();
  stopRemux();
  worker = new Worker("worker.js", { type: "module" });
  chip.textContent = `${file.name} · reading…`;
  worker.onmessage = ({ data }) => {
    worker.terminate();
    loaded(file, data);
  };
  worker.onerror = (event) => {
    worker.terminate();
    loaded(file, { boxes: { error: event.message }, movie: { error: event.message } });
  };
  worker.postMessage({ file });
}

function stopRemux() {
  remuxWorker?.terminate();
  if (state.remux?.url) {
    URL.revokeObjectURL(state.remux.url);
  }
  state.remux = null;
}

function remuxTo(output) {
  const { data } = state;
  const name = `${data.file.name.replace(/\.[^.]*$/, "")}.${OUTPUTS[output].suffix}.mp4`;
  stopRemux();
  state.remux = { output, writing: true };
  render();
  const finished = (remux) => {
    remuxWorker.terminate();
    if (state.data === data) {
      state.remux = { output, ...remux };
      render();
    }
  };
  remuxWorker = new Worker("worker.js", { type: "module" });
  remuxWorker.onmessage = ({ data: reply }) =>
    finished(reply.error ? { error: reply.error } : { name, size: reply.file.size, url: URL.createObjectURL(reply.file) });
  remuxWorker.onerror = (event) => finished({ error: event.message });
  remuxWorker.postMessage({ request: "remux", file: data.file, output, name });
}

function loaded(file, { boxes, movie }) {
  const tracks = movie.value?.tracks ?? [];
  const samples = movie.value?.samples ?? [];
  const samplesByTrack = new Map(tracks.map((track) => [track.track_id, []]));
  for (const sample of samples) {
    samplesByTrack.get(sample.track_id)?.push(sample);
  }
  const trackStatistics = new Map(
    tracks.map((track) => {
      const trackSamples = samplesByTrack.get(track.track_id);
      let total = 0;
      let ticks = 0;
      let sync = 0;
      for (const sample of trackSamples) {
        total += Number(sample.size);
        ticks += sample.sample_duration;
        sync += sample.sync ? 1 : 0;
      }
      const durationSeconds = ticks / track.timescale;
      return [
        track.track_id,
        { samples: trackSamples.length, sync, total, durationSeconds, bitrate: durationSeconds ? (total * 8) / durationSeconds / 1000 : 0 },
      ];
    }),
  );
  state.data = {
    file,
    boxes: (boxes.value ?? []).map((box) => ({ ...box, size: box.size ?? BigInt(file.size) - box.offset })),
    boxesError: boxes.error ?? null,
    tracks,
    samples,
    samplesByTrack,
    trackStatistics,
    samplesError: movie.error ?? null,
  };
  Object.assign(state, { tab: "overview", chosen: null, collapsed: new Set(), track: null, page: 0 });
  chip.textContent = `${file.name} · ${bytes(file.size)}`;
  render();
}

function render({ keepScroll = false } = {}) {
  const { data, tab } = state;
  const view = VIEWS[tab];
  const scrolled = list.querySelector(".scroll")?.scrollTop ?? 0;
  tabs.replaceChildren(
    ...Object.entries(VIEWS).map(([name, { count }]) =>
      element(
        "button",
        { type: "button", "data-tab": name, "aria-pressed": String(tab === name) },
        name,
        data && count ? element("span", { class: "count" }, grouped(count(data))) : null,
      ),
    ),
  );
  main.classList.toggle("wide", !VIEWS[tab].inspect || !data);
  if (!data) {
    list.replaceChildren(
      element(
        "div",
        { class: "scroll empty" },
        element(
          "div",
          { class: "drop" },
          element("p", {}, element("strong", {}, "Drop an MP4 here"), " or use Open file…"),
          element("p", {}, "The boxes, the tracks and the samples of an ISO base media file, read by the isobmff crate in this browser."),
        ),
      ),
    );
    return;
  }
  const error = view.error?.(data);
  if (error) {
    list.replaceChildren(element("div", { class: "scroll" }, element("p", { class: "error" }, error)));
  } else {
    view.render();
  }
  if (keepScroll) {
    list.querySelector(".scroll").scrollTop = scrolled;
  }
  renderInspector();
}

function trackPairs(track) {
  const statistics = state.data.trackStatistics.get(track.track_id);
  return [
    ["Duration", seconds(statistics.durationSeconds)],
    ["Samples", `${grouped(statistics.samples)} (${grouped(statistics.sync)} sync)`],
    ["Size", bytes(statistics.total)],
    ["Bitrate", statistics.durationSeconds ? `${grouped(Math.round(statistics.bitrate))} kbit/s` : "—"],
    ["Timescale", `${grouped(track.timescale)} /s`],
  ];
}

function pairs(entries) {
  return element("dl", {}, entries.flatMap(([term, description]) => [element("dt", {}, term), element("dd", {}, description)]));
}

function card(label, value) {
  return element("div", { class: "card" }, element("div", { class: "label" }, label), element("div", { class: "value" }, value));
}

function renderOverview() {
  const { file, boxes, tracks, samples, trackStatistics, boxesError, samplesError } = state.data;
  const longest = Math.max(0, ...[...trackStatistics.values()].map((statistics) => statistics.durationSeconds));
  const sections = [
    element(
      "div",
      { class: "cards" },
      card("Size", bytes(file.size)),
      card("Boxes", grouped(boxes.length)),
      card("Tracks", grouped(tracks.length)),
      card("Samples", grouped(samples.length)),
      card("Duration", longest ? seconds(longest) : "—"),
    ),
    element(
      "div",
      {},
      element("h2", {}, "File layout"),
      boxesError
        ? element("p", { class: "error" }, boxesError)
        : [
            element(
              "div",
              { class: "map" },
              boxes
                .filter((box) => box.depth === 0)
                .map((box) =>
                  element("div", {
                    title: `${box.box_type} · ${bytes(box.size)} at ${grouped(box.offset)}`,
                    style: `flex: ${Number(box.size)} 0 0; background: var(--${["moov", "moof", "mdat"].includes(box.box_type) ? box.box_type : "other"})`,
                  }),
                ),
            ),
            element(
              "div",
              { class: "legend" },
              ["moov", "moof", "mdat", "other"].map((type) =>
                element("span", { style: `--swatch: var(--${type})` }, type === "other" ? "other boxes" : type),
              ),
            ),
          ],
    ),
    element(
      "div",
      {},
      element("h2", {}, "Tracks"),
      samplesError
        ? element("p", { class: "error" }, samplesError)
        : element(
            "div",
            { class: "tracks" },
            tracks.map((track) =>
              element(
                "div",
                { class: "card" },
                element(
                  "h3",
                  {},
                  kind(track.handler_type),
                  element("span", { class: "badge" }, track.sample_entries.join(", ")),
                  element("span", { class: "badge" }, `track ${track.track_id}`),
                ),
                pairs(trackPairs(track)),
              ),
            ),
          ),
    ),
  ];
  list.replaceChildren(element("div", { class: "scroll" }, element("div", { class: "overview" }, sections)));
}

function renderRemux() {
  const { remux } = state;
  list.replaceChildren(
    element(
      "div",
      { class: "scroll" },
      element(
        "div",
        { class: "overview" },
        element(
          "div",
          { class: "remux" },
          element("h2", {}, "Remux"),
          element(
            "p",
            { class: "muted" },
            "Write the samples of this file out again as a fragmented or a non-fragmented MP4. The output is written in this browser and kept in its storage until the next remux.",
          ),
          element(
            "div",
            { class: "choices" },
            Object.entries(OUTPUTS).map(([output, { label }]) =>
              element(
                "button",
                { type: "button", "data-remux": output, disabled: remux?.writing ?? false, "aria-pressed": String(remux?.output === output) },
                label,
              ),
            ),
          ),
          remux?.writing ? element("p", { class: "muted" }, "Writing…") : null,
          remux?.error ? element("p", { class: "error" }, remux.error) : null,
          remux?.url
            ? element(
                "p",
                {},
                element("a", { class: "open-file", href: remux.url, download: remux.name }, `Download ${remux.name}`),
                " ",
                element("span", { class: "muted" }, bytes(remux.size)),
              )
            : null,
        ),
      ),
    ),
  );
}

function table(headings, entries) {
  return element(
    "table",
    {},
    element(
      "thead",
      {},
      element(
        "tr",
        {},
        headings.map(([heading, optional]) => element("th", { class: optional ? "optional" : null }, heading)),
      ),
    ),
    element(
      "tbody",
      {},
      entries.map(([index, cells]) =>
        element(
          "tr",
          { "data-index": index, tabindex: 0, class: state.chosen === index ? "chosen" : null },
          cells.map((cell, column) =>
            cell instanceof HTMLTableCellElement ? cell : element("td", { class: headings[column][1] ? "optional" : null }, cell),
          ),
        ),
      ),
    ),
  );
}

function renderBoxes() {
  const { boxes, file } = state.data;
  const entries = [];
  let hiddenDeeperThan = Infinity;
  boxes.forEach((box, index) => {
    if (box.depth > hiddenDeeperThan) {
      return;
    }
    const parent = boxes[index + 1]?.depth > box.depth;
    const collapsed = state.collapsed.has(index);
    hiddenDeeperThan = collapsed ? box.depth : Infinity;
    entries.push([
      index,
      [
        element(
          "td",
          { style: `padding-left: ${12 + box.depth * 18}px` },
          element("span", { class: "twisty", "data-twisty": parent ? index : null }, parent ? (collapsed ? "▸" : "▾") : ""),
          box.box_type,
          element("span", { class: "box-name optional" }, BOX_NAMES[box.box_type] ?? ""),
        ),
        grouped(box.offset),
        grouped(box.size),
        element("span", { class: "share", style: `width: ${(Number(box.size) / file.size) * 100}%` }),
      ],
    ]);
  });
  list.replaceChildren(element("div", { class: "scroll" }, table([["Box"], ["Offset"], ["Size"], ["Share of file", true]], entries)));
}

function renderTracks() {
  const { tracks, trackStatistics } = state.data;
  list.replaceChildren(
    element(
      "div",
      { class: "scroll" },
      table(
        [["Track"], ["Kind"], ["Codec"], ["Duration", true], ["Samples"], ["Size", true]],
        tracks.map((track, index) => {
          const statistics = trackStatistics.get(track.track_id);
          return [
            index,
            [
              track.track_id,
              kind(track.handler_type),
              track.sample_entries.join(", "),
              seconds(statistics.durationSeconds),
              grouped(statistics.samples),
              bytes(statistics.total),
            ],
          ];
        }),
      ),
    ),
  );
}

function chosenSamples() {
  const { samples, samplesByTrack } = state.data;
  return state.track == null ? samples : samplesByTrack.get(state.track);
}

function renderSamples() {
  const { tracks } = state.data;
  const shown = chosenSamples();
  const start = state.page * PAGE_LENGTH;
  const end = Math.min(start + PAGE_LENGTH, shown.length);
  const timescales = new Map(tracks.map((track) => [track.track_id, track.timescale]));
  list.replaceChildren(
    element(
      "div",
      { class: "toolbar" },
      element("button", { type: "button", "data-track": "", "aria-pressed": String(state.track == null) }, "All"),
      tracks.map((track) =>
        element(
          "button",
          { type: "button", "data-track": track.track_id, "aria-pressed": String(state.track === track.track_id) },
          `${track.track_id} · ${kind(track.handler_type)}`,
        ),
      ),
      element(
        "span",
        { class: "pages" },
        element("button", { type: "button", "data-page": -1, disabled: start === 0, "aria-label": "Previous page" }, "‹"),
        `${grouped(shown.length ? start + 1 : 0)}–${grouped(end)} of ${grouped(shown.length)}`,
        element("button", { type: "button", "data-page": 1, disabled: end === shown.length, "aria-label": "Next page" }, "›"),
      ),
    ),
    element(
      "div",
      { class: "scroll" },
      table(
        [["Track"], ["Decode time"], ["Duration (ticks)", true], ["Composition offset (ticks)", true], ["Sync"], ["Offset", true], ["Size"]],
        shown.slice(start, end).map((sample, at) => [
          start + at,
          [
            sample.track_id,
            seconds(Number(sample.decode_time) / timescales.get(sample.track_id)),
            grouped(sample.sample_duration),
            grouped(sample.sample_composition_time_offset),
            element("td", { class: sample.sync ? "sync" : "muted" }, sample.sync ? "●" : "○"),
            grouped(sample.offset),
            grouped(sample.size),
          ],
        ]),
      ),
    ),
  );
}

async function hex(offset, size, limit) {
  const start = Number(offset);
  const length = Math.min(Number(size), limit);
  const data = new Uint8Array(await state.data.file.slice(start, start + length).arrayBuffer());
  const perLine = inspector.clientWidth < NARROW_HEX_WIDTH ? 8 : 16;
  const lines = [];
  for (let at = 0; at < data.length; at += perLine) {
    const line = data.subarray(at, at + perLine);
    lines.push(
      element(
        "div",
        {},
        element("span", { class: "dim" }, (start + at).toString(16).padStart(8, "0")),
        "  ",
        Array.from(line, (byte) => byte.toString(16).padStart(2, "0")).join(" ").padEnd(perLine * 3 - 1),
        "  ",
        element(
          "span",
          { class: "dim" },
          Array.from(line, (byte) => (byte >= 0x20 && byte < 0x7f ? String.fromCharCode(byte) : ".")).join(""),
        ),
      ),
    );
  }
  return element(
    "div",
    {},
    element("div", { class: "hex" }, lines),
    length < Number(size) ? element("p", { class: "muted" }, `The first ${grouped(length)} of ${grouped(size)} bytes`) : null,
  );
}

function choose(index) {
  list.querySelector("tr.chosen")?.classList.remove("chosen");
  list.querySelector(`tr[data-index="${index}"]`)?.classList.add("chosen");
  state.chosen = index;
  renderInspector();
}

async function inspectBox(data, chosen) {
  const box = data.boxes[chosen];
  const path = [box.box_type];
  for (let index = chosen - 1, depth = box.depth; index >= 0 && depth > 0; index -= 1) {
    if (data.boxes[index].depth === depth - 1) {
      path.unshift(data.boxes[index].box_type);
      depth -= 1;
    }
  }
  return [
    element("h2", {}, box.box_type),
    element("div", { class: "subtitle" }, BOX_NAMES[box.box_type] ?? "Box"),
    pairs([
      ["Path", path.join(" › ")],
      ["Offset", located(box.offset)],
      ["Size", `${grouped(box.size)} (${bytes(box.size)})`],
      ["Share of file", `${((Number(box.size) / data.file.size) * 100).toFixed(2)} %`],
    ]),
    await hex(box.offset, box.size, BOX_HEX_LENGTH),
  ];
}

function inspectTrack(data, chosen) {
  const track = data.tracks[chosen];
  return [
    element("h2", {}, `Track ${track.track_id}`),
    element("div", { class: "subtitle" }, `${kind(track.handler_type)} · ${track.sample_entries.join(", ")}`),
    pairs([
      ["Handler", track.handler_type],
      ["Header duration", track.duration == null ? "indeterminate" : `${grouped(track.duration)} ticks (${seconds(Number(track.duration) / track.timescale)})`],
      ...trackPairs(track),
    ]),
  ];
}

async function inspectSample(data, chosen) {
  const sample = chosenSamples()[chosen];
  const track = data.tracks.find((candidate) => candidate.track_id === sample.track_id);
  return [
    element("h2", {}, "Sample"),
    element("div", { class: "subtitle" }, `Track ${sample.track_id} · ${kind(track.handler_type)}`),
    pairs([
      ["Decode time", `${grouped(sample.decode_time)} ticks (${seconds(Number(sample.decode_time) / track.timescale)})`],
      ["Duration", `${grouped(sample.sample_duration)} ticks`],
      ["Composition offset", `${grouped(sample.sample_composition_time_offset)} ticks`],
      ["Sync", sample.sync ? "yes" : "no"],
      ["Description", `${sample.sample_description_index} (${track.sample_entries[sample.sample_description_index - 1] ?? "?"})`],
      ["Offset", located(sample.offset)],
      ["Size", grouped(sample.size)],
    ]),
    await hex(sample.offset, sample.size, HEX_LENGTH),
  ];
}

async function renderInspector() {
  const { chosen, tab, data } = state;
  const view = VIEWS[tab];
  inspector.classList.toggle("open", chosen != null);
  if (chosen == null || !data || !view.inspect) {
    inspector.replaceChildren(view.noun ? element("p", { class: "muted" }, `Choose a ${view.noun} to see it here.`) : "");
    return;
  }
  const content = await view.inspect(data, chosen);
  if (state.chosen === chosen && state.tab === tab && state.data === data) {
    inspector.replaceChildren(element("button", { type: "button", class: "close", "data-close": true }, "Close"), ...content);
  }
}

tabs.addEventListener("click", (event) => {
  const button = event.target.closest("[data-tab]");
  if (button && state.data) {
    Object.assign(state, { tab: button.dataset.tab, chosen: null });
    render();
  }
});

list.addEventListener("click", (event) => {
  const twisty = event.target.closest("[data-twisty]");
  const track = event.target.closest("[data-track]");
  const page = event.target.closest("[data-page]");
  const row = event.target.closest("tr[data-index]");
  const remux = event.target.closest("[data-remux]");
  if (remux) {
    remuxTo(remux.dataset.remux);
  } else if (twisty) {
    const index = Number(twisty.dataset.twisty);
    if (!state.collapsed.delete(index)) {
      state.collapsed.add(index);
    }
    render({ keepScroll: true });
  } else if (track) {
    Object.assign(state, { track: track.dataset.track === "" ? null : Number(track.dataset.track), page: 0, chosen: null });
    render();
  } else if (page) {
    Object.assign(state, { page: state.page + Number(page.dataset.page), chosen: null });
    render();
  } else if (row) {
    choose(Number(row.dataset.index));
  }
});

list.addEventListener("keydown", (event) => {
  const row = event.target.closest("tr[data-index]");
  if (row && (event.key === "Enter" || event.key === " ")) {
    event.preventDefault();
    choose(Number(row.dataset.index));
  }
});

inspector.addEventListener("click", (event) => {
  if (event.target.closest("[data-close]")) {
    choose(null);
  }
});

document.querySelector("#open").addEventListener("click", () => input.click());
input.addEventListener("change", () => open(input.files[0]));
window.addEventListener("dragover", (event) => event.preventDefault());
window.addEventListener("drop", (event) => {
  event.preventDefault();
  open(event.dataTransfer.files[0]);
});

render();
