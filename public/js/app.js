// Full-panel navigation gets a deliberate focus destination. Small live updates
// (search results, flight cards and polling summaries) leave focus untouched.
document.addEventListener("htmx:after:swap", (event) => {
  const original = event.detail?.ctx?.target;
  const panelIds = new Set([
    "admin-panel", "flights-panel", "flight-create-panel", "flight-create-slot", "auth-panel", "profile-panel",
  ]);
  if (!original?.id || !panelIds.has(original.id)) return;
  const panel = document.getElementById(original.id);
  if (!panel) return;
  const destination = panel.querySelector('[aria-invalid="true"]')
    || panel.querySelector('[role="alert"]')
    || (original.id === "admin-panel" ? panel.querySelector("h2") : null)
    || panel.querySelector("h1, h2");
  if (!destination) return;
  if (!destination.matches("input, select, textarea, button, a[href]")) {
    destination.setAttribute("tabindex", "-1");
  }
  // HTMX finishes updating document.title immediately after this event.
  requestAnimationFrame(() => {
    if (destination.isConnected) destination.focus();
  });
});
