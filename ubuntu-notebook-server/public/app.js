// Ubuntu Notebook frontend.
//
// This page talks to the Rust server. The browser asks for the basic auth
// username and password on the first request. The credentials are then sent
// with every request to this server.
//
// The page updates only the cell that changes. It does not redraw the whole
// notebook. A full redraw removes the buttons under the mouse pointer. That
// stops a click from reaching the button.

const cellsEl = document.getElementById("cells");
const statusEl = document.getElementById("status");
const statusTextEl = document.getElementById("status-text");
const statusSpinnerEl = document.getElementById("status-spinner");
const cellTemplate = document.getElementById("cell-template");
const runAllButton = document.getElementById("run-all");

// The cells, in notebook order. Each entry is a copy of the server state.
let cells = [];
// The DOM element of each cell, by cell id.
const nodes = new Map();
// The number of commands that are running now.
let activeRuns = 0;
// True while Run all is running.
let runAllActive = false;

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
  statusTextEl.textContent = message;
  updateStatusBar();
}

// Show an error in the status line.
function showError(error) {
  showStatus(`Error: ${error.message}`);
}

// Show the status line when it has a message or a command is running. Show
// the spinner in the status line while a command runs.
function updateStatusBar() {
  const busy = activeRuns > 0;
  statusSpinnerEl.hidden = !busy;
  statusEl.hidden = !busy && statusTextEl.textContent === "";
}

// Find a cell in the local copy. Return undefined when the cell is missing.
function findCell(id) {
  return cells.find((cell) => cell.id === id);
}

// Show the hint when the notebook has no cells.
function updateEmptyHint() {
  const hint = cellsEl.querySelector(".empty-hint");
  if (cells.length === 0 && !hint) {
    const newHint = document.createElement("p");
    newHint.className = "empty-hint";
    newHint.textContent = "The notebook is empty. Add a command cell or a comment cell.";
    cellsEl.appendChild(newHint);
  } else if (cells.length > 0 && hint) {
    hint.remove();
  }
}

// Draw all cells.
function renderCells() {
  cellsEl.innerHTML = "";
  nodes.clear();
  for (const cell of cells) {
    const node = createCellNode(cell);
    nodes.set(cell.id, node);
    cellsEl.appendChild(node);
  }
  updateEmptyHint();
}

// Create the element for one cell. Return the element.
function createCellNode(cell) {
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

  // Save when the text box loses focus. Do not redraw the cell.
  textarea.addEventListener("change", () => {
    saveContent(cell.id, textarea.value).catch(showError);
  });

  // Enter adds a new line. This is the default behavior. Shift+Enter runs the
  // cell and adds a new cell below it.
  textarea.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && event.shiftKey && !event.isComposing) {
      event.preventDefault();
      runAndAddBelow(cell.id).catch(showError);
    }
  });

  const runButton = node.querySelector(".run-button");
  const output = node.querySelector(".cell-output");
  if (cell.cell_type === "command") {
    if (cell.output) {
      output.hidden = false;
      output.textContent = cell.output;
    }
    runButton.addEventListener("click", () => {
      runCell(cell.id);
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

// Add a new cell. The cellType is "command" or "comment". When afterId is
// set, the new cell goes below that cell. Otherwise it goes to the end.
async function addCell(cellType, afterId = null) {
  const cell = await api("/api/cells", {
    method: "POST",
    body: JSON.stringify({ cell_type: cellType, content: "", after_id: afterId }),
  });

  const index = afterId ? cells.findIndex((item) => item.id === afterId) : -1;
  if (index >= 0) {
    cells.splice(index + 1, 0, cell);
  } else {
    cells.push(cell);
  }

  const node = createCellNode(cell);
  nodes.set(cell.id, node);
  const anchor = index >= 0 ? nodes.get(afterId) : undefined;
  if (anchor) {
    anchor.after(node);
  } else {
    cellsEl.appendChild(node);
  }
  updateEmptyHint();
  node.querySelector(".cell-content").focus();
  return cell;
}

// Save the content of a cell. Skip the request when the content did not change.
async function saveContent(id, content) {
  const cell = findCell(id);
  if (!cell || cell.content === content) {
    return;
  }
  await saveCell(id, { content });
}

// Save changes to a cell on the server. Update the local copy.
async function saveCell(id, patch) {
  const updated = await api(`/api/cells/${encodeURIComponent(id)}`, {
    method: "PUT",
    body: JSON.stringify(patch),
  });
  const cell = findCell(id);
  if (cell && patch.content !== undefined) {
    cell.content = updated.content;
  }
  return updated;
}

// Delete a cell.
async function deleteCell(id) {
  await api(`/api/cells/${encodeURIComponent(id)}`, { method: "DELETE" });
  cells = cells.filter((cell) => cell.id !== id);
  const node = nodes.get(id);
  if (node) {
    node.remove();
    nodes.delete(id);
  }
  updateEmptyHint();
  showStatus("Deleted the cell.");
}

// Show or hide the running state of a command cell.
function setCellRunning(node, running) {
  node.classList.toggle("running", running);
  node.querySelector(".run-button").disabled = running;
  node.querySelector(".run-label").textContent = running ? "Running" : "Run";
  node.querySelector(".run-spinner").hidden = !running;
  node.querySelector(".cell-running").hidden = !running;
}

// Show text in the output area of a cell.
function showOutput(node, text) {
  const output = node.querySelector(".cell-output");
  output.hidden = false;
  output.textContent = text;
}

// Run the command in a cell. Show a spinner while it runs. Show the output in
// the cell. Return the server result, or null when the run failed.
async function runCell(id) {
  const cell = findCell(id);
  const node = nodes.get(id);
  if (!cell || !node || cell.cell_type !== "command") {
    return null;
  }
  if (node.classList.contains("running")) {
    return null;
  }

  // Read the command now. The text box can change while the command runs.
  const command = node.querySelector(".cell-content").value;
  const elapsedEl = node.querySelector(".run-elapsed");
  const started = Date.now();

  setCellRunning(node, true);
  elapsedEl.textContent = "0 s";
  const timer = setInterval(() => {
    elapsedEl.textContent = `${Math.floor((Date.now() - started) / 1000)} s`;
  }, 250);
  node.querySelector(".cell-output").hidden = true;
  activeRuns += 1;
  updateStatusBar();

  // Save the content for the next page load. The run sends the command, so
  // do not wait for the save.
  saveContent(id, command).catch(showError);

  try {
    const result = await api("/api/execute", {
      method: "POST",
      body: JSON.stringify({ cell_id: id, command }),
    });
    const current = findCell(id);
    if (current) {
      current.output = result.output;
    }
    let text = result.output;
    if (result.exit_code !== null && result.exit_code !== undefined) {
      text += `\n(exit code: ${result.exit_code})`;
    }
    showOutput(node, text);
    if (!runAllActive) {
      showStatus("The command finished.");
    }
    return result;
  } catch (error) {
    showOutput(node, `Error: ${error.message}`);
    showError(error);
    return null;
  } finally {
    clearInterval(timer);
    setCellRunning(node, false);
    activeRuns -= 1;
    updateStatusBar();
  }
}

// Run the cell and add a new command cell below it. A comment cell does not
// run. It only adds the new cell. The run continues while the new cell opens.
async function runAndAddBelow(id) {
  const cell = findCell(id);
  if (!cell) {
    return;
  }
  if (cell.cell_type === "command") {
    runCell(id);
  }
  await addCell("command", id);
}

// Run every command cell in order. Continue when a command fails. Show the
// progress in the status line.
async function runAll() {
  if (runAllActive) {
    return;
  }
  const ids = cells
    .filter((cell) => cell.cell_type === "command")
    .map((cell) => cell.id)
    .filter((id) => {
      const node = nodes.get(id);
      return node && node.querySelector(".cell-content").value.trim() !== "";
    });
  if (ids.length === 0) {
    showStatus("There are no command cells to run.");
    return;
  }

  runAllActive = true;
  runAllButton.disabled = true;
  let failed = 0;
  try {
    for (let index = 0; index < ids.length; index += 1) {
      showStatus(`Running cell ${index + 1} of ${ids.length}.`);
      const result = await runCell(ids[index]);
      if (!result || result.exit_code !== 0) {
        failed += 1;
      }
    }
    showStatus(
      failed === 0
        ? `Ran ${ids.length} cells. All cells finished with exit code 0.`
        : `Ran ${ids.length} cells. ${failed} cell(s) failed or did not run.`
    );
  } finally {
    runAllActive = false;
    runAllButton.disabled = false;
  }
}

// Wire up the buttons. Load the cells.
document.getElementById("add-command").addEventListener("click", () => {
  addCell("command")
    .then(() => showStatus("Added a command cell."))
    .catch(showError);
});
document.getElementById("add-comment").addEventListener("click", () => {
  addCell("comment")
    .then(() => showStatus("Added a comment cell."))
    .catch(showError);
});
runAllButton.addEventListener("click", () => {
  runAll().catch(showError);
});
updateStatusBar();
loadCells().catch(showError);
