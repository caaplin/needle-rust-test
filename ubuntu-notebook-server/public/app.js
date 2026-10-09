// Ubuntu Notebook frontend.
//
// This page talks to the Rust server. The browser asks for the basic auth
// username and password on the first request. The credentials are then sent
// with every request to this server.

const cellsEl = document.getElementById("cells");
const statusEl = document.getElementById("status");
const cellTemplate = document.getElementById("cell-template");

let cells = [];

// Call the server API. Return the decoded JSON body.
async function api(path, options = {}) {
  const response = await fetch(path, {
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    ...options,
  });
  if (response.status === 204) {
    return null;
  }
  const body = await response.json().catch(() => null);
  if (!response.ok) {
    const message =
      body && body.error ? body.error : `the request failed with status ${response.status}`;
    throw new Error(message);
  }
  return body;
}

// Show a message in the status line. Pass an empty string to clear it.
function showStatus(message) {
  statusEl.textContent = message;
}

// Show an error in the status line.
function showError(error) {
  showStatus(`Error: ${error.message}`);
}

// Draw all cells.
function renderCells() {
  cellsEl.innerHTML = "";
  if (cells.length === 0) {
    const hint = document.createElement("p");
    hint.className = "empty-hint";
    hint.textContent = "The notebook is empty. Add a command cell or a comment cell.";
    cellsEl.appendChild(hint);
    return;
  }
  for (const cell of cells) {
    cellsEl.appendChild(renderCell(cell));
  }
}

// Draw one cell. Return the cell element.
function renderCell(cell) {
  const node = cellTemplate.content.firstElementChild.cloneNode(true);
  node.dataset.id = cell.id;
  node.classList.add(cell.cell_type === "comment" ? "comment" : "command");
  node.querySelector(".cell-type").textContent = cell.cell_type;
  node.querySelector(".cell-id").textContent = `#${cell.id}`;

  const textarea = node.querySelector(".cell-content");
  textarea.value = cell.content;
  textarea.placeholder =
    cell.cell_type === "comment"
      ? "Write a comment..."
      : "Type a command, for example: ls -la";
  textarea.addEventListener("change", () => {
    saveCell(cell.id, { content: textarea.value }).catch(showError);
  });

  const output = node.querySelector(".cell-output");
  const runButton = node.querySelector(".run-button");
  if (cell.cell_type === "command") {
    if (cell.output) {
      output.hidden = false;
      output.textContent = cell.output;
    }
    runButton.addEventListener("click", () => {
      runCell(cell.id).catch(showError);
    });
  } else {
    runButton.hidden = true;
  }

  node.querySelector(".delete-button").addEventListener("click", () => {
    deleteCell(cell.id).catch(showError);
  });

  return node;
}

// Load the cell list from the server.
async function loadCells() {
  cells = await api("/api/cells");
  renderCells();
}

// Add a new cell. The cellType is "command" or "comment".
async function addCell(cellType) {
  const cell = await api("/api/cells", {
    method: "POST",
    body: JSON.stringify({ cell_type: cellType, content: "" }),
  });
  cells.push(cell);
  renderCells();
  showStatus(`Added a ${cellType} cell.`);
}

// Save changes to a cell.
async function saveCell(id, patch) {
  const updated = await api(`/api/cells/${encodeURIComponent(id)}`, {
    method: "PUT",
    body: JSON.stringify(patch),
  });
  cells = cells.map((cell) => (cell.id === id ? updated : cell));
  renderCells();
}

// Delete a cell.
async function deleteCell(id) {
  await api(`/api/cells/${encodeURIComponent(id)}`, { method: "DELETE" });
  cells = cells.filter((cell) => cell.id !== id);
  renderCells();
  showStatus("Deleted the cell.");
}

// Run the command in a cell. Show the output in the cell.
async function runCell(id) {
  const node = cellsEl.querySelector(`.cell[data-id="${CSS.escape(id)}"]`);
  if (!node) {
    return;
  }
  const command = node.querySelector(".cell-content").value;
  const output = node.querySelector(".cell-output");
  const runButton = node.querySelector(".run-button");

  runButton.disabled = true;
  output.hidden = false;
  output.textContent = "Running...";
  try {
    const result = await api("/api/execute", {
      method: "POST",
      body: JSON.stringify({ cell_id: id, command }),
    });
    const cell = cells.find((item) => item.id === id);
    if (cell) {
      cell.output = result.output;
    }
    let text = result.output;
    if (result.exit_code !== null && result.exit_code !== undefined) {
      text += `\n(exit code: ${result.exit_code})`;
    }
    output.textContent = text;
    showStatus("The command finished.");
  } catch (error) {
    output.textContent = `Error: ${error.message}`;
    showError(error);
  } finally {
    runButton.disabled = false;
  }
}

// Wire up the buttons. Load the cells.
document.getElementById("add-command").addEventListener("click", () => {
  addCell("command").catch(showError);
});
document.getElementById("add-comment").addEventListener("click", () => {
  addCell("comment").catch(showError);
});
loadCells().catch(showError);
