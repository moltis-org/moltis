// Blocking theme init — prevents flash of wrong theme on share pages.
let t: string = localStorage.getItem("moltis-theme") || "system";
if (t === "system") t = matchMedia("(prefers-color-scheme:dark)").matches ? "dark" : "light";
document.documentElement.setAttribute("data-theme", t);
