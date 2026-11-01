# Koharu for iPad and iPhone

Native SwiftUI/WKWebView app, separate from the desktop CEF executable. Requires iOS/iPadOS 26 or newer. English and Vietnamese UI; the translation target catalog is generated from Koharu's Rust language catalog (currently 42 languages). The base translation prompt and Rawkuma navigation script are shared source files with the desktop app.

## Reading and translation

1. In **Settings**, choose **LM Studio** or **On-device GGUF** and a target language.
2. For LM Studio, enter its LAN root URL, for example `http://192.168.1.20:1234`, refresh the model list and select a loaded model. The initial `localhost` address is a placeholder on iPad; use **Find server** or replace it. Already loaded models sort first. Optional authentication is stored in Keychain. **Find server** checks private IPv4 interfaces with mask `255.255.255.0`, using the selected port. It sends no saved credentials to discovered addresses and follows no redirects. Allow the local-network permission when iOS asks.
3. For local inference, download a catalog model, enter a direct HTTPS GGUF URL, or import a GGUF from Files. Weights stay on the device. The official prebuilt llama.cpp Metal framework runs inference on physical iPads/iPhones. Start with a small model; available RAM limits model size and context. The 0.8B catalog entry is useful for a runtime check, but its translation quality can be poor.
4. Open **Rawkuma**, navigate to a chapter, then **Translate this chapter**. Next/Prev/Home use the site's actual links. Chapter menus use a native selector populated from the site's loaded chapter links; fractional chapter URLs are preserved.
5. Open the chapter in **Library**. **Run** defaults to **Entire project**; **Retranslate** replaces AI translations while preserving text edited by hand. Change language to create a separate copy. **Original chapter** returns to the saved source URL. Pinch to zoom, compare the original, edit text, Undo/Redo, and share a PNG or chapter PDF.

Apple Vision performs Japanese/English OCR locally. The remote provider receives OCR text, target language and translation instructions; the iPad path does not send page images to LM Studio. Rendering is local. Optional **AI cleanup** downloads a pinned LaMa CoreML model (about 40 MB); when disabled, white speech cards cover detected text. The iPad renderer and OCR are distinct from the desktop ML/editor pipeline. PSD export, full layer/brush editing and cross-device synchronization are not implemented.

## Cache and storage

Chapters and preferences live in the app's Application Support directory. Source bytes are immutable SHA-256 named files. **Check source updates** revalidates ETag/Last-Modified and reuses HTTP 304 responses. Derived pages are reused only when configuration and source match. **New copy** preserves a separate chapter. **Clear cache** removes translated PNGs for the selected chapter and preserves original images, OCR, edited text and source URL. **Run** regenerates missing pages. Chapter deletion removes that chapter and its originals after confirmation.

Completed pages are committed individually with atomic manifests and one-step Undo/Redo. Stop discards the uncommitted page. Model downloads use temporary files rather than holding the whole GGUF in memory; model weights are excluded from backup. Keep the app in the foreground during processing and model downloads; background transfer/resume is not implemented.

## Build

Install Xcode with an iOS SDK and `brew install xcodegen`, then from this directory:

```sh
python3 scripts/bootstrap.py

# Simulator: same UI, OCR, renderer and LM Studio; no local llama slice.
xcodebuild -project KoharuPad.xcodeproj -scheme KoharuPadSimulator \
  -configuration Debug -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath DerivedData CODE_SIGNING_ALLOWED=NO build

# Physical device: choose your signing team in Xcode or pass it explicitly.
xcodebuild -project KoharuPad.xcodeproj -scheme KoharuPad \
  -configuration Debug -destination 'generic/platform=iOS' \
  -derivedDataPath DerivedDataDevice -archivePath build/KoharuPad.xcarchive \
  DEVELOPMENT_TEAM=YOUR_TEAM_ID -allowProvisioningUpdates archive
```

Open the generated project in Xcode, select **KoharuPad**, a connected trusted iPad/iPhone, and your signing team. Physical devices require Developer Mode and a provisioning profile containing their UDID. Simulator unit tests use scheme **KoharuPadSimulator** on an installed iPad simulator runtime.

The bootstrap pins the upstream prebuilt llama.cpp archive and checks its SHA-256. Its current archive contains iOS arm64 and macOS slices, with no simulator slice. Vendor weights/frameworks, generated Xcode projects, language JSON, icon PNG, DerivedData, archives and IPAs are ignored. Run bootstrap after updating the authoritative Rust language catalog. Localizations remain source-controlled.

## Verification limits

Five focused contract/storage/render checks passed on the iPad simulator. A signed Kuruby development archive and IPA export build for iOS arm64; installation requires provisioning the connected device. Next and Prev were exercised on a real Rawkuma page inside the simulator app, and the native chapter selector opened. The same production Swift LM Studio client ran on Mac against a Windows server with Gemma 4 26B and returned two valid Vietnamese translations. The production Swift local-inference code loaded Qwen 3.5 0.8B Q4_K_M and produced schema-valid two-segment JSON on the Mac; that does not establish translation quality or iPad M5 performance. The pinned LaMa CoreML model also ran a synthetic input on Mac CPU with finite output of shape `[1,3,512,512]`; this does not validate cleanup quality or pixel alignment in the iPad renderer. Physical-device installation, on-device inference, the remaining navigation controls and a full chapter translation on iPad still require device validation.

See [THIRD_PARTY.md](THIRD_PARTY.md) for runtime and downloadable model sources/licenses.
