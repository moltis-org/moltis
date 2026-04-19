import { d as sendRpc, by as refreshProvidersPage, $ } from "./theme.js";
import { e as els, c as createValidationProgress, o as openProviderModal, s as setFormError, a as setValidationProgress, r as resetValidationProgress, b as createValidationRequestId, d as bindValidationProgressEvents, f as completeValidationProgress, g as showModelSelector, h as fetchModels, i as closeProviderModal, j as showApiKeyForm, k as showOAuthFlow } from "../main.js";
import { F as validateProviderKey, o as onEvent } from "./voice-utils.js";
import "./jsxRuntime.module.js";
import "./branding.js";
import "./time-format.js";
function showCustomProviderForm() {
  const m = els();
  m.title.textContent = "OpenAI Compatible";
  m.body.textContent = "";
  const form = document.createElement("div");
  form.className = "provider-key-form";
  const urlLabel = document.createElement("label");
  urlLabel.className = "text-xs text-[var(--muted)]";
  urlLabel.textContent = "Endpoint URL";
  form.appendChild(urlLabel);
  const urlInp = document.createElement("input");
  urlInp.className = "provider-key-input";
  urlInp.type = "text";
  urlInp.placeholder = "https://api.example.com/v1";
  form.appendChild(urlInp);
  const keyLabel = document.createElement("label");
  keyLabel.className = "text-xs text-[var(--muted)] mt-2";
  keyLabel.textContent = "API Key";
  form.appendChild(keyLabel);
  const keyInp = document.createElement("input");
  keyInp.className = "provider-key-input";
  keyInp.type = "password";
  keyInp.placeholder = "sk-...";
  form.appendChild(keyInp);
  const modelLabel = document.createElement("label");
  modelLabel.className = "text-xs text-[var(--muted)] mt-2";
  modelLabel.textContent = "Model ID (optional)";
  form.appendChild(modelLabel);
  const modelInp = document.createElement("input");
  modelInp.className = "provider-key-input";
  modelInp.type = "text";
  modelInp.placeholder = "Leave blank for auto-discovery";
  form.appendChild(modelInp);
  const errorPanel = document.createElement("div");
  errorPanel.className = "alert-error-text text-[var(--error)] whitespace-pre-line";
  errorPanel.style.display = "none";
  form.appendChild(errorPanel);
  const validationProgress = createValidationProgress(form, "mt-1");
  const btns = document.createElement("div");
  btns.className = "btn-row";
  btns.style.marginTop = "12px";
  const backBtn = document.createElement("button");
  backBtn.className = "provider-btn provider-btn-secondary";
  backBtn.textContent = "Back";
  backBtn.addEventListener("click", openProviderModal);
  btns.appendChild(backBtn);
  const saveBtn = document.createElement("button");
  saveBtn.className = "provider-btn";
  saveBtn.textContent = "Add Provider";
  saveBtn.addEventListener("click", () => {
    const url = urlInp.value.trim();
    const key = keyInp.value.trim();
    const model = modelInp.value.trim() || null;
    if (!url) {
      setFormError(errorPanel, "Endpoint URL is required.");
      return;
    }
    if (!key) {
      setFormError(errorPanel, "API key is required.");
      return;
    }
    saveBtn.disabled = true;
    saveBtn.textContent = "Adding...";
    setValidationProgress(validationProgress, 8, "Saving provider settings...");
    setFormError(errorPanel, null);
    sendRpc("providers.add_custom", { baseUrl: url, apiKey: key, model }).then((res) => {
      var _a;
      if (!(res == null ? void 0 : res.ok)) {
        saveBtn.disabled = false;
        saveBtn.textContent = "Add Provider";
        resetValidationProgress(validationProgress);
        setFormError(errorPanel, ((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message) || "Failed to add provider.");
        return;
      }
      const result = res.payload;
      const providerName = result.providerName;
      const displayName = result.displayName;
      const requestId = createValidationRequestId();
      setValidationProgress(validationProgress, 12, "Discovering models...");
      const stopProgressEvents = bindValidationProgressEvents(validationProgress, requestId);
      validateProviderKey(providerName, key, url, model, requestId).then((valResult) => {
        if (!(valResult.valid || model)) {
          saveBtn.disabled = false;
          saveBtn.textContent = "Add Provider";
          resetValidationProgress(validationProgress);
          setFormError(errorPanel, valResult.error || "No models discovered. Please specify a model ID.");
          return;
        }
        if (valResult.models && valResult.models.length > 0) {
          completeValidationProgress(validationProgress, "Done.");
          const customProvider = {
            name: providerName,
            displayName,
            authType: "api-key",
            keyOptional: false
          };
          showModelSelector(customProvider, valResult.models, key, url, model, true);
        } else if (model) {
          sendRpc("providers.save_model", { provider: providerName, model }).then(() => {
            completeValidationProgress(validationProgress, "Done.");
            fetchModels();
            if (refreshProvidersPage) refreshProvidersPage();
            m.body.textContent = "";
            const status = document.createElement("div");
            status.className = "provider-status";
            status.textContent = `${displayName} configured successfully!`;
            m.body.appendChild(status);
            setTimeout(closeProviderModal, 1500);
          });
        } else {
          saveBtn.disabled = false;
          saveBtn.textContent = "Add Provider";
          resetValidationProgress(validationProgress);
          setFormError(errorPanel, "No models discovered. Please specify a model ID.");
        }
      }).catch((err) => {
        saveBtn.disabled = false;
        saveBtn.textContent = "Add Provider";
        resetValidationProgress(validationProgress);
        setFormError(errorPanel, (err == null ? void 0 : err.message) || "Validation failed.");
      }).finally(() => {
        stopProgressEvents();
      });
    }).catch((err) => {
      saveBtn.disabled = false;
      saveBtn.textContent = "Add Provider";
      resetValidationProgress(validationProgress);
      setFormError(errorPanel, (err == null ? void 0 : err.message) || "Failed to add provider.");
    });
  });
  btns.appendChild(saveBtn);
  form.appendChild(btns);
  m.body.appendChild(form);
  urlInp.focus();
}
let selectedBackend = null;
function showLocalModelFlow(provider) {
  const m = els();
  m.title.textContent = provider.displayName;
  m.body.textContent = "Loading system info...";
  sendRpc("providers.local.system_info", {}).then((sysRes) => {
    var _a;
    if (!(sysRes == null ? void 0 : sysRes.ok)) {
      m.body.textContent = ((_a = sysRes == null ? void 0 : sysRes.error) == null ? void 0 : _a.message) || "Failed to get system info";
      return;
    }
    const sysInfo = sysRes.payload;
    sendRpc("providers.local.models", {}).then((modelsRes) => {
      var _a2;
      if (!(modelsRes == null ? void 0 : modelsRes.ok)) {
        m.body.textContent = ((_a2 = modelsRes == null ? void 0 : modelsRes.error) == null ? void 0 : _a2.message) || "Failed to get models";
        return;
      }
      const modelsData = modelsRes.payload;
      renderLocalModelSelection(provider, sysInfo, modelsData);
    });
  });
}
function renderLocalModelSelection(provider, sysInfo, modelsData) {
  const m = els();
  m.body.textContent = "";
  selectedBackend = sysInfo.recommendedBackend || "GGUF";
  const wrapper = document.createElement("div");
  wrapper.className = "provider-key-form";
  const sysSection = document.createElement("div");
  sysSection.className = "flex flex-col gap-2 mb-4";
  const sysTitle = document.createElement("div");
  sysTitle.className = "text-xs font-medium text-[var(--text-strong)]";
  sysTitle.textContent = "System Info";
  sysSection.appendChild(sysTitle);
  const sysDetails = document.createElement("div");
  sysDetails.className = "flex gap-3 text-xs text-[var(--muted)]";
  const ramSpan = document.createElement("span");
  ramSpan.textContent = `RAM: ${sysInfo.totalRamGb}GB`;
  sysDetails.appendChild(ramSpan);
  const tierSpan = document.createElement("span");
  tierSpan.textContent = `Tier: ${sysInfo.memoryTier}`;
  sysDetails.appendChild(tierSpan);
  if (sysInfo.hasGpu) {
    const gpuSpan = document.createElement("span");
    gpuSpan.className = "text-[var(--ok)]";
    gpuSpan.textContent = "GPU available";
    sysDetails.appendChild(gpuSpan);
  }
  sysSection.appendChild(sysDetails);
  wrapper.appendChild(sysSection);
  const backends = sysInfo.availableBackends || [];
  if (sysInfo.isAppleSilicon && backends.length > 0) {
    const backendSection = document.createElement("div");
    backendSection.className = "flex flex-col gap-2 mb-4";
    const backendLabel = document.createElement("div");
    backendLabel.className = "text-xs font-medium text-[var(--text-strong)]";
    backendLabel.textContent = "Inference Backend";
    backendSection.appendChild(backendLabel);
    const backendCards = document.createElement("div");
    backendCards.className = "flex flex-col gap-2";
    backends.forEach((b) => {
      const card = document.createElement("div");
      card.className = "backend-card";
      if (!b.available) card.className += " disabled";
      if (b.id === selectedBackend) card.className += " selected";
      card.dataset.backendId = b.id;
      const header = document.createElement("div");
      header.className = "flex items-center justify-between";
      const name = document.createElement("span");
      name.className = "backend-name text-sm font-medium text-[var(--text)]";
      name.textContent = b.name;
      header.appendChild(name);
      const badges = document.createElement("div");
      badges.className = "flex gap-2";
      if (b.id === sysInfo.recommendedBackend && b.available) {
        const recBadge = document.createElement("span");
        recBadge.className = "recommended-badge";
        recBadge.textContent = "Recommended";
        badges.appendChild(recBadge);
      }
      if (!b.available) {
        const unavailBadge = document.createElement("span");
        unavailBadge.className = "tier-badge";
        unavailBadge.textContent = "Not installed";
        badges.appendChild(unavailBadge);
      }
      header.appendChild(badges);
      card.appendChild(header);
      const desc = document.createElement("div");
      desc.className = "text-xs text-[var(--muted)] mt-1";
      desc.textContent = b.description;
      card.appendChild(desc);
      if (!b.available && b.id === "MLX") {
        const cmds = b.installCommands || ["pip install mlx-lm"];
        const tpl = $("tpl-install-hint");
        const hintEl = tpl.content.cloneNode(true).firstElementChild;
        const labelEl = hintEl.querySelector("[data-install-label]");
        const container = hintEl.querySelector("[data-install-commands]");
        labelEl.textContent = cmds.length === 1 ? "Install with:" : "Install with any of:";
        const cmdTpl = $("tpl-install-cmd");
        cmds.forEach((c) => {
          const cmdEl = cmdTpl.content.cloneNode(true).firstElementChild;
          cmdEl.textContent = c;
          container.appendChild(cmdEl);
        });
        card.appendChild(hintEl);
      }
      if (b.available) {
        card.addEventListener("click", () => {
          backendCards.querySelectorAll(".backend-card").forEach((c) => {
            c.classList.remove("selected");
          });
          card.classList.add("selected");
          selectedBackend = b.id;
          if (wrapper._renderModelsForBackend) {
            wrapper._renderModelsForBackend(b.id);
          }
          if (wrapper._updateFilenameVisibility) {
            wrapper._updateFilenameVisibility(b.id);
          }
        });
      }
      backendCards.appendChild(card);
    });
    backendSection.appendChild(backendCards);
    wrapper.appendChild(backendSection);
  } else if (sysInfo.backendNote) {
    const backendDiv = document.createElement("div");
    backendDiv.className = "text-xs text-[var(--muted)] mb-4";
    backendDiv.textContent = `Backend: ${sysInfo.backendNote}`;
    wrapper.appendChild(backendDiv);
  }
  const modelsTitle = document.createElement("div");
  modelsTitle.className = "text-xs font-medium text-[var(--text-strong)] mb-2";
  modelsTitle.textContent = "Select a Model";
  wrapper.appendChild(modelsTitle);
  const modelsList = document.createElement("div");
  modelsList.className = "flex flex-col gap-2";
  modelsList.id = "local-model-list";
  function renderModelsForBackend(backend) {
    modelsList.textContent = "";
    const recommended = modelsData.recommended || [];
    const filtered = recommended.filter((mdl) => mdl.backend === backend);
    if (filtered.length === 0) {
      const empty = document.createElement("div");
      empty.className = "text-xs text-[var(--muted)] py-4 text-center";
      empty.textContent = `No models available for ${backend}`;
      modelsList.appendChild(empty);
      return;
    }
    filtered.forEach((model) => {
      const card = createModelCard(model, provider, sysInfo.totalRamGb);
      modelsList.appendChild(card);
    });
  }
  renderModelsForBackend(selectedBackend);
  wrapper._renderModelsForBackend = renderModelsForBackend;
  wrapper.appendChild(modelsList);
  const searchSection = document.createElement("div");
  searchSection.className = "flex flex-col gap-2 mt-4 pt-4 border-t border-[var(--border)]";
  const searchLabel = document.createElement("div");
  searchLabel.className = "text-xs font-medium text-[var(--text-strong)]";
  searchLabel.textContent = "Search HuggingFace";
  searchSection.appendChild(searchLabel);
  const searchRow = document.createElement("div");
  searchRow.className = "flex gap-2";
  const searchInput = document.createElement("input");
  searchInput.type = "text";
  searchInput.placeholder = "Search models...";
  searchInput.className = "provider-input flex-1";
  searchRow.appendChild(searchInput);
  const searchBtn = document.createElement("button");
  searchBtn.className = "provider-btn provider-btn-secondary";
  searchBtn.textContent = "Search";
  searchRow.appendChild(searchBtn);
  searchSection.appendChild(searchRow);
  const searchResults = document.createElement("div");
  searchResults.className = "flex flex-col gap-2 max-h-48 overflow-y-auto";
  searchResults.id = "hf-search-results";
  searchSection.appendChild(searchResults);
  const doSearch = async () => {
    var _a, _b;
    const query = searchInput.value.trim();
    if (!query) return;
    searchBtn.disabled = true;
    searchBtn.textContent = "Searching...";
    searchResults.textContent = "";
    const res = await sendRpc("providers.local.search_hf", {
      query,
      backend: selectedBackend,
      limit: 15
    });
    searchBtn.disabled = false;
    searchBtn.textContent = "Search";
    if (!((res == null ? void 0 : res.ok) && ((_b = (_a = res.payload) == null ? void 0 : _a.results) == null ? void 0 : _b.length))) {
      const noResults = document.createElement("div");
      noResults.className = "text-xs text-[var(--muted)] py-2";
      noResults.textContent = "No results found";
      searchResults.appendChild(noResults);
      return;
    }
    res.payload.results.forEach((result) => {
      const card = createHfSearchResultCard(result, provider);
      searchResults.appendChild(card);
    });
  };
  searchBtn.addEventListener("click", doSearch);
  searchInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing) doSearch();
  });
  let searchTimeout = null;
  searchInput.addEventListener("input", () => {
    if (searchTimeout) clearTimeout(searchTimeout);
    const query = searchInput.value.trim();
    if (query.length >= 2) {
      searchTimeout = setTimeout(doSearch, 500);
    }
  });
  wrapper.appendChild(searchSection);
  const customSection = document.createElement("div");
  customSection.className = "flex flex-col gap-2 mt-4 pt-4 border-t border-[var(--border)]";
  const customLabel = document.createElement("div");
  customLabel.className = "text-xs font-medium text-[var(--text-strong)]";
  customLabel.textContent = "Or enter HuggingFace repo URL";
  customSection.appendChild(customLabel);
  const customRow = document.createElement("div");
  customRow.className = "flex gap-2";
  const customInput = document.createElement("input");
  customInput.type = "text";
  customInput.placeholder = selectedBackend === "MLX" ? "mlx-community/Model-Name" : "TheBloke/Model-GGUF";
  customInput.className = "provider-input flex-1";
  customRow.appendChild(customInput);
  const customBtn = document.createElement("button");
  customBtn.className = "provider-btn";
  customBtn.textContent = "Use";
  customRow.appendChild(customBtn);
  customSection.appendChild(customRow);
  const filenameRow = document.createElement("div");
  filenameRow.className = "flex gap-2";
  filenameRow.style.display = selectedBackend === "GGUF" ? "flex" : "none";
  const filenameInput = document.createElement("input");
  filenameInput.type = "text";
  filenameInput.placeholder = "model-file.gguf (required for GGUF)";
  filenameInput.className = "provider-input flex-1";
  filenameRow.appendChild(filenameInput);
  customSection.appendChild(filenameRow);
  wrapper._updateFilenameVisibility = (backend) => {
    filenameRow.style.display = backend === "GGUF" ? "flex" : "none";
    customInput.placeholder = backend === "MLX" ? "mlx-community/Model-Name" : "TheBloke/Model-GGUF";
  };
  customBtn.addEventListener("click", async () => {
    var _a;
    const repo = customInput.value.trim();
    if (!repo) return;
    const params = {
      hfRepo: repo,
      backend: selectedBackend
    };
    if (selectedBackend === "GGUF") {
      const filename = filenameInput.value.trim();
      if (!filename) {
        filenameInput.focus();
        return;
      }
      params.hfFilename = filename;
    }
    customBtn.disabled = true;
    customBtn.textContent = "Configuring...";
    const res = await sendRpc("providers.local.configure_custom", params);
    customBtn.disabled = false;
    customBtn.textContent = "Use";
    if (res == null ? void 0 : res.ok) {
      fetchModels();
      if (refreshProvidersPage) refreshProvidersPage();
      showModelDownloadProgress({ id: res.payload.modelId, displayName: repo }, provider);
    } else {
      const err = ((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message) || "Failed to configure model";
      const errEl = document.createElement("div");
      errEl.className = "text-xs text-[var(--error)] py-2";
      errEl.textContent = err;
      searchResults.textContent = "";
      searchResults.appendChild(errEl);
    }
  });
  wrapper.appendChild(customSection);
  const btns = document.createElement("div");
  btns.className = "btn-row mt-4";
  const backBtn = document.createElement("button");
  backBtn.className = "provider-btn provider-btn-secondary";
  backBtn.textContent = "Back";
  backBtn.addEventListener("click", openProviderModal);
  btns.appendChild(backBtn);
  wrapper.appendChild(btns);
  m.body.appendChild(wrapper);
}
function createHfSearchResultCard(model, provider) {
  const card = document.createElement("div");
  card.className = "model-card";
  const header = document.createElement("div");
  header.className = "flex items-center justify-between";
  const name = document.createElement("span");
  name.className = "text-sm font-medium text-[var(--text)]";
  name.textContent = model.displayName;
  header.appendChild(name);
  const stats = document.createElement("div");
  stats.className = "flex gap-2 text-xs text-[var(--muted)]";
  if (model.downloads) {
    const dl = document.createElement("span");
    dl.textContent = `↓${formatDownloads(model.downloads)}`;
    stats.appendChild(dl);
  }
  if (model.likes) {
    const likes = document.createElement("span");
    likes.textContent = `♥${model.likes}`;
    stats.appendChild(likes);
  }
  header.appendChild(stats);
  card.appendChild(header);
  const repo = document.createElement("div");
  repo.className = "text-xs text-[var(--muted)] mt-1";
  repo.textContent = model.id;
  card.appendChild(repo);
  card.addEventListener("click", async () => {
    var _a;
    if (card.dataset.configuring) return;
    card.dataset.configuring = "true";
    const params = {
      hfRepo: model.id,
      backend: model.backend
    };
    if (model.backend === "GGUF") {
      const filename = prompt("Enter the GGUF filename (e.g., model-q4_k_m.gguf):");
      if (!filename) {
        delete card.dataset.configuring;
        return;
      }
      params.hfFilename = filename;
    }
    card.style.opacity = "0.5";
    card.style.pointerEvents = "none";
    const modalEls = els();
    modalEls.body.textContent = "";
    const statusWrapper = document.createElement("div");
    statusWrapper.className = "provider-key-form";
    const statusText = document.createElement("div");
    statusText.className = "text-sm text-[var(--text)]";
    statusText.textContent = `Configuring ${model.displayName}...`;
    statusWrapper.appendChild(statusText);
    modalEls.body.appendChild(statusWrapper);
    const res = await sendRpc("providers.local.configure_custom", params);
    if (res == null ? void 0 : res.ok) {
      fetchModels();
      if (refreshProvidersPage) refreshProvidersPage();
      showModelDownloadProgress(
        { id: res.payload.modelId, displayName: model.displayName },
        provider
      );
    } else {
      const err = ((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message) || "Failed to configure model";
      statusText.className = "text-sm text-[var(--error)]";
      statusText.textContent = err;
    }
  });
  return card;
}
function formatDownloads(n) {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`;
  return n.toString();
}
function createModelCard(model, provider, totalRamGb) {
  const card = document.createElement("div");
  card.className = "model-card";
  const detectedRamGb = Number.isFinite(totalRamGb) ? totalRamGb : 0;
  const hasEnoughRam = detectedRamGb >= model.minRamGb;
  const header = document.createElement("div");
  header.className = "flex items-center justify-between";
  const name = document.createElement("span");
  name.className = "text-sm font-medium text-[var(--text)]";
  name.textContent = model.displayName;
  header.appendChild(name);
  const badges = document.createElement("div");
  badges.className = "flex gap-2";
  const ramBadge = document.createElement("span");
  ramBadge.className = "tier-badge";
  ramBadge.textContent = `${model.minRamGb}GB`;
  badges.appendChild(ramBadge);
  if (model.suggested && hasEnoughRam) {
    const suggestedBadge = document.createElement("span");
    suggestedBadge.className = "recommended-badge";
    suggestedBadge.textContent = "Recommended";
    badges.appendChild(suggestedBadge);
  }
  if (!hasEnoughRam) {
    const insufficientBadge = document.createElement("span");
    insufficientBadge.className = "tier-badge";
    insufficientBadge.textContent = "Insufficient RAM";
    badges.appendChild(insufficientBadge);
  }
  header.appendChild(badges);
  card.appendChild(header);
  const meta = document.createElement("div");
  meta.className = "text-xs text-[var(--muted)] mt-1";
  meta.textContent = `Context: ${(model.contextWindow / 1e3).toFixed(0)}k tokens`;
  card.appendChild(meta);
  if (!hasEnoughRam) {
    card.classList.add("disabled");
    const warning = document.createElement("div");
    warning.className = "text-xs text-[var(--error)] mt-1";
    warning.textContent = `You do not have enough RAM for this model (${detectedRamGb}GB detected, ${model.minRamGb}GB required).`;
    card.appendChild(warning);
    return card;
  }
  card.addEventListener("click", () => selectLocalModel(model, provider));
  return card;
}
function showModelDownloadProgress(model, provider) {
  const m = els();
  m.modal.classList.remove("hidden");
  m.body.textContent = "";
  const wrapper = document.createElement("div");
  wrapper.className = "provider-key-form";
  const status = document.createElement("div");
  status.className = "text-sm text-[var(--text)]";
  status.textContent = `Configuring ${model.displayName}...`;
  wrapper.appendChild(status);
  const progress = document.createElement("div");
  progress.className = "download-progress mt-4";
  const progressBar = document.createElement("div");
  progressBar.className = "download-progress-bar";
  progressBar.style.width = "0%";
  progress.appendChild(progressBar);
  const progressText = document.createElement("div");
  progressText.className = "text-xs text-[var(--muted)] mt-2";
  progress.appendChild(progressText);
  wrapper.appendChild(progress);
  m.body.appendChild(wrapper);
  const off = onEvent("local-llm.download", (payload) => {
    const p = payload;
    if (p.modelId !== model.id) return;
    if (p.error) {
      status.textContent = p.error;
      status.className = "text-sm text-[var(--error)]";
      off();
      return;
    }
    if (p.complete) {
      status.textContent = `${model.displayName} downloaded successfully!`;
      status.className = "provider-status";
      progressBar.style.width = "100%";
      progressText.textContent = "";
      off();
      fetchModels();
      if (refreshProvidersPage) refreshProvidersPage();
      setTimeout(closeProviderModal, 1500);
      return;
    }
    if (p.progress != null) {
      progressBar.style.width = `${p.progress.toFixed(1)}%`;
      status.textContent = `Downloading ${model.displayName}...`;
    }
    if (p.downloaded != null) {
      const downloadedMb = (p.downloaded / (1024 * 1024)).toFixed(1);
      if (p.total != null) {
        const totalMb = (p.total / (1024 * 1024)).toFixed(1);
        progressText.textContent = `${downloadedMb} MB / ${totalMb} MB`;
      } else {
        progressText.textContent = `${downloadedMb} MB downloaded`;
      }
    }
  });
  pollLocalStatus(model, provider, status, progress, off);
}
function selectLocalModel(model, provider) {
  const m = els();
  m.body.textContent = "";
  const wrapper = document.createElement("div");
  wrapper.className = "provider-key-form";
  const status = document.createElement("div");
  status.className = "text-sm text-[var(--text)]";
  status.textContent = `Configuring ${model.displayName}...`;
  wrapper.appendChild(status);
  const progress = document.createElement("div");
  progress.className = "download-progress mt-4";
  const progressBar = document.createElement("div");
  progressBar.className = "download-progress-bar";
  progressBar.style.width = "0%";
  progress.appendChild(progressBar);
  const progressText = document.createElement("div");
  progressText.className = "text-xs text-[var(--muted)] mt-2";
  progress.appendChild(progressText);
  wrapper.appendChild(progress);
  m.body.appendChild(wrapper);
  const off = onEvent("local-llm.download", (payload) => {
    const p = payload;
    if (p.modelId !== model.id) return;
    if (p.error) {
      status.textContent = p.error;
      status.className = "text-sm text-[var(--error)]";
      off();
      return;
    }
    if (p.complete) {
      status.textContent = `${model.displayName} downloaded successfully!`;
      status.className = "provider-status";
      progressBar.style.width = "100%";
      progressText.textContent = "";
      off();
      fetchModels();
      if (refreshProvidersPage) refreshProvidersPage();
      setTimeout(closeProviderModal, 1500);
      return;
    }
    if (p.progress != null) {
      progressBar.style.width = `${p.progress.toFixed(1)}%`;
      status.textContent = `Downloading ${model.displayName}...`;
    }
    if (p.downloaded != null) {
      const downloadedMb = (p.downloaded / (1024 * 1024)).toFixed(1);
      if (p.total != null) {
        const totalMb = (p.total / (1024 * 1024)).toFixed(1);
        progressText.textContent = `${downloadedMb} MB / ${totalMb} MB`;
      } else {
        progressText.textContent = `${downloadedMb} MB downloaded`;
      }
    }
  });
  sendRpc("providers.local.configure", { modelId: model.id, backend: selectedBackend }).then((res) => {
    var _a;
    if (!(res == null ? void 0 : res.ok)) {
      status.textContent = ((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message) || "Failed to configure model";
      status.className = "text-sm text-[var(--error)]";
      off();
      return;
    }
    pollLocalStatus(model, provider, status, progress, off);
  });
}
function pollLocalStatus(model, _provider, statusEl, progressEl, offEvent) {
  let attempts = 0;
  const maxAttempts = 300;
  let completed = false;
  const timer = setInterval(() => {
    if (completed) {
      clearInterval(timer);
      return;
    }
    attempts++;
    if (attempts > maxAttempts) {
      clearInterval(timer);
      if (offEvent) offEvent();
      statusEl.textContent = "Configuration timed out. Please try again.";
      statusEl.className = "text-sm text-[var(--error)]";
      return;
    }
    sendRpc("providers.local.status", {}).then(
      (res) => {
        if (!(res == null ? void 0 : res.ok)) return;
        const st = res.payload;
        if (st.status === "ready" || st.status === "loaded") {
          completed = true;
          clearInterval(timer);
          if (offEvent) offEvent();
          statusEl.textContent = `${model.displayName} configured successfully!`;
          statusEl.className = "provider-status";
          progressEl.style.display = "none";
          fetchModels();
          if (refreshProvidersPage) refreshProvidersPage();
          setTimeout(closeProviderModal, 1500);
        } else if (st.status === "error") {
          completed = true;
          clearInterval(timer);
          if (offEvent) offEvent();
          statusEl.textContent = st.error || "Configuration failed";
          statusEl.className = "text-sm text-[var(--error)]";
        }
      }
    );
  }, 2e3);
}
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
