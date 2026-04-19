function trimString(value) {
  return typeof value === "string" ? value.trim() : "";
}
function identityName(identity) {
  const name = trimString(identity == null ? void 0 : identity.name);
  return name || "moltis";
}
function identityEmoji(identity) {
  return trimString(identity == null ? void 0 : identity.emoji);
}
function formatPageTitle(identity) {
  return identityName(identity);
}
function formatLoginTitle(identity) {
  return identityName(identity);
}
function emojiFaviconPng(emoji) {
  const canvas = document.createElement("canvas");
  canvas.width = 64;
  canvas.height = 64;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.clearRect(0, 0, 64, 64);
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.font = "52px 'Apple Color Emoji','Segoe UI Emoji','Noto Color Emoji',sans-serif";
  ctx.fillText(emoji, 32, 34);
  return canvas.toDataURL("image/png");
}
function applyIdentityFavicon(identity) {
  const emoji = identityEmoji(identity);
  if (!emoji) return false;
  let links = Array.from(document.querySelectorAll('link[rel="icon"]'));
  if (links.length === 0) {
    const fallback = document.createElement("link");
    fallback.rel = "icon";
    document.head.appendChild(fallback);
    links = [fallback];
  }
  const href = emojiFaviconPng(emoji);
  if (!href) return false;
  for (const link of links) {
    link.type = "image/png";
    link.removeAttribute("sizes");
    link.href = href;
  }
  return true;
}
export {
  applyIdentityFavicon as a,
  formatLoginTitle as b,
  formatPageTitle as f
};
