# Koharu Native Messaging Host & Web Extension

This crate implements the Native Messaging host for the Koharu Chrome/Web Extension, allowing standard web browsers to run image detection, OCR, translation, and inpainting locally via the Koharu ML pipeline.

## Overview

The integration consists of two main components:
1. **Chrome Extension** ([`extensions/chrome`](../../extensions/chrome)): Captures page manga/comic images, chunks image data, sends processing requests, and overlays rendered text back onto the webpage.
2. **Native Messaging Host** ([`crates/koharu-native-messaging`](./)): A stdio-based native host binary (`koharu-native-messaging.exe`) that receives requests over standard input/output, executes the local ML pipeline, and streams status updates and processed canvas payloads back to the extension.

---

## Setup Instructions

### 1. Build the Native Host Binary

Build the native host binary using Cargo:

```powershell
cargo build --package koharu-native-messaging
```

The compiled binary will be placed at:
- Debug: `d:\koharu\target\debug\koharu-native-messaging.exe`
- Release: `d:\koharu\target\release\koharu-native-messaging.exe`

---

### 2. Load the Web Extension

1. Open Chrome, Edge, or any Chromium-based browser and navigate to `chrome://extensions`.
2. Enable **Developer mode** (toggle in the top-right corner).
3. Click **Load unpacked** and select the extension directory:
   [`extensions/chrome`](../../extensions/chrome)
4. Copy the generated **Extension ID** displayed on the extension card (e.g. `fonodfpfngfkfcdhddapkcgficghpoog`).

---

### 3. Configure Native Messaging Host Manifest

Inspect [`com.koharu.native_host.json`](./com.koharu.native_host.json):

```json
{
  "name": "com.koharu.native_host",
  "description": "Koharu Chrome Extension Native Messaging Host",
  "path": "..\\..\\target\\debug\\koharu-native-messaging.exe",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://fonodfpfngfkfcdhddapkcgficghpoog/"
  ]
}
```

- **`allowed_origins`**: Update `chrome-extension://<EXTENSION_ID>/` with the Extension ID obtained in Step 2.
- **`path`**: Set to relative path by default, and automatically updated to the full absolute path of the built binary when running `setup.bat`.

---

### 4. Register the Host Registry Entry

Run [`setup.bat`](./setup.bat) to register the native messaging host on any Windows PC:

```cmd
.\setup.bat
```

`setup.bat` is fully portable and automatically:
- Resolves the repository root and script location using `%~dp0` (works regardless of installation drive or folder path).
- Detects if a `release` or `debug` binary exists.
- Dynamically updates the binary path in `com.koharu.native_host.json`.
- Adds registry keys for both **Google Chrome** (`HKCU\Software\Google\Chrome\...`) and **Microsoft Edge** (`HKCU\Software\Microsoft\Edge\...`).

---

## Protocol Specification

Communication between the Chrome extension and host uses standard Native Messaging framing: a 32-bit little-endian integer prefix specifying the payload length in bytes, followed by the JSON payload.

### Extension Requests
- **`UploadChunk`**: Transfers image data chunks encoded in Base64.
- **`Process`**: Triggers execution for specified pipeline stages (`Detection`, `Ocr`, `Translation`, `Inpainting`).

### Extension Responses
- **`ChunkReceived`**: Acknowledges chunk receipt.
- **`Progress`**: Streams active model loading and execution stage status.
- **`DownloadChunk`**: Streams processed canvas result chunks back to extension.
- **`Success`**: Signifies pipeline completion with text overlay coordinates.
- **`Error`**: Reports execution or payload parsing errors.

---

## Logging & Diagnostics

The host logs diagnostic output to:
`%USERPROFILE%\.koharu\native-host.log`

Check this log file if the extension fails to connect or if pipeline steps encounter runtime errors.
