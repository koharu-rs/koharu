# Koharu for iPad and iPhone

Native SwiftUI/WKWebView app, separate from the desktop CEF executable. Requires iOS/iPadOS 26 or newer. English and Vietnamese UI; the translation target catalog is generated from Koharu's Rust language catalog (currently 42 languages). The base translation prompt and Rawkuma navigation script are shared source files with the desktop app.

## Reading and translation

1. In **Settings**, choose **LM Studio** or **On-device GGUF** and a target language.
2. For LM Studio, enter its LAN root URL, for example `http://192.168.1.20:1234`, refresh the model list and select a loaded model. The initial `localhost` address is a placeholder on iPad; use **Find server** or replace it. Already loaded models sort first. Optional authentication is stored in Keychain. **Find server** checks private IPv4 interfaces with mask `255.255.255.0`, using the selected port. It sends no saved credentials to discovered addresses and follows no redirects. Allow the local-network permission when iOS asks.
3. For local inference, download a catalog model, enter a direct HTTPS GGUF URL, or import a GGUF from Files. Weights stay on the device. The official prebuilt llama.cpp Metal framework runs inference on physical iPads/iPhones. Start with a small model; available RAM limits model size and context. The 0.8B catalog entry is useful for a runtime check, but its translation quality can be poor.
4. Open **Rawkuma**, navigate to a chapter, then **Translate this chapter**. Next/Prev/Home use the site's actual links. Chapter menus use a native selector populated from the site's chapter links. Empty HTMX placeholders are loaded through their same-origin chapter endpoint; fractional chapter URLs are preserved.
5. Open the chapter in **Library**. **Run** defaults to **Entire project**; **Retranslate** replaces AI translations while preserving text edited by hand. Change language to create a separate copy. **Original chapter** returns to the saved source URL. Pinch to zoom, compare the original, edit text, Undo/Redo, and share a PNG or chapter PDF.

Apple Vision document recognition performs Japanese/English OCR locally, including vertical Japanese speech. OCR has a derived-stage revision: Run rescans older OCR automatically unless the page contains manual edits; in that case, make a new copy to preserve those edits. The remote provider receives OCR text, target language and translation instructions; the iPad path does not send page images to LM Studio. Rendering is local. Optional **AI cleanup** downloads a pinned LaMa CoreML model (about 40 MB); when disabled, white speech cards cover detected text. The iPad renderer and OCR are distinct from the desktop ML/editor pipeline. PSD export, full layer/brush editing and cross-device synchronization are not implemented.

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

Five focused contract/storage/render checks passed on the iPad simulator. Next, Prev, Home and selecting a fractional chapter from the native menu were exercised against Rawkuma in the simulator. Three focused DOM tests cover navigation, exact chapter URLs and loading missing chapter lists.

Version 0.1.1 was signed with Kuruby and installed on a physical iPad Pro 11-inch (M5), iPadOS 27.0. Model refresh and LAN /24 discovery found the Windows LM Studio server. The production GGUF runtime passed a focused physical-device check with Qwen 3.5 0.8B Q4_K_M: two schema-valid translations in 9.192 seconds including loading. Both outputs were the same greeting, so this establishes runtime execution, not useful translation quality or a steady-state benchmark. A vertical Japanese manga fixture produced 8 speech regions and 88 characters in the production OCR path on that device (0.511 seconds).

The same production Swift LM Studio client also ran on Mac against the Windows Gemma 4 26B server and returned two valid Vietnamese translations. The pinned LaMa CoreML model ran a synthetic input on Mac CPU with finite output of shape `[1,3,512,512]`; cleanup quality and pixel alignment need visual validation on representative pages. A full chapter using the revised OCR is still under device validation.

Optional hardware checks are in scheme **KoharuPad**, target **KoharuPadDeviceChecks**. They skip unless you supply the fixtures in the app's Documents directory: `Qwen3.5-0.8B-Q4_K_M.gguf` and/or `OCRFixture.image`. The OCR fixture must have SHA-256 `3dade0142057ce7db8cf170dd14fba9614f755909c7facfb65ac0576dcbb7ec0`. Weights and manga fixtures are not bundled or committed. Run only the relevant check with `-only-testing:KoharuPadDeviceChecks/LocalRuntimeChecks/TEST_NAME` and a connected-device destination.

See [THIRD_PARTY.md](THIRD_PARTY.md) for runtime and downloadable model sources/licenses.
