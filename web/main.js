const input = document.querySelector("#file");
const status = document.querySelector("#status");
const rows = document.querySelector("#boxes tbody");

input.addEventListener("change", () => {
  const [file] = input.files;
  if (!file) {
    return;
  }
  rows.replaceChildren();
  status.textContent = `Reading ${file.name}…`;

  const worker = new Worker("worker.js", { type: "module" });
  worker.onmessage = ({ data }) => {
    worker.terminate();
    if (data.reply === "error") {
      status.textContent = data.message;
      return;
    }
    status.textContent = `${data.boxes.length} boxes in ${file.name}`;
    const fragment = document.createDocumentFragment();
    for (const box of data.boxes) {
      fragment.append(row(box));
    }
    rows.replaceChildren(fragment);
  };
  worker.onerror = (event) => {
    worker.terminate();
    status.textContent = event.message;
  };
  worker.postMessage({ request: "dump", file });
});

function row({ boxType, offset, size, depth }) {
  const cells = [boxType, offset, size ?? "to end of file"];
  const rowElement = document.createElement("tr");
  for (const text of cells) {
    const cell = document.createElement("td");
    cell.textContent = text;
    rowElement.append(cell);
  }
  rowElement.style.setProperty("--depth", depth);
  return rowElement;
}
