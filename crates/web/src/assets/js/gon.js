const gon = window.__MOLTIS__ || {};
const listeners = {};
function get(key) {
  return gon[key] ?? null;
}
function set(key, value) {
  gon[key] = value;
  notify(key, value);
}
function onChange(key, fn) {
  if (!listeners[key]) listeners[key] = [];
  listeners[key].push(fn);
}
function refresh() {
  return fetch(`/api/gon?_=${Date.now()}`, {
    cache: "no-store",
    headers: {
      "Cache-Control": "no-cache",
      Pragma: "no-cache"
    }
  }).then((r) => r.ok ? r.json() : null).then((data) => {
    if (!data) return;
    for (const key of Object.keys(data)) {
      gon[key] = data[key];
      notify(key, data[key]);
    }
  });
}
function notify(key, value) {
  for (const fn of listeners[key] || []) fn(value);
}
export {
  get,
  onChange,
  refresh,
  set
};
