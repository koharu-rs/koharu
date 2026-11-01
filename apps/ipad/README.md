# Koharu for iPad and iPhone

Native SwiftUI/WKWebView app, separate from the desktop CEF executable. Requires iOS/iPadOS 26 or newer. English and Vietnamese UI; the translation target catalog is generated from Koharu's Rust language catalog (currently 42 languages). The base translation prompt and Rawkuma navigation script are shared source files with the desktop app.

## Reading and translation

1. In **Settings**, choose **LM Studio** or **On-device GGUF** and a target language.
2. For LM Studio, enter its LAN root URL, for example `http://192.168.1.20:1234`, refresh the model list and select a loaded model. The initial `localhost` address is a placeholder on iPad; use **Find server** or replace it. Already loaded models sort first. Optional authentication is stored in Keychain. **Find server** checks private IPv4 interfaces with mask `255.255.255.0`, using the selected port. It sends no saved credentials to discovered addresses and follows no redirects. Allow the local-network permission when iOS asks.
3. For local inference, download a catalog model, enter a direct HTTPS GGUF URL, or import a GGUF from Files. Weights stay on the device. The official prebuilt llama.cpp Metal framework runs inference on physical iPads/iPhones. Start with a small model; available RAM limits model size and context. The 0.8B catalog entry is useful for a runtime check, but its translation quality can be poor.
4. Open **Rawkuma**, navigate to a chapter, then **Translate this chapter**. Next/Prev/Home use the site's actual links. Chapter menus use a native selector populated from the site's chapter links. Empty HTMX placeholders are loaded through their same-origin chapter endpoint; fractional chapter URLs are preserved.
5. Open the chapter in **Library**. **Run** defaults to **Entire project**; **Retranslate** replaces AI translations while preserving text edited by hand. Change language to create a separate copy. **Original chapter** returns to the saved source URL. Swipe left for the next page and right for the previous page at fit zoom; while zoomed in, drag to pan without turning pages. Back navigation gestures are reserved for page reading while the reader is visible; use the Back button to return to the library. Other screens retain their navigation gestures. Reading position is saved through the same page selection as the buttons and slider. Pinch to zoom, compare the original, edit text, Undo/Redo, and share a PNG or chapter PDF.

## OCR and inpainting models

In **Settings → OCR model**, select **Manga OCR**, **Baberu OCR**, **Hayai OCR v2.1**, or **PaddleOCR VL 1.6**. Apple Vision remains a built-in option and detects text regions for the other readers. The selected model reads each detected crop locally; selecting another reader does not silently fall back to Vision. The translation target is independent of the OCR model's supported source languages.

In **Settings → Image cleanup**, enable AI inpainting and select **LaMa Manga**, **AOT GAN**, **FLUX.2 Klein**, or **RORem mixed**. Processing uses 512×512 context tiles at the source resolution, instead of stretching the whole page into a square. Only masked pixels are composited into the original page. This avoids blurring/resizing untouched artwork. Without cleanup, translated text uses white speech cards.

| Model | Download (decimal) | Runtime / variant | Estimated free RAM needed |
|---|---:|---|---:|
| Manga OCR | 461 MB | ONNX encoder/decoder, Japanese | 1,100 MB |
| Baberu OCR | 243 MB | Publisher FP16 encoder + INT8 decoder, v1.2 | 800 MB |
| Hayai OCR v2.1 | 145 MB | Publisher dynamic INT8 ONNX | 800 MB |
| PaddleOCR VL 1.6 | 1.82 GB | Official GGUF + image projector, llama.cpp Metal | 2,800 MB |
| LaMa Manga | 207 MB | Manga ONNX variant, CPU | 900 MB |
| AOT GAN | 23 MB | ONNX with Core ML acceleration | 650 MB |
| FLUX.2 Klein 4B | 5.35 GB | Q4_K_M + Qwen3 text encoder + FLUX.2 small VAE, Metal | 6,500 MB |
| RORem mixed | 3.83 GB | Q4_K UNet + SDXL VAE/CLIP encoders, Metal | 6,200 MB |

ONNX OCR uses CPU; PaddleOCR and diffusion use Metal. AOT's supported operations use Core ML, with remaining operations handled by ONNX Runtime. OCR, translation and cleanup release their model allocations between stages; the app does not hold both local translation and diffusion weights on the GPU. The estimated RAM figures are admission heuristics, not a guarantee: iPadOS can reclaim/limit memory at runtime. Settings warns when current available memory is below the requirement; Run checks again and reports the selected model and available/required memory. It never substitutes another model. FLUX/RORem are much slower than LaMa and may not fit smaller iPads.

Each selection has its own **Download selected model**, ready status, source link and delete action. Downloads include every companion encoder/VAE/projector needed by that model. Commit-pinned URLs, byte counts and SHA-256 checksums are in `Resources/vision-models.json`; all files must verify before the model directory is published. Stopping a download discards its staging files. Weights are separate downloads, never bundled in the IPA. Use the foreground during downloads; background resume is not implemented.

OCR signatures include the selected model and checkpoint hashes. Run rescans when they change, unless that page has manual edits; make a new copy in that case to preserve edits. Source text and translations remain separate. LM Studio receives OCR text, target language and translation instructions, without page images. Rendering is local. The iPad pipeline is separate from the desktop ML/editor pipeline; PSD export, full layer/brush editing and cross-device synchronization are not implemented.

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
  -derivedDataPath DerivedData -clonedSourcePackagesDirPath Vendor/Packages \
  CODE_SIGNING_ALLOWED=NO build

# Physical device: choose your signing team in Xcode or pass it explicitly.
xcodebuild -project KoharuPad.xcodeproj -scheme KoharuPad \
  -configuration Debug -destination 'generic/platform=iOS' \
  -derivedDataPath DerivedDataDevice -clonedSourcePackagesDirPath Vendor/Packages \
  -archivePath build/KoharuPad.xcarchive \
  DEVELOPMENT_TEAM=YOUR_TEAM_ID -allowProvisioningUpdates archive
```

Open the generated project in Xcode, select **KoharuPad**, a connected trusted iPad/iPhone, and your signing team. Physical devices require Developer Mode and a provisioning profile containing their UDID. Simulator unit tests use scheme **KoharuPadSimulator** on an installed iPad simulator runtime.

The bootstrap pins the upstream prebuilt llama.cpp b12024 and stable-diffusion-native v0.3.0 archives and verifies their SHA-256. ONNX Runtime 1.24.2 is resolved through the official Microsoft Swift package (its binary has a package checksum). The llama archive contains iOS arm64 and macOS slices, with no simulator slice; diffusion and ONNX include simulator slices. Vendor weights/frameworks, generated Xcode projects, language JSON, icon PNG, DerivedData, archives and IPAs are ignored. Run bootstrap after updating the authoritative Rust language catalog. Localizations remain source-controlled.

## Verification limits

Version **0.2.0 (build 5)** was built in Debug and checked on the physical iPad Pro 11-inch M5, iPadOS 27.0. All eight downloaded models ran, with no RAM skips. Focused native comparisons used identical inputs against the pinned Python ONNX implementations (and the same pinned GGUF/runtime on Mac for Paddle):

| Focused check | Measured cold/first-call result |
|---|---|
| Manga / Baberu / Hayai OCR | Exact expected Japanese string, 0.395 / 0.459 / 0.832 s including model loading |
| PaddleOCR VL 1.6 | Same Japanese string as Mac; 1.575 s for the focused test including loading |
| LaMa Manga | 4.093 s; masked RGB MAE 0 against Python |
| AOT GAN with Core ML | 4.064 s; masked RGB MAE 0.000247 against Python, versus 121.518 s on the CPU path |
| FLUX.2 Klein / RORem mixed | Actual 512×512 cleanup completed on M5; 59.365 / 63.893 s for each focused test including loading |

The GAN regression embeds the 512-pixel reference crop in a 720×1200 portrait page, verifies output size and checks every unmasked RGB value unchanged. The two diffusion outputs were visually inspected on the same Japanese title crop: the text was removed, though Klein added a faint gray texture on the white mask. These small fixtures establish runtime/port correctness, not manga-wide quality, general OCR accuracy or steady-state benchmarks. Whole-chapter runs with every combination of the new models have not been performed. Core ML can log default dynamic-shape compilation warnings on iPadOS 27; the actual 512-pixel AOT prediction passes the reference comparison.

Optional `VisionModelChecks` require the pinned model files under Application Support/Koharu/models/vision and `Documents/VisionChecks` fixtures. `title.png` is the existing repository fixture `crates/koharu-ml/benches/fixtures/ocr/title.png`; Generate the matching Python goldens with `scripts/vision_reference.py --models /path/to/vision --out /path/to/VisionChecks`, then copy that output directory into the app's Documents directory. The recorded reference environment used ONNX Runtime 1.31.1, Pillow 13.0.0, NumPy 2.5.4, tokenizers and jaconv 0.4.0; iPad uses ONNX Runtime 1.24.2. The helper verifies all five ONNX model sets against the manifest before generation and imports the two OCR publishers' commit-pinned readers. It reproduced the recorded text JSON and both PNG files exactly. Missing assets and inadequate RAM are reported as skips, never successful inference. Run individual checks with `-only-testing:KoharuPadDeviceChecks/VisionModelChecks/TEST_NAME`.

Version 0.1.3 reserves both edge and full-content back gestures while the reader is visible, then restores their previous enabled states on exit. A focused physical-device test covers disabling and restoration. Actual M5 UI checks: long right swipe 7 → 6, left swipe 6 → 7 without leaving the reader, and Back → Library.

Version 0.1.2 adds horizontal page swipes. On the physical M5, left swipe advanced page 5 to 6 and right swipe returned page 6 to 5. Swipe navigation uses the existing saved page index; zoomed images retain horizontal drags for panning.

Five focused contract/storage/render checks passed on the iPad simulator. Next, Prev, Home and selecting a fractional chapter from the native menu were exercised against Rawkuma in the simulator. Three focused DOM tests cover navigation, exact chapter URLs and loading missing chapter lists.

Version 0.1.1 was signed with Kuruby and installed on a physical iPad Pro 11-inch (M5), iPadOS 27.0. Model refresh and LAN /24 discovery found the Windows LM Studio server. The production GGUF runtime passed a focused physical-device check with Qwen 3.5 0.8B Q4_K_M: two schema-valid translations in 9.192 seconds including loading. Both outputs were the same greeting, so this establishes runtime execution, not useful translation quality or a steady-state benchmark. A vertical Japanese manga fixture produced 8 speech regions and 88 characters in the production OCR path on that device (0.511 seconds).

The same production Swift LM Studio client also ran on Mac against the Windows Gemma 4 26B server and returned two valid Vietnamese translations. The former 40 MB LaMa CoreML conversion ran a synthetic input on Mac CPU; version 0.2.0 replaces it with the selectable models above. On the physical M5, one 30-page Rawkuma chapter then completed with revised OCR (193 regions), Windows LM Studio translation, the former LaMa cleanup and 30 saved rendered pages. The first page was visually checked; this is not a full editorial review of every translation or text box.

Optional hardware checks are in scheme **KoharuPad**, target **KoharuPadDeviceChecks**. They skip unless you supply the fixtures in the app's Documents directory: `Qwen3.5-0.8B-Q4_K_M.gguf` and/or `OCRFixture.image`. The OCR fixture must have SHA-256 `3dade0142057ce7db8cf170dd14fba9614f755909c7facfb65ac0576dcbb7ec0`. Weights and manga fixtures are not bundled or committed. Run only the relevant check with `-only-testing:KoharuPadDeviceChecks/LocalRuntimeChecks/TEST_NAME` and a connected-device destination.

See [THIRD_PARTY.md](THIRD_PARTY.md) for runtime and downloadable model sources/licenses.
