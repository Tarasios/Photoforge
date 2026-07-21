// Plain JS frontend — no framework, no bundler. Talks to the Rust backend
// through the Tauri v2 global (`window.__TAURI__`).

// Outside the Tauri webview (e.g. opening index.html in a browser) the global
// is missing; stub it so the UI still renders instead of dying on line one.
const tauri = window.__TAURI__;
const invoke = tauri
  ? tauri.core.invoke
  : async () => { throw new Error("backend not available — run the PhotoForge desktop app"); };
const listen = tauri ? tauri.event.listen : () => {};

const $ = (id) => document.getElementById(id);
const wire = (id, handler) => $(id).addEventListener("click", handler);

// ---------------------------------------------------------------- first run
if (!localStorage.getItem("pf_onboarded")) {
  $("onboarding").classList.remove("hidden");
}
wire("onboard-close", () => {
  localStorage.setItem("pf_onboarded", "1");
  $("onboarding").classList.add("hidden");
});
wire("help-btn", () => $("onboarding").classList.remove("hidden"));

// ---------------------------------------------------------------- tabs
document.querySelectorAll(".tab").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    document.querySelectorAll(".screen").forEach((s) => s.classList.add("hidden"));
    $(`screen-${btn.dataset.screen}`).classList.remove("hidden");
  });
});

// ---------------------------------------------------------------- library
function fmtWhen(ts) {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString();
}

async function refreshRoots() {
  try {
    const roots = await invoke("get_scan_roots");
    const table = $("roots-table");
    const body = $("roots-body");
    body.innerHTML = "";
    $("roots-empty").classList.toggle("hidden", roots.length > 0);
    table.classList.toggle("hidden", roots.length === 0);
    for (const r of roots) {
      const tr = document.createElement("tr");
      const rescan = document.createElement("button");
      rescan.textContent = "Re-scan";
      rescan.addEventListener("click", () => {
        $("root").value = r.path;
        $("scan-btn").click();
      });
      const cells = [r.path, String(r.scanned), String(r.errors), fmtWhen(r.last_scan_ts)];
      for (const [i, text] of cells.entries()) {
        const td = document.createElement("td");
        td.textContent = text;
        if (i === 0) td.className = "mono";
        tr.appendChild(td);
      }
      const tdBtn = document.createElement("td");
      tdBtn.appendChild(rescan);
      tr.appendChild(tdBtn);
      body.appendChild(tr);
    }
  } catch (err) {
    $("scan-out").textContent = `Error: ${err}`;
  }
}

async function refreshSkips() {
  try {
    const { builtin, user } = await invoke("get_skip_dirs");
    const chips = $("builtin-skips");
    chips.innerHTML = "";
    for (const name of builtin) {
      const c = document.createElement("span");
      c.className = "chip";
      c.textContent = name;
      chips.appendChild(c);
    }
    const list = $("user-skips");
    list.innerHTML = "";
    if (!user.length) {
      const li = document.createElement("li");
      li.className = "dim";
      li.textContent = "None yet.";
      list.appendChild(li);
    }
    for (const path of user) {
      const li = document.createElement("li");
      const span = document.createElement("span");
      span.className = "mono";
      span.textContent = path;
      const rm = document.createElement("button");
      rm.textContent = "Remove";
      rm.addEventListener("click", async () => {
        await invoke("remove_skip_dir", { path });
        refreshSkips();
      });
      li.append(span, rm);
      list.appendChild(li);
    }
  } catch (err) {
    $("scan-out").textContent = `Error: ${err}`;
  }
}

wire("add-skip-btn", async () => {
  try {
    const folder = await invoke("pick_folder");
    if (folder) {
      await invoke("add_skip_dir", { path: folder });
      refreshSkips();
    }
  } catch (err) {
    $("scan-out").textContent = `Error: ${err}`;
  }
});

listen("scan-progress", (ev) => {
  const { done, total } = ev.payload;
  $("scan-progress").classList.remove("hidden");
  $("scan-bar").style.width = total ? `${(100 * done) / total}%` : "0%";
  $("scan-out").textContent = `Processing ${done} / ${total}…`;
});

wire("browse-btn", async () => {
  try {
    const folder = await invoke("pick_folder");
    if (folder) $("root").value = folder;
  } catch (err) {
    $("scan-out").textContent = `Error: ${err}`;
  }
});

wire("scan-btn", async () => {
  const root = $("root").value.trim();
  if (!root) {
    $("scan-out").textContent = "Pick a folder first.";
    return;
  }
  $("scan-btn").disabled = true;
  $("scan-out").textContent = "Enumerating…";
  try {
    const s = await invoke("scan_dir", { root });
    $("scan-out").textContent =
      `Done: scanned ${s.scanned}, added ${s.added}, skipped ${s.skipped} unchanged, errors ${s.errors}.`;
    refreshRoots();
  } catch (err) {
    $("scan-out").textContent = `Error: ${err}`;
  } finally {
    $("scan-btn").disabled = false;
    $("scan-progress").classList.add("hidden");
  }
});

refreshRoots();
refreshSkips();

// ---------------------------------------------------------------- duplicates
let currentGroups = [];

$("dupe-mode").addEventListener("change", () => {
  $("k-wrap").style.display = $("dupe-mode").value === "near" ? "" : "none";
});
$("k-wrap").style.display = "none";

async function loadThumb(imgEl, path) {
  try {
    imgEl.src = await invoke("get_thumbnail", { path, maxEdge: 256 });
  } catch {
    imgEl.alt = "(no preview)";
  }
}

function fmtBytes(n) {
  if (n > 1048576) return `${(n / 1048576).toFixed(1)} MB`;
  if (n > 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${n} B`;
}

function renderGroups() {
  const host = $("dupe-groups");
  host.innerHTML = "";
  $("dupes-empty").classList.toggle("hidden", currentGroups.length > 0);
  currentGroups.forEach((g, gi) => {
    const card = document.createElement("div");
    card.className = "card group";
    const head = document.createElement("p");
    head.className = "group-head";
    head.textContent =
      `Group ${gi + 1} — ${g.files.length} files, ${fmtBytes(g.wasted_bytes)} reclaimable`;
    card.appendChild(head);

    const rowEl = document.createElement("div");
    rowEl.className = "thumb-row";
    g.files.forEach((f, fi) => {
      const cell = document.createElement("label");
      cell.className = "thumb-cell";
      const img = document.createElement("img");
      loadThumb(img, f.path);
      const cb = document.createElement("input");
      cb.type = "checkbox";
      cb.dataset.path = f.path;
      // "Keep largest" preselect: files are sorted largest-first, so
      // everything but index 0 is a move candidate.
      cb.checked = fi > 0;
      const meta = document.createElement("span");
      meta.className = "thumb-meta";
      const dims = f.width ? `${f.width}x${f.height}` : "?";
      meta.textContent = `${fmtBytes(f.size)} · ${dims}`;
      const path = document.createElement("span");
      path.className = "thumb-path";
      path.title = f.path;
      path.textContent = f.path;
      cell.append(cb, img, meta, path);
      rowEl.appendChild(cell);
    });
    card.appendChild(rowEl);
    host.appendChild(card);
  });
}

wire("find-dupes-btn", async () => {
  $("dupes-out").textContent = "Searching…";
  try {
    currentGroups =
      $("dupe-mode").value === "near"
        ? await invoke("get_near_dupes", { k: Number($("dupe-k").value) || 5 })
        : await invoke("get_exact_dupes");
    const wasted = currentGroups.reduce((a, g) => a + g.wasted_bytes, 0);
    $("dupes-out").textContent = currentGroups.length
      ? `${currentGroups.length} group(s), ${fmtBytes(wasted)} reclaimable.`
      : "No duplicates found.";
    renderGroups();
  } catch (err) {
    $("dupes-out").textContent = `Error: ${err}`;
  }
});

wire("keep-largest-btn", () => {
  document.querySelectorAll(".thumb-row").forEach((row) => {
    row.querySelectorAll("input[type=checkbox]").forEach((cb, i) => {
      cb.checked = i > 0;
    });
  });
});

wire("move-selected-btn", async () => {
  const paths = [...document.querySelectorAll("#dupe-groups input:checked")].map(
    (cb) => cb.dataset.path
  );
  if (!paths.length) {
    $("dupes-out").textContent = "Nothing selected.";
    return;
  }
  // _Duplicates lives next to the first selected file's parent folder.
  const first = paths[0];
  const destDir = first.slice(0, first.lastIndexOf("\\")) + "\\_Duplicates";
  try {
    const s = await invoke("move_to_duplicates", { paths, destDir });
    $("dupes-out").textContent =
      `Moved ${s.moved} file(s) to ${destDir} (${s.errors} errors). Undo is available.`;
    $("find-dupes-btn").click();
  } catch (err) {
    $("dupes-out").textContent = `Error: ${err}`;
  }
});

wire("undo-btn", async () => {
  try {
    const s = await invoke("undo_moves", { count: 1000 });
    $("dupes-out").textContent = `Undo: restored ${s.moved} file(s), ${s.errors} errors.`;
    $("find-dupes-btn").click();
  } catch (err) {
    $("dupes-out").textContent = `Error: ${err}`;
  }
});

// ---------------------------------------------------------------- review
let queue = [];
let cursor = 0;

wire("classify-btn", async () => {
  $("review-out").textContent = "Classifying…";
  try {
    const s = await invoke("run_classifier");
    $("review-out").textContent =
      `Classified: ${s.photo} photo · ${s.non_photo} non-photo · ${s.ambiguous} ambiguous.`;
  } catch (err) {
    $("review-out").textContent = `Error: ${err}`;
  }
});

async function showCurrent() {
  const empty = !queue.length || cursor < 0 || cursor >= queue.length;
  $("review-stage").classList.toggle("hidden", empty);
  $("review-empty").classList.toggle("hidden", !empty);
  if (empty) return;
  const item = queue[cursor];
  $("review-path").textContent = item.path;
  $("review-label").textContent = item.label ?? "?";
  $("review-rule").textContent = item.rule ? `(rule: ${item.rule})` : "";
  $("review-pos").textContent = `${cursor + 1} / ${queue.length}`;
  $("review-img").src = "";
  loadThumb($("review-img"), item.path);
}

wire("load-review-btn", async () => {
  try {
    queue = await invoke("get_review_queue", { limit: 500 });
    cursor = 0;
    if (!queue.length) $("review-out").textContent = "Queue empty — run the classifier first.";
    showCurrent();
  } catch (err) {
    $("review-out").textContent = `Error: ${err}`;
  }
});

wire("export-csv-btn", async () => {
  try {
    const dest = "photoforge_labels.csv";
    const n = await invoke("export_labels_csv", { dest });
    $("review-out").textContent = `Exported ${n} corrected label(s) to ${dest}.`;
  } catch (err) {
    $("review-out").textContent = `Error: ${err}`;
  }
});

async function decide(flip) {
  const item = queue[cursor];
  if (!item) return;
  const label = flip
    ? item.label === "photo" ? "non_photo" : "photo"
    : item.label === "non_photo" ? "non_photo" : "photo"; // confirming 'ambiguous' marks it a photo
  try {
    await invoke("set_label", { id: item.id, label });
    queue.splice(cursor, 1); // reviewed items leave the queue
    if (cursor >= queue.length) cursor = queue.length - 1;
    showCurrent();
  } catch (err) {
    $("review-out").textContent = `Error: ${err}`;
  }
}

// Keyboard-driven review: only when the Review screen is visible.
document.addEventListener("keydown", (ev) => {
  if ($("screen-review").classList.contains("hidden")) return;
  if (ev.target.tagName === "INPUT") return;
  switch (ev.key.toLowerCase()) {
    case "y": decide(false); break;
    case "n": decide(true); break;
    case "arrowright": cursor = Math.min(cursor + 1, queue.length - 1); showCurrent(); break;
    case "arrowleft": cursor = Math.max(cursor - 1, 0); showCurrent(); break;
  }
});
