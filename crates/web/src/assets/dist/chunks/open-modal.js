import { b as sendRpc } from "./theme.js";
import { e as els, s as showApiKeyForm, a as showOAuthFlow, b as showLocalModelFlow, c as showCustomProviderForm } from "../main.js";
import "./ws-connect.js";
import "./branding.js";
import "./voice-utils.js";
import "./time-format.js";
function openProviderModalImpl() {
  const m = els();
  m.modal.classList.remove("hidden");
  m.title.textContent = "Add LLM";
  m.body.textContent = "Loading...";
  sendRpc("providers.available", {}).then((res) => {
    if (!(res == null ? void 0 : res.ok)) {
      m.body.textContent = "Failed to load LLM providers.";
      return;
    }
    const providers = res.payload || [];
    providers.sort((a, b) => {
      const aOrder = Number.isFinite(a.uiOrder) ? a.uiOrder : Number.MAX_SAFE_INTEGER;
      const bOrder = Number.isFinite(b.uiOrder) ? b.uiOrder : Number.MAX_SAFE_INTEGER;
      if (aOrder !== bOrder) return aOrder - bOrder;
      return a.displayName.localeCompare(b.displayName);
    });
    m.body.textContent = "";
    providers.forEach((p) => {
      const item = document.createElement("div");
      item.className = "provider-item";
      const name = document.createElement("span");
      name.className = "provider-item-name";
      name.textContent = p.displayName;
      item.appendChild(name);
      const badges = document.createElement("div");
      badges.className = "badge-row";
      if (p.configured) {
        const check = document.createElement("span");
        check.className = "provider-item-badge configured";
        check.textContent = "configured";
        badges.appendChild(check);
      }
      if (p.isCustom) {
        const customBadge = document.createElement("span");
        customBadge.className = "provider-item-badge api-key";
        customBadge.textContent = "Custom";
        badges.appendChild(customBadge);
      } else {
        const badge = document.createElement("span");
        badge.className = `provider-item-badge ${p.authType}`;
        if (p.authType === "oauth") {
          badge.textContent = "OAuth";
        } else if (p.authType === "local") {
          badge.textContent = "Local";
        } else {
          badge.textContent = "API Key";
        }
        badges.appendChild(badge);
      }
      item.appendChild(badges);
      item.addEventListener("click", () => {
        if (p.authType === "api-key") showApiKeyForm(p);
        else if (p.authType === "oauth") showOAuthFlow(p);
        else if (p.authType === "local") showLocalModelFlow(p);
      });
      m.body.appendChild(item);
    });
    const separator = document.createElement("div");
    separator.className = "border-t border-[var(--border)] my-2";
    m.body.appendChild(separator);
    const customItem = document.createElement("div");
    customItem.className = "provider-item";
    const customName = document.createElement("span");
    customName.className = "provider-item-name";
    customName.textContent = "OpenAI Compatible";
    customItem.appendChild(customName);
    const customBadges = document.createElement("div");
    customBadges.className = "badge-row";
    const anyBadge = document.createElement("span");
    anyBadge.className = "provider-item-badge api-key";
    anyBadge.textContent = "Any Endpoint";
    customBadges.appendChild(anyBadge);
    customItem.appendChild(customBadges);
    customItem.addEventListener("click", showCustomProviderForm);
    m.body.appendChild(customItem);
  });
}
export {
  openProviderModalImpl
};
