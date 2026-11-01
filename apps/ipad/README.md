# Koharu for iPad and iPhone

Native SwiftUI/WKWebView app, separate from the desktop CEF executable. Requires iOS/iPadOS 26 or newer. The interface supports English (US/UK), French, Arabic, Spanish, Portuguese, German, Korean, Japanese, Traditional Chinese, Simplified Chinese and Vietnamese. **Settings → Interface → App language** changes the interface immediately and saves it separately from the chapter's translation target. **System default** follows the device's supported language preference. Arabic uses right-to-left controls/layout; URL and token fields stay left-to-right. Native system permission/share dialogs follow iPadOS's app-language settings. The translation target catalog is generated from Koharu's Rust language catalog (43 targets, including separate US and UK English). The base translation prompt and Rawkuma navigation script are shared source files with the desktop app.

## Reading and translation

1. In **Settings**, choose a translation provider and a target language: **LM Studio**, **On-device GGUF**, or one of the hosted APIs below.
2. For LM Studio, enter its LAN root URL, for example `http://192.168.1.20:1234`, refresh the model list and select a loaded model. The initial `localhost` address is a placeholder on iPad; use **Find server** or replace it. Already loaded models sort first. Optional authentication is stored in Keychain. **Find server** checks private IPv4 interfaces with mask `255.255.255.0`, using the selected port. It sends no saved credentials to discovered addresses and follows no redirects. Allow the local-network permission when iOS asks.
3. For local inference, download a catalog model, enter a direct HTTPS GGUF URL, or import a GGUF from Files. Weights stay on the device. The official prebuilt llama.cpp Metal framework runs inference on physical iPads/iPhones. Start with a small model; available RAM limits model size and context. The 0.8B catalog entry is useful for a runtime check, but its translation quality can be poor.
4. Open **Rawkuma**, navigate to a chapter, then **Translate this chapter**. Next/Prev/Home use the site's actual links. Chapter menus use a native selector populated from the site's chapter links. Empty HTMX placeholders are loaded through their same-origin chapter endpoint; fractional chapter URLs are preserved.
5. Open the chapter in **Library**. **Run** defaults to **Entire project**; **Retranslate** replaces AI translations while preserving text edited by hand. Change language to create a separate copy. **Original chapter** returns to the saved source URL. Swipe left for the next page and right for the previous page at fit zoom; while zoomed in, drag to pan without turning pages. Back navigation gestures are reserved for page reading while the reader is visible; use the Back button to return to the library. Other screens retain their navigation gestures. Reading position is saved through the same page selection as the buttons and slider. Pinch to zoom, compare the original, edit text, Undo/Redo, and share a PNG or chapter PDF.

## OCR and inpainting models

In **Settings → Translation → Segments per request**, enter **1–100**; the default is **100**. OCR finishes each page before its segments are sent in sequential batches. A 26-segment page now uses one request with the default, while a 103-segment page uses 100 + 3. Pages are never combined. This execution setting is saved on the device separately from project processing fingerprints, so changing it does not invalidate completed pages. Output budgets grow with the segment count up to 8,192 tokens; LM Studio allows 180–600 seconds per request, and hosted APIs allow up to 660 seconds total. Model/provider context, output and request-size limits still apply. Reduce the batch size or increase local context if a large request cannot complete; the app never silently drops segments or publishes partial JSON.

**AI inpainting defaults to off.** Enable it explicitly to select/download a cleanup model; switching it off uses bordered white cards and hides cleanup-model controls. Existing explicit settings are retained when updating.

After importing a chapter from Rawkuma, the reader opens that exact saved chapter as soon as its original pages are available, before translation finishes. It replaces the previous reader route; an import failure does not redirect to another chapter. Back returns to the library list.

On iPhone/compact widths, **0.3.5** uses an inline Rawkuma title, compact address bar and a single-row chapter action with a page-count badge. Back/Forward use icon buttons; **Check source updates** and **New copy** are in the **⋯** menu. The address bar's trailing action opens an edited URL or reloads the current URL; the keyboard's Go key also opens the typed address. Regular-width iPad keeps its full toolbar. Debug build and simulator UI checks covered Vietnamese, a long German action label, opening a chapter URL and changing the source-update option. Full translation was not rerun for this layout change. The signed universal app 0.3.5 (build 12) was installed/launched on an iPhone 15 Pro Max with iOS 26.6; device metadata confirms the version. Physical screen mirroring is unavailable on this Mac for devices below iOS 27, so visual checks used the iPhone simulator.

In **Settings → OCR model**, select **Manga OCR**, **Baberu OCR**, **Hayai OCR v2.1**, or **PaddleOCR VL 1.6**. Apple Vision remains a built-in option and detects text regions for the other readers. The selected model reads each detected crop locally; selecting another reader does not silently fall back to Vision. The translation target is independent of the OCR model's supported source languages.

In **Settings → Image cleanup**, enable AI inpainting and select **LaMa Manga**, **AOT GAN**, **FLUX.2 Klein**, or **RORem mixed**. Processing uses 512×512 context tiles at the source resolution, instead of stretching the whole page into a square. Only masked pixels are composited into the original page. This avoids blurring/resizing untouched artwork. With **AI inpainting off**, the app covers each translated OCR region with an opaque white rectangle with a 1-pixel gray border, including the cleanup mask's 3-pixel margin, then draws the translation over it. All white backgrounds are painted first so overlapping regions cannot erase translated text. This mode needs no cleanup model and also covers any artwork inside those rectangles; it does not reconstruct the background. In **0.3.3**, use **Run** to rebuild existing page images with this behavior.

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

Translated text wraps at word boundaries. The font fits both the bubble height and the widest word, so narrow Japanese OCR columns do not force Vietnamese/English/Arabic words into individual characters. Automatic hyphenation is disabled and Korean uses word-priority wrapping; unspaced Japanese/Chinese retains native line breaking. The bubble geometry and saved translation text are preserved.

After updating to **0.3.2**, use **Run** to refresh existing page layouts. With unchanged processing settings and complete cached translations, the app redraws the stored text without calling an AI model again. Enabled cleanup may run again when rebuilding the page image. **Retranslate** explicitly requests new AI translations.

## Cache and storage

Chapters and preferences live in the app's Application Support directory. Source bytes are immutable SHA-256 named files. **Check source updates** revalidates ETag/Last-Modified and reuses HTTP 304 responses. Derived pages are reused only when configuration and source match. **New copy** preserves a separate chapter. **Clear cache** removes translated PNGs for the selected chapter and preserves original images, OCR, edited text and source URL. **Run** regenerates missing pages. Chapter deletion removes that chapter and its originals after confirmation.

Completed pages are committed individually with atomic manifests and one-step Undo/Redo. Stop discards the uncommitted page. Entire-project processing is sequential: OCR → translation → cleanup/render → save for each page, then the next page. LM Studio therefore receives no new page while FLUX/RORem is cleaning the current one. Version 0.3.1 shows the actual stage, cleanup model and completed/total cleanup tiles instead of leaving “Translating 1/N” visible through inpainting. A full page can contain many 512-pixel tiles and take several minutes with FLUX; no model is silently substituted. The screen's automatic idle timer is disabled while the app is busy and restored when the job ends; this does not enable background inference. A failure includes the page/stage context and no incomplete page is committed. Model downloads use temporary files rather than holding the whole GGUF in memory; model weights are excluded from backup. Keep the app in the foreground during processing and model downloads; background transfer/resume is not implemented.

## Hosted translation APIs (0.3.0)

In **Settings → Translation → Provider**, choose **OpenAI, Anthropic, DeepSeek, Qwen, MiniMax, Gemini, Grok, OpenRouter or DeepL**. Enter the provider's own API key, press **Validate key and load models**, then choose a model. Validation uses authenticated discovery/usage requests and does not generate a paid sample. The model picker stays unavailable until validation succeeds. Changing a key, region or workspace cancels and invalidates the displayed validation. Each provider keeps its own selected model. Saved keys can be removed from the same section.

Keys are stored in this device's Keychain (`AfterFirstUnlockThisDeviceOnly`), never in preferences, source, chapter exports or the IPA. Hosted requests use provider-owned HTTPS origins, reject redirects and send credentials in headers. The selected service receives OCR text and the shared translation instructions; images remain on the device. LM Studio and on-device processing remain available. The app does not transfer development-machine keys to iPad.

| Provider | Discovery and translation contract |
| --- | --- |
| OpenAI | Authenticated Models API; Responses API with strict JSON Schema and `store: false`. Text model families are filtered to exclude image/audio/realtime and other specialized APIs. |
| Anthropic | Paginated authenticated Models API; Messages API with `output_config.format`. |
| DeepSeek | Authenticated model list; strict function output through the documented beta Chat Completions API. |
| Qwen | Select API region and enter the **Model Studio workspace ID**. Authenticated paginated model catalog plus native JSON Schema; currently Qwen 3.7 Plus/Flash/Max and 3.8 Max/Flash families. JSON-only models are excluded. |
| MiniMax | International or China API; authenticated model list. Currently only **MiniMax-Text-01** is selectable because the native API documents JSON Schema for that model. A key with no supported model reports this explicitly. |
| Gemini | Paginated authenticated Models API; `generateContent` with `responseJsonSchema`. Specialized image/audio/live models are excluded. |
| Grok | Authenticated language-model catalog; Responses API with strict JSON Schema. |
| OpenRouter | Validates `/key` first because its model catalog is public. Lists text models advertising `structured_outputs`; requests require parameter-compatible routing. |
| DeepL | Authenticated usage request; Free/Pro origin selected by the key's `:fx` suffix. Offers the documented **quality**, **speed**, and **prefer quality with fallback** modes instead of pretending it has an LLM catalog. Checks target-language availability before translating. Additional instructions use `custom_instructions` and are limited to 300 characters. |

A provider accepting a key does not guarantee generation permission or remaining credit for every model. Authentication, rate/quota limits, unsupported requests and incomplete answers stop the page without committing partial translations. No provider fallback or JSON repair is performed. The shared prompt is unchanged; providers receive a portable native schema and the existing strict decoder checks exact segment count, unique IDs and text types. Runtime discovery is cached for at most five minutes per credential/origin; the actual translation request still enforces current authorization.

The Swift adapters are traceable to the desktop provider ownership at [`c9717fc`](https://github.com/flowerfx/koharu/tree/c9717fc9cd11e2d0c540d9912f7f40a234bc1048/crates/koharu-translator/src/remote), updated against the official APIs: [OpenAI](https://developers.openai.com/api/docs/guides/structured-outputs), [Anthropic](https://platform.claude.com/docs/en/build-with-claude/structured-outputs), [DeepSeek](https://api-docs.deepseek.com/guides/tool_calls/), [Qwen](https://www.alibabacloud.com/help/en/model-studio/qwen-structured-output), [MiniMax](https://platform.minimax.io/docs/api-reference/text-post), [Gemini](https://ai.google.dev/gemini-api/docs/structured-output), [Grok](https://docs.x.ai/developers/model-capabilities/text/structured-outputs), [OpenRouter](https://openrouter.ai/docs/guides/features/structured-outputs), [DeepL](https://developers.deepl.com/api-reference/translate/request-translation). Intentional differences are DeepSeek strict tools (instead of JSON-only mode), portable schema constraints for Claude/DeepSeek, header-only Gemini authentication, and authenticated OpenRouter key validation.

## Build

Install Xcode with an iOS SDK and `brew install xcodegen`, then from this directory:

```sh
python3 scripts/bootstrap.py

# Simulator: UI, Apple Vision, white-card rendering and LM Studio.
# Vision-model execution and memory admission are validated on physical hardware.
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

## Verification

Version **0.3.2 (build 9)** passed three focused Debug `TypographyChecks` on the physical M5: line fragments never bisect Vietnamese, English, French, Arabic or Korean words at widths 14–140 pixels; Japanese/Chinese text without spaces retains usable font sizes; and a stale rendered page is regenerated while preserving cached text with no AI model configured. The production renderer also produced a visually inspected multilingual PNG fixture. Existing chapter images must be refreshed with **Run** to use the new layout; this check does not claim that old PNGs change at installation time.

Version **0.3.3 (build 10)** passed focused physical M5 Debug checks for white-card rendering and cached-text refresh. The rendering fixture verifies opaque padded corners, the 1-pixel gray border, unchanged pixels outside the mask and untranslated regions, and identical output when overlapping regions are reversed. Its output was visually inspected. No full-chapter rerun was required.

Version **0.3.4 (build 11)** passed three focused physical M5 Debug checks (0.038 seconds): configurable batches preserve all 103 segments in order for both 100 + 3 and 7-segment groups; the batch setting persists separately from processing preferences; and a cached import publishes the correct chapter before translation, while rejected imports publish no route. Six other hosted-provider checks passed on Mac against the unchanged native sources and mocked HTTP, including 100-segment output budget/strict decoding and the existing nine provider protocols. The engine check requires the app's shared prompt resource and was therefore run on the physical iPad instead of the temporary Mac harness. Simulator UI verified editing/saving 25, and a reader opened on chapter 1 switched to chapter 2.1 (14 original pages) after Rawkuma import. The simulator deliberately had no local model, so this checks navigation before a translation failure, not a live 100-segment model generation or whole-chapter quality.

Version **0.3.1 (build 8)** corrects misleading whole-project progress reporting during cleanup. A focused physical M5 check completed **two consecutive pages** with real Apple Vision OCR, the configured LAN LM Studio and one FLUX tile per page, persisting both rendered results in **112.712 seconds**. The check uses an isolated temporary library and the already installed immutable weights; it does not edit the user's chapters or preferences. The same native reader job (`MangaLibrary.run`) is exercised with its default whole-project scope. A separate physical check completed two FLUX tiles in **100.538 seconds** in the same page/context, verifying progress events 0 → 1 → 2 and a saved image of the original size. Per-tile autorelease pools now release native temporary buffers without waiting for all tiles in the page. After installation, the user's actual **35-page chapter completed with LM Studio + AOT GAN**. The saved manifest contains 35 rendered pages with one matching configuration fingerprint, and the processing bar disappeared normally. The top reader page counter remains the current reading position; the bottom stage label reports processing progress. This establishes chapter completion; it is not a full editorial review of translation quality or text placement, or a controlled speed benchmark.

Version **0.3.0 (build 7)** was built and archived in Debug, exported as a development IPA, and installed/launched on the physical iPad Pro M5 using Kuruby signing. Five focused `HostedProviderTests` passed on Mac using the same unmodified Foundation/Keychain/network/translation sources and XCTest file through a temporary Swift package: nine provider request/response contracts, credential/region boundaries, DeepL Free routing, authenticated OpenRouter validation, pagination, revoked-key cache invalidation, truncated responses and corrupt segment IDs. Simulator XCTest injection stalled in dyld before app startup, so it is not reported as a successful iOS test run. The standalone simulator app runs normally: its provider menu, Vietnamese Qwen region/workspace form, German long labels and a live invalid-OpenAI-key rejection with the model selector locked were checked visually. All 13 localization catalogs contain the same 183 keys and pass plist validation.

The production Swift OpenAI client ran on Mac using an existing environment credential: 52 filtered model entries were discovered and `gpt-4.1-mini` returned two valid Japanese-to-Vietnamese translations through the shared prompt/schema/decoder. No development-machine key was saved to iPad. The other eight services have protocol tests but have **not** been exercised with valid customer keys; model access, regional account configuration and translation quality still need confirmation with those accounts. No new whole-chapter test was run for this provider change.
 limits

Version **0.2.1 (build 6)** adds the 12 interface choices and British English as a separate translation target. All 148 UI/native-error keys and the two privacy descriptions have matching catalogs in 13 bundles (12 requested choices plus generic English fallback). Two focused Rust language tests passed. On the iPad 26.4 simulator, the complete language menu, both four-model menus, live Arabic RTL → German LTR switching, German long labels and unchanged Vietnamese translation target were checked. Recreating the presentation controllers when crossing RTL/LTR fixes UIKit's stale mirrored text while preserving the selected tab and browser owner. AOT GAN was downloaded through the app, checksum-verified and published as ready. The signed Debug build was installed on the physical M5. Model memory/inference results below come from hardware; the simulator is used for UI checks and can report unavailable memory to the admission check. Translations were drafted with the user's local LM Studio model and reviewed for UI terminology; this is not a native-speaker review of every language.

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
