// Store active overlays keyed by a unique ID assigned to original images
const activeOverlays = new Map();

const trackedSpinners = new Set();

const busyImages = new WeakSet();

const STAGE_ORDER = ["detection", "ocr", "translation", "inpainting"];

const STAGE_LABELS = {
  started: "Starting",
  detection: "Finding text",
  ocr: "Reading text",
  translation: "Translating",
  inpainting: "Cleaning artwork",
  transfer: "Receiving result",
};

const MIN_IMAGE_SIZE = 120;

const HIDE_DELAY = 300;

// CSS insertion
const style = document.createElement("style");
style.textContent = `
  .koharu-translate-btn {
    position: fixed;
    z-index: 2147483646;
    background: oklch(0.645 0.246 16.439);
    color: oklch(0.985 0 0);
    border: none;
    padding: 6px 12px;
    border-radius: 6px;
    cursor: pointer;
    font-weight: 500;
    font-size: 12px;
    line-height: 1;
    box-shadow: 0 4px 6px -1px rgba(0,0,0,0.3);
    transition: background 0.2s;
    font-family: system-ui, sans-serif;
  }
  .koharu-translate-btn:hover {
    background: oklch(0.586 0.253 17.585);
  }
  .koharu-spinner {
    position: fixed;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    color: oklch(0.985 0 0);
    font-weight: bold;
    font-size: 14px;
    z-index: 2147483647;
    pointer-events: none;
    font-family: system-ui, sans-serif;
    text-align: center;
    padding: 8px;
    box-sizing: border-box;
  }
  .koharu-spinner-stage {
    font-weight: 600;
    font-size: 14px;
    text-shadow: 0 1px 3px rgba(0,0,0,0.8);
  }
  .koharu-spinner-detail {
    font-weight: normal;
    font-size: 11px;
    opacity: 0.85;
    text-shadow: 0 1px 3px rgba(0,0,0,0.8);
  }
  .koharu-spinner-steps {
    display: flex;
    gap: 4px;
    margin-top: 2px;
  }
  .koharu-spinner-step {
    width: 18px;
    height: 3px;
    border-radius: 2px;
    background: rgba(255,255,255,0.3);
  }
  .koharu-spinner-step.active {
    background: oklch(0.712 0.194 13.428);
  }
  .koharu-spinner-step.done {
    background: oklch(0.645 0.246 16.439);
  }
`;
document.head.appendChild(style);
let translateButton = null;
let hoveredImage = null;
let hideTimer = null;
let trackingFrame = null;

let siteDisabled = false;

function applyDisabledSites(sites = []) {
  siteDisabled = sites.includes(location.hostname);
  if (siteDisabled) hideTranslateButton();
}

chrome.storage.local
  .get("disabledSites")
  .then(({ disabledSites }) => applyDisabledSites(disabledSites));
chrome.storage.onChanged.addListener((changes) => {
  if (changes.disabledSites) applyDisabledSites(changes.disabledSites.newValue);
});

document.addEventListener("mouseover", (e) => {
  if (siteDisabled) return;

  if (translateButton && translateButton.contains(e.target)) {
    cancelHide();
    return;
  }

  const img = imageFromEvent(e);
  if (img && isTranslatable(img)) {
    showTranslateButton(img);
  } else {
    scheduleHide();
  }
});

function imageFromEvent(e) {
  if (e.target && e.target.tagName === "IMG") {
    return e.target;
  }

  const stack = document.elementsFromPoint(e.clientX, e.clientY);
  return stack.find((el) => el.tagName === "IMG") || null;
}

function isTranslatable(img) {
  if (busyImages.has(img)) return false;

  const rect = img.getBoundingClientRect();
  return rect.width >= MIN_IMAGE_SIZE && rect.height >= MIN_IMAGE_SIZE;
}

function getTranslateButton() {
  if (translateButton) return translateButton;

  translateButton = document.createElement("button");
  translateButton.className = "koharu-translate-btn";
  translateButton.textContent = "Translate";
  translateButton.style.display = "none";

  translateButton.addEventListener("mouseenter", cancelHide);
  translateButton.addEventListener("mouseleave", scheduleHide);
  translateButton.addEventListener("click", () => {
    const img = hoveredImage;
    hideTranslateButton();
    if (img) startTranslation(img);
  });

  document.body.appendChild(translateButton);
  return translateButton;
}

function showTranslateButton(img) {
  cancelHide();
  hoveredImage = img;

  const btn = getTranslateButton();
  btn.style.display = "block";
  positionButton(img);
  ensureTracking();
}

function hideTranslateButton() {
  cancelHide();
  hoveredImage = null;
  if (translateButton) translateButton.style.display = "none";
}

function scheduleHide() {
  cancelHide();
  hideTimer = setTimeout(hideTranslateButton, HIDE_DELAY);
}

function cancelHide() {
  if (hideTimer) {
    clearTimeout(hideTimer);
    hideTimer = null;
  }
}

function positionButton(img) {
  const rect = img.getBoundingClientRect();
  translateButton.style.top = `${rect.top + 10}px`;
  translateButton.style.left = `${rect.left + 10}px`;

  const offscreen =
    rect.bottom <= 0 ||
    rect.top >= window.innerHeight ||
    rect.right <= 0 ||
    rect.left >= window.innerWidth;
  translateButton.style.visibility = offscreen ? "hidden" : "visible";
}

function positionSpinner(state) {
  const rect = state.img.getBoundingClientRect();
  state.spinner.style.top = `${rect.top}px`;
  state.spinner.style.left = `${rect.left}px`;
  state.spinner.style.width = `${rect.width}px`;
  state.spinner.style.height = `${rect.height}px`;
}

function ensureTracking() {
  if (trackingFrame === null) {
    trackingFrame = requestAnimationFrame(trackPositions);
  }
}

function trackPositions() {
  trackingFrame = null;

  if (
    hoveredImage &&
    translateButton &&
    translateButton.style.display !== "none"
  ) {
    if (hoveredImage.isConnected) {
      positionButton(hoveredImage);
    } else {
      hideTranslateButton();
    }
  }

  for (const state of trackedSpinners) {
    positionSpinner(state);
  }

  if (hoveredImage || trackedSpinners.size > 0) {
    trackingFrame = requestAnimationFrame(trackPositions);
  }
}

async function startTranslation(img) {
  const id = "img_" + Math.random().toString(36).substring(2, 9);
  img.dataset.koharuId = id;
  busyImages.add(img);

  // Show spinner
  const spinner = document.createElement("div");
  spinner.className = "koharu-spinner";

  const stageEl = document.createElement("div");
  stageEl.className = "koharu-spinner-stage";
  stageEl.textContent = "Starting";

  const detailEl = document.createElement("div");
  detailEl.className = "koharu-spinner-detail";
  detailEl.textContent = "Uploading image to Koharu";

  const stepsEl = document.createElement("div");
  stepsEl.className = "koharu-spinner-steps";
  const stepEls = {};
  for (const stage of STAGE_ORDER) {
    const step = document.createElement("div");
    step.className = "koharu-spinner-step";
    step.title = STAGE_LABELS[stage];
    stepsEl.appendChild(step);
    stepEls[stage] = step;
  }

  spinner.append(stageEl, detailEl, stepsEl);
  document.body.appendChild(spinner);

  const state = { spinner, img, stageEl, detailEl, stepEls };
  activeOverlays.set(id, state);
  trackedSpinners.add(state);
  positionSpinner(state);
  ensureTracking();

  chrome.runtime.sendMessage(
    {
      action: "TranslateImage",
      payload: { url: img.src },
    },
    (resp) => {
      if (resp && resp.transferId) {
        console.log("Translation started, transferId:", resp.transferId);
        const state = activeOverlays.get(id);
        state.transferId = resp.transferId;
        activeOverlays.set(resp.transferId, { id, ...state });
      }
    },
  );
}

function finishTranslation(transferId) {
  const state = activeOverlays.get(transferId);
  if (!state) return null;

  state.spinner.remove();
  busyImages.delete(state.img);

  for (const tracked of trackedSpinners) {
    if (tracked.spinner === state.spinner) {
      trackedSpinners.delete(tracked);
      break;
    }
  }

  activeOverlays.delete(transferId);
  if (state.id) activeOverlays.delete(state.id);

  return state;
}

chrome.runtime.onMessage.addListener((message) => {
  if (message.action === "Progress") {
    const { transferId, stage, message: detail } = message.payload;
    const state = activeOverlays.get(transferId);
    if (!state || !state.stageEl) return;

    state.stageEl.textContent = STAGE_LABELS[stage] || stage;
    state.detailEl.textContent = detail || "";

    const current = STAGE_ORDER.indexOf(stage);
    if (current !== -1) {
      STAGE_ORDER.forEach((name, i) => {
        const step = state.stepEls[name];
        if (!step) return;
        step.className =
          "koharu-spinner-step" +
          (i < current ? " done" : i === current ? " active" : "");
      });
    } else if (stage === "transfer") {
      STAGE_ORDER.forEach((name) => {
        const step = state.stepEls[name];
        if (step) step.className = "koharu-spinner-step done";
      });
    }
  } else if (message.action === "TranslationResult") {
    const { transferId, inpaintedImage, texts } = message.payload;
    const state = finishTranslation(transferId);
    if (!state) return;

    renderOverlay(state.img, inpaintedImage, texts);
  } else if (message.action === "Error") {
    const { transferId, message: errMsg } = message.payload;
    const state = finishTranslation(transferId);
    if (!state) return;

    alert("Translation failed: " + errMsg);
  }
});

function renderOverlay(img, inpaintedBase64, texts) {
  if (inpaintedBase64) {
    img.src = "data:image/png;base64," + inpaintedBase64;
    img.removeAttribute("srcset");
  }
}
