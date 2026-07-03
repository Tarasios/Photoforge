// Plain JS frontend — no framework, no bundler. Talks to the Rust backend
// through the Tauri v2 global (`window.__TAURI__.core.invoke`).

const { invoke } = window.__TAURI__.core;

function wire(id, handler) {
  document.getElementById(id).addEventListener("click", handler);
}

wire("greet-btn", async () => {
  const name = document.getElementById("name").value || "world";
  const out = document.getElementById("greet-out");
  try {
    out.textContent = await invoke("greet", { name });
  } catch (err) {
    out.textContent = `Error: ${err}`;
  }
});

wire("scan-btn", async () => {
  const root = document.getElementById("root").value.trim();
  const out = document.getElementById("scan-out");
  if (!root) {
    out.textContent = "Enter a folder path first.";
    return;
  }
  out.textContent = "Scanning…";
  try {
    const s = await invoke("scan_dir", { root });
    out.textContent = `Scanned ${s.scanned}, added ${s.added}, skipped ${s.skipped}, errors ${s.errors} (core v${s.core_version}).`;
  } catch (err) {
    out.textContent = `Error: ${err}`;
  }
});
