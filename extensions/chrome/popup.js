const dot = document.getElementById("dot");
const statusLabel = document.getElementById("status-label");
const statusNote = document.getElementById("status-note");
const powerButton = document.getElementById("power");

let engineRunning = false;
let settingsError = null;

function refresh() {
  chrome.runtime.sendMessage({ action: "GetStatus" }, (status) => {
    if (chrome.runtime.lastError || !status) {
      render({ connected: false, active: 0, lastError: "Background worker unavailable." });
      return;
    }
    render(status);
  });
}

function render(status) {
  const { connected, active, lastError } = status;

  if (settingsError !== (status.settingsError ?? null)) {
    settingsError = status.settingsError ?? null;
    renderSettingsNote();
  }
  engineRunning = connected;
  dot.className = "dot";
  statusNote.className = "status-note";
  powerButton.className = connected ? "stop" : "";
  powerButton.disabled = false;

  if (connected) {
    dot.classList.add("online");
    statusLabel.textContent = "Engine running";
    statusNote.textContent = active > 0
      ? `${active} ${active === 1 ? "page" : "pages"} translating`
      : "Models loaded and idle";
    powerButton.textContent = active > 0 ? "Stop engine (cancels work)" : "Stop engine";
  } else if (lastError) {
    dot.classList.add("error");
    statusNote.classList.add("error");
    statusLabel.textContent = "Disconnected";
    statusNote.textContent = lastError;
    powerButton.textContent = "Retry";
  } else {
    statusLabel.textContent = "Engine idle";
    statusNote.textContent = "Starts on the first translation.";
    powerButton.textContent = "Start engine";
  }
}

powerButton.addEventListener("click", () => {
  const action = engineRunning ? "Disconnect" : "Connect";
  powerButton.disabled = true;
  statusNote.className = "status-note";

  if (engineRunning) {
    powerButton.textContent = "Stopping…";
    statusLabel.textContent = "Unloading models…";
    statusNote.textContent = "Frees the GPU memory the host is holding.";
  } else {
    powerButton.textContent = "Starting…";
    statusLabel.textContent = "Loading models…";
    statusNote.textContent = "This takes a few seconds on first start.";
  }

  chrome.runtime.sendMessage({ action }, () => {
    if (chrome.runtime.lastError) {
      console.warn(`${action} failed:`, chrome.runtime.lastError);
    }
    refresh();
  });
});

refresh();
setInterval(refresh, 1000);

const siteCard = document.getElementById("site");
const siteIcon = document.getElementById("site-icon");
const siteSwitch = document.getElementById("site-switch");
const siteState = document.getElementById("site-state");

let siteHost = null;
let disabledSites = [];

function renderSite() {
  const enabled = !disabledSites.includes(siteHost);
  siteSwitch.checked = enabled;
  siteState.textContent = enabled ? "on" : "off";
}

siteIcon.addEventListener("error", () => {
  siteIcon.hidden = true;
});

siteSwitch.addEventListener("change", () => {
  disabledSites = siteSwitch.checked
    ? disabledSites.filter((site) => site !== siteHost)
    : [...disabledSites, siteHost];
  renderSite();
  chrome.storage.local.set({ disabledSites });
});

Promise.all([
  chrome.tabs.query({ active: true, currentWindow: true }),
  chrome.storage.local.get("disabledSites"),
]).then(([[tab], stored]) => {
  const url = tab?.url ? new URL(tab.url) : null;
  if (!url || (url.protocol !== "http:" && url.protocol !== "https:")) return;

  siteHost = url.hostname;
  disabledSites = stored.disabledSites ?? [];
  document.getElementById("site-name").textContent = siteHost.replace(/^www\./, "");
  if (tab.favIconUrl) siteIcon.src = tab.favIconUrl;
  else siteIcon.hidden = true;
  siteCard.hidden = false;
  renderSite();
});

const STAGE_MODELS = {
  detection: [["koharu-layout-rfdetr-seg-2xl", "Koharu Layout RF-DETR Seg 2XL"]],
  ocr: [
    ["paddleocr-vl-1.6", "PaddleOCR-VL 1.6"],
    ["manga-ocr", "Manga OCR"],
    ["baberu-ocr", "Baberu OCR"],
    ["hayai-ocr", "Hayai OCR"],
  ],
  inpainting: [
    ["lama", "LaMa"],
    ["aot-inpainting", "AOT Inpainting"],
    ["flux2-klein", "FLUX.2 Klein"],
    ["rorem-mixed", "RORem Mixed"],
  ],
};

const settingsToggle = document.getElementById("settings-toggle");
const settingsBody = document.getElementById("settings-body");

function setSettingsCollapsed(collapsed) {
  settingsToggle.setAttribute("aria-expanded", String(!collapsed));
  settingsBody.hidden = collapsed;
}

settingsToggle.addEventListener("click", () => {
  const collapsed = !settingsBody.hidden;
  setSettingsCollapsed(collapsed);
  chrome.storage.local.set({ settingsCollapsed: collapsed });
});

const fields = document.getElementById("settings-fields");
const settingsNote = document.getElementById("settings-note");
const quantizationField = document.getElementById("quantization-field");
const selects = Object.fromEntries(
  ["detection", "ocr", "inpainting", "provider", "model", "quantization", "language"].map(
    (id) => [id, document.getElementById(id)],
  ),
);

let catalog = null;
let pipeline = null;

function fill(select, options, value) {
  select.replaceChildren(
    ...options.map(([id, name]) => new Option(name, id, false, id === value)),
  );
}

function renderSettingsNote() {
  settingsNote.className = settingsError ? "status-note error" : "status-note";
  if (settingsError) {
    settingsNote.textContent = settingsError;
  } else if (!catalog) {
    settingsNote.textContent = "Start the engine once to load the available models.";
  } else {
    settingsNote.textContent = pipeline
      ? "Saved for this extension. Applies to the next translation."
      : "Using Koharu's defaults.";
  }
}

function currentPipeline() {
  return pipeline ?? catalog.pipeline;
}

function renderSettings() {
  fields.hidden = !catalog;
  renderSettingsNote();
  if (!catalog) return;

  const { translationModels, providers, languages } = catalog;
  const current = currentPipeline();
  for (const stage of Object.keys(STAGE_MODELS)) {
    fill(selects[stage], STAGE_MODELS[stage], current[stage].model);
  }

  const selected = current.translation.model;
  const models = translationModels.some(
    (model) => model.provider === selected.provider && model.model === selected.model,
  )
    ? translationModels
    : [
      {
        ...selected,
        name: selected.model ?? providerName(selected.provider),
        quantizations: [],
      },
      ...translationModels,
    ];
  const offered = new Set(models.map((model) => model.provider));
  fill(
    selects.provider,
    providers.filter((provider) => offered.has(provider.id)).map(({ id, name }) => [id, name]),
    selected.provider,
  );

  const providerModels = models.filter((model) => model.provider === selected.provider);
  fill(
    selects.model,
    providerModels.map((model) => [model.model ?? "", model.name]),
    selected.model ?? "",
  );

  const { quantizations } = providerModels.find((model) => model.model === selected.model);
  quantizationField.hidden = quantizations.length === 0;
  fill(
    selects.quantization,
    quantizations.map(({ id, name }) => [id, name]),
    selected.quantization,
  );

  fill(
    selects.language,
    languages.map(({ id, name }) => [id, name]),
    current.translation.target_language,
  );
}

function providerName(id) {
  return catalog.providers.find((provider) => provider.id === id)?.name ?? id;
}

function modelSelection(model) {
  return {
    provider: model.provider,
    model: model.model,
    quantization: model.quantizations[0]?.id ?? null,
    vision: model.vision,
    reasoning: model.reasoning,
  };
}

function savePipeline(next) {
  pipeline = next;
  renderSettings();
  chrome.runtime.sendMessage({ action: "SaveSettings", payload: { pipeline } });
}

function saveTranslation(translation) {
  const current = currentPipeline();
  savePipeline({ ...current, translation: { ...current.translation, ...translation } });
}

for (const stage of Object.keys(STAGE_MODELS)) {
  selects[stage].addEventListener("change", () => {
    savePipeline({ ...currentPipeline(), [stage]: { model: selects[stage].value } });
  });
}

selects.provider.addEventListener("change", () => {
  const model = catalog.translationModels.find(
    (candidate) => candidate.provider === selects.provider.value,
  );
  saveTranslation({ model: modelSelection(model) });
});

selects.model.addEventListener("change", () => {
  const model = catalog.translationModels.find(
    (candidate) =>
      candidate.provider === selects.provider.value &&
      (candidate.model ?? "") === selects.model.value,
  );
  saveTranslation({ model: modelSelection(model) });
});

selects.quantization.addEventListener("change", () => {
  saveTranslation({
    model: { ...currentPipeline().translation.model, quantization: selects.quantization.value },
  });
});

selects.language.addEventListener("change", () => {
  saveTranslation({ target_language: selects.language.value });
});

chrome.storage.local.get(["catalog", "pipeline", "settingsCollapsed"]).then((stored) => {
  setSettingsCollapsed(stored.settingsCollapsed ?? true);
  catalog = stored.catalog ?? null;
  pipeline = stored.pipeline ?? null;
  renderSettings();
});

chrome.storage.onChanged.addListener((changes) => {
  if (!changes.catalog) return;
  catalog = changes.catalog.newValue ?? null;
  renderSettings();
});
