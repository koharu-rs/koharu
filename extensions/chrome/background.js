let port = null;
let activeRequests = new Map(); // maps transferId -> { port, contentTabId }
let downloadSessions = new Map(); // maps transferId -> { chunks: Map, total: number }
let lastError = null;
let settingsError = null;

// Get and sync the engine settings with the native host.
async function syncSettings(nativePort) {
  const { pipeline } = await chrome.storage.local.get("pipeline");
  nativePort.postMessage(
    pipeline
      ? { action: "UpdateSettings", payload: { pipeline } }
      : { action: "GetSettings" },
  );
}

function getNativePort() {
  if (port) return port;

  console.log("Connecting to Koharu Native Host...");
  lastError = null;
  port = chrome.runtime.connectNative("com.koharu.native_host");
  syncSettings(port);

  port.onMessage.addListener((response) => {
    if (response.status === "settings") {
      const { status, ...catalog } = response;
      settingsError = null;
      chrome.storage.local.set({ catalog });
      return;
    }
    if (response.status === "error" && !response.transferId) {
      console.error("Native host error:", response.message);
      settingsError = response.message;
      return;
    }

    console.log(
      "Received native response status:",
      response.status,
      "for ID:",
      response.transferId,
    );
    const transferId = response.transferId;

    if (!transferId) {
      console.error("Native message missing transferId:", response);
      return;
    }

    const request = activeRequests.get(transferId);
    if (!request) return;

    if (response.status === "success") {
      // Forward final translation layers back to the content script
      chrome.tabs.sendMessage(request.contentTabId, {
        action: "TranslationResult",
        payload: {
          transferId,
          inpaintedImage: response.inpaintedImage,
          texts: response.texts,
        },
      });
      activeRequests.delete(transferId);
    } else if (response.status === "chunk_received") {
      console.log(`Chunk received for ${transferId}: ${response.index}`);
    } else if (response.status === "progress") {
      chrome.tabs.sendMessage(request.contentTabId, {
        action: "Progress",
        payload: {
          transferId,
          stage: response.stage,
          message: response.message,
        },
      });
    } else if (response.status === "download_chunk") {
      let session = downloadSessions.get(transferId);
      if (!session) {
        session = { chunks: new Map(), total: response.total };
        downloadSessions.set(transferId, session);
      }

      session.chunks.set(response.index, response.data);
      console.log(
        `Download chunk received for ${transferId}: ${response.index + 1}/${response.total}`,
      );

      chrome.tabs.sendMessage(request.contentTabId, {
        action: "Progress",
        payload: {
          transferId,
          stage: "transfer",
          message: `Receiving image ${session.chunks.size}/${session.total}`,
        },
      });

      if (session.chunks.size === session.total) {
        let chunksArray = [];
        for (let i = 0; i < session.total; i++) {
          chunksArray.push(session.chunks.get(i));
        }
        let fullData = chunksArray.join("");
        downloadSessions.delete(transferId);

        chrome.tabs.sendMessage(request.contentTabId, {
          action: "TranslationResult",
          payload: {
            transferId,
            inpaintedImage: fullData,
            texts: [],
          },
        });
        activeRequests.delete(transferId);
      }
    } else if (response.status === "error") {
      chrome.tabs.sendMessage(request.contentTabId, {
        action: "Error",
        payload: {
          transferId,
          message: response.message,
        },
      });
      activeRequests.delete(transferId);
    }
  });

  port.onDisconnect.addListener(() => {
    console.warn(
      "Native messaging port disconnected:",
      chrome.runtime.lastError,
    );
    lastError = chrome.runtime.lastError
      ? chrome.runtime.lastError.message
      : null;
    port = null;

    // Error out all active requests
    for (const [transferId, request] of activeRequests.entries()) {
      chrome.tabs.sendMessage(request.contentTabId, {
        action: "Error",
        payload: {
          transferId,
          message: "Connection to local Koharu native service was lost.",
        },
      });
    }
    activeRequests.clear();
    downloadSessions.clear();
  });

  return port;
}

function stopNativeHost() {
  if (!port) return;

  for (const [transferId, request] of activeRequests.entries()) {
    chrome.tabs.sendMessage(request.contentTabId, {
      action: "Error",
      payload: {
        transferId,
        message: "Koharu engine was stopped.",
      },
    });
  }
  activeRequests.clear();
  downloadSessions.clear();

  port.disconnect();
  port = null;
  lastError = null;
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.action === "TranslateImage") {
    const tabId = sender.tab.id;
    const { url } = message.payload;
    const transferId = "transfer_" + Math.random().toString(36).substring(2, 9);

    activeRequests.set(transferId, { contentTabId: tabId });

    fetchAndProcess(transferId, url);
    sendResponse({ transferId });
  } else if (message.action === "GetStatus") {
    sendResponse({
      connected: port !== null,
      active: activeRequests.size,
      lastError,
      settingsError,
    });
  } else if (message.action === "Connect") {
    getNativePort();
    lastError = null;
    sendResponse({ connected: port !== null });
  } else if (message.action === "Disconnect") {
    stopNativeHost();
    sendResponse({ connected: port !== null });
  } else if (message.action === "SaveSettings") {
    const { pipeline } = message.payload;
    chrome.storage.local.set({ pipeline }).then(() => {
      if (port) port.postMessage({ action: "UpdateSettings", payload: { pipeline } });
      sendResponse({ saved: true });
    });
  }
  return true;
});

async function fetchAndProcess(transferId, url) {
  try {
    console.log(`Background worker fetching: ${url}`);
    const response = await fetch(url);
    const blob = await response.blob();
    const base64Data = await new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onloadend = () => resolve(reader.result.split(",")[1]);
      reader.onerror = reject;
      reader.readAsDataURL(blob);
    });
    await uploadAndProcess(transferId, base64Data);
  } catch (err) {
    console.error("Failed to fetch image or upload:", err);
    const request = activeRequests.get(transferId);
    if (request) {
      chrome.tabs.sendMessage(request.contentTabId, {
        action: "Error",
        payload: {
          transferId,
          message: "Failed to download image: " + err.message,
        },
      });
      activeRequests.delete(transferId);
    }
  }
}

async function uploadAndProcess(transferId, base64Data) {
  try {
    const nativePort = getNativePort();

    // Chunk size: 500 KB (base64 characters = 500,000 bytes)
    const CHUNK_SIZE = 500 * 1024;
    const totalChunks = Math.ceil(base64Data.length / CHUNK_SIZE);

    console.log(
      `Uploading ${transferId} to native host in ${totalChunks} chunks...`,
    );

    for (let i = 0; i < totalChunks; i++) {
      const start = i * CHUNK_SIZE;
      const end = Math.min(start + CHUNK_SIZE, base64Data.length);
      const chunk = base64Data.substring(start, end);

      nativePort.postMessage({
        action: "UploadChunk",
        payload: {
          transferId,
          index: i,
          total: totalChunks,
          data: chunk,
        },
      });
    }

    // Now request processing
    console.log(`Requesting processing for ${transferId}...`);
    nativePort.postMessage({
      action: "Process",
      payload: {
        transferId,
        stages: ["detection", "ocr", "translation", "inpainting"],
      },
    });
  } catch (err) {
    console.error("Failed to upload and process image:", err);
    const request = activeRequests.get(transferId);
    if (request) {
      chrome.tabs.sendMessage(request.contentTabId, {
        action: "Error",
        payload: {
          transferId,
          message: "Internal Extension Error: " + err.message,
        },
      });
      activeRequests.delete(transferId);
    }
  }
}
