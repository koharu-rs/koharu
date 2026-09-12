# CBZ 与项目工作流交付记录

## 已实现

1. **Phase 1：CBZ。** File → Export 增加 CBZ；全项目按页面顺序导出，不使用页面栏选择。PNG/PSD 继续使用原选择范围和命名规则。
2. **Phase 2：预设与术语表。** Process 增加 Workflow presets 和 Project glossary，保留 Standard 与原处理命令，新增 Consistent Project Translation。
3. **主要文件。** `crates/koharu-app/src/commands/{cbz,workflow,glossary}.rs`；`crates/koharu-pipeline/src/{scheduler,execution,terminology}.rs`；`crates/koharu-scene/src/glossary.rs`；`crates/koharu-translator/src/{glossary,prompt,backend,lib}.rs`；前端 `ExportCbzDialog.tsx`、`WorkflowDialog.tsx`、`ProjectGlossaryDialog.tsx`、`TitleBar.tsx`、`ActivityCenter.tsx`；正式生成的 `packages/bridge/src/protocol.ts`。
4. **新数据结构。** `ExportConfig`、`ArchiveImageFormat`；`WorkflowPreset`（name/scheduling/scope/stages/review_glossary）、`WorkflowScope`、`WorkflowProgress`；`ProjectGlossary`（entries/candidates/ignored）、`GlossaryEntry`、`GlossaryCandidate`、八类 `GlossaryCategory`。共享 Job 增加 kind、workflow 和 awaiting_review 状态。
5. **新 UI。** CBZ 格式/质量对话框；预设选择、编辑、保存、删除与范围选择；术语已确认/候选分页表格；人工增删改、启停、逐项/批量确认、忽略、JSON 导入导出；Activity 阶段列表与审核入口。新文案提供中文、英文，其余语言保留英文回退。
6. **CBZ 路径。** 正式 renderer → rasterizer → 单页编码 → Stored ZIP member → 释放单页 buffer。PNG 共享正式 encoder；JPEG RGBA 合成白底后编码 RGB；JPEG/WebP 质量默认 90/85，限制 1–100，并保存于现有配置系统。P001 起编号，超过 999 页自动扩宽。相邻目录唯一 `.part` 文件，finish/sync 后原子 persist；失败/取消清理，保留原目标文件。
7. **调度。** 预设按 detection → OCR → terminology → translation → inpainting 顺序执行，省略的阶段不自动补齐。启动时固定项目/选页范围；每个 pipeline 阶段完成全部范围后才进入下一个。复用现有 scheduler、pipeline、model lifecycle 和提交机制；阶段内模型复用，阶段结束卸载，页面缓存逐页释放。没有引入 DAG 引擎。
8. **术语分析。** 逐页读取已有 OCR，不重新识图、不向 LLM 发送语料。结合频次、跨页重复、日文姓氏/敬称、片假名、复合词及组织/地名/称号/技能/物品线索。NFKC 和边缘标点规范化；同页同一 content 不重复计数；规范化去重、确定性排序。最多保留 5000 个候选，超过 100000 个不同词时明确报错并要求缩小范围。已确认/忽略项排除，已有候选译文与人工分类在重新分析时保留。
9. **保存。** 术语表作为 revision 1 的原生 project component 保存；随项目重开恢复，参与 undo/redo，不依赖 provider。旧项目没有该 component 时视为空表。预设与导出质量使用现有配置系统，缺失字段有默认值。编辑使用项目身份和预期术语版本校验，避免覆盖其他修改。JSON 导入上限 4 MiB，只合并已确认项，遇到冲突整次不提交。
10. **实际注入。** 每个现有翻译 batch 只选取启用且匹配的确认项，NFKC 匹配、最长项优先、英文单词边界保护。公共 prompt 明确固定 target、仅允许语法所需变化；本地和所有提示词型云端 provider 共享。DeepL/Google Cloud Translation/彩云在公共 translator 层保留确认术语，翻译周边片段后重组，保持原 segment 数量。翻译 patch 观察项目术语版本，防止旧输入结果覆盖新术语。
11. **取消与进度。** 两阶段均使用共享 Job/Activity/stopJob。CBZ 显示当前页面与完成数，逐页停止并清理 partial。工作流显示每阶段状态与页数；候选审核通过同一 Job 的 oneshot 屏障暂停/继续，等待审核也能取消。取消/失败不回滚已提交页面；可在预设中勾选剩余阶段重新运行。
12. **新增测试。** ZIP 重开、顺序、1/3/1000 页命名、20 页 WebP/85 解码；PNG/JPEG/WebP 解码和尺寸、白底和半透明合成、质量边界、部分写入后取消与失败清理；阶段屏障、范围、失败停止、取消保留与重跑；20 页 scene OCR 统计和实际共享 translator 提交；术语保存重开/编辑/删除/停用/undo/旧项目、候选去重与保留人工译文；最长匹配与公共 prompt；前端格式选择、选页预设、候选确认、保存后继续与取消。

## 验证命令与结果

所有命令在仓库根执行。系统 Rust 1.98.1；Bun 1.3.14。复用已有 Visual Studio CMake/Ninja，临时 libclang 仅用于构建；未修改仓库构建系统。最终 shell 另加入已安装 Windows SDK 的资源编译器路径。既有字体测试按项目机制初始化了缓存中的 LibTorch，未下载检测/OCR/修复模型权重。

| 命令                                                                                                                                               | 结果                                                                                     |
| -------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `bun install --ignore-scripts --frozen-lockfile`                                                                                                   | 通过，安装依赖；未运行仓库全局 postinstall                                               |
| `cargo fmt --all --check`                                                                                                                          | 通过                                                                                     |
| `cargo check -p koharu-app`                                                                                                                        | 通过                                                                                     |
| `cargo check`                                                                                                                                      | 通过，默认桌面目标                                                                       |
| `cargo clippy -- -D warnings`                                                                                                                      | 通过                                                                                     |
| `cargo clippy -p koharu-app --all-targets -- -D warnings`                                                                                          | 通过                                                                                     |
| `cargo test -p koharu-app cbz::tests`                                                                                                              | Phase 1 的 4 项通过；最终又增加 20 页 WebP 测试                                          |
| `cargo test -p koharu-scene -p koharu-pipeline -p koharu-translator -p koharu-app --lib`                                                           | 163 项通过（15 + 61 + 35 + 52）                                                          |
| `cargo test --workspace --lib --tests --no-fail-fast`                                                                                              | 通过：394 项，0 失败；23 项沿用仓库已有 ignore（21 项原生运行库/模型检查、2 项排版基线） |
| `cargo run -p koharu-app --bin generate`                                                                                                           | 正式生成成功；重复生成 SHA256 相同，无漂移                                               |
| `bun run lint`                                                                                                                                     | bridge/ui/app 均通过                                                                     |
| `bun x tsc --noEmit -p packages/koharu/tsconfig.json`                                                                                              | 通过                                                                                     |
| `bun run --filter @koharu/app test`                                                                                                                | 14 个文件、100 项通过                                                                    |
| `bun run --filter @koharu/app test -- tests/components/cbz-export.test.tsx tests/components/workflow-glossary.test.tsx`                            | 最后边界修正后 5 项通过                                                                  |
| `bun x wasm-pack@0.15.0 build crates/koharu-canvas --dev --target web --out-dir ../../packages/bridge/src/wasm --out-name koharu_canvas --no-pack` | 通过，生成前端所需本地 WASM；产物按仓库规则忽略                                          |
| `bun run --filter @koharu/app build`                                                                                                               | 通过，Next 生产构建与静态生成                                                            |
| `git diff --check`                                                                                                                                 | 通过                                                                                     |

初次前端构建因缺少 WASM 产物失败，执行正式 WASM 构建后通过。新增保存重开测试初次因测试持有提交快照造成文件锁失败，释放快照后通过；新测试的类型错误也已修正。完整 workspace 的两个字体处理测试经过约 171 秒初始化后通过。另一次显式过滤字体测试的尝试（`cargo test --workspace --lib --tests --no-fail-fast -- --skip font_detector::processor::tests::maps_upstream_regression_layout --skip font_detector::processor::tests::preprocesses_a_batch_in_upstream_shape_and_range`）在构建时因 shell 缺少 `rc.exe` 路径失败；补入现有 Windows SDK 后恢复未过滤验证。Windows 链接器仍输出现有 LIBCMT/LNK4098 警告，未通过改变核心链接配置隐藏它。

## 场景验收与限制

- **A：逻辑验证。** 20 页原生 scene，预置 OCR：高橋 12 页、エーテル 8 页；统计、确认术语、通过公共 translator 提交统一译文的自动化测试通过。完整四阶段顺序另有 scheduler 屏障测试，审核继续有 UI 测试。没有将这些测试冒充真实 20 页漫画的 GPU/模型端到端运行。
- **B：通过。** 20 页 WebP、质量 85 的 CBZ，P001.webp–P020.webp，ZIP 重开并逐成员解码验证。
- **C：通过。** 部分页写入后取消，partial 清除、新 final 不存在、旧 final 保留；模拟最终提交失败同样清理。
- **D：通过。** 取消/提交失败保留 20 页 OCR 与已完成翻译，禁止进入后续修复阶段；重跑翻译完成全部页面。

没有提供真实漫画测试项目，也未配置/下载检测、OCR、修复权重或付费翻译凭据，因此未执行真实图像模型的完整桌面交互验收。候选提取为可解释规则，识别/分类可能需要人工修正；候选界面现支持使用当前模型 AI 分批填写空白译文；LLM 候选提取精炼仍是可选后续增强。传统翻译服务的片段保护会减少整句上下文，语序变化需要人工检查。工作流进程本身不跨应用重启恢复；已提交结果和术语保存，重启后可选择剩余阶段重跑。

## 历史方案适配与后续拆分

- [#1009](https://github.com/koharu-rs/koharu/pull/1009) / [#1016](https://github.com/koharu-rs/koharu/issues/1016)：保留全项目 CBZ、三种内部格式、顺序编号与质量设计；适配当前 `CefRuntime`、正式 renderer/rasterizer、共享 Job、配置与协议生成器；使用 NamedTempFile 原子提交。没有机械 cherry-pick，未添加 AVIF/CB7。
- [#996](https://github.com/koharu-rs/koharu/issues/996)：增加可选 stage-major 屏障与模型复用，保留现有 scheduler 行为；失败后使用现有提交语义并可选择阶段重跑，没有扩展为通用重试引擎。
- [#592](https://github.com/koharu-rs/koharu/issues/592)：在当前已有阶段选择之上保存命名预设、范围和调度方式，没有恢复旧 pipeline 架构。
- [#339](https://github.com/koharu-rs/koharu/issues/339) / [#511](https://github.com/koharu-rs/koharu/issues/511)：项目级确认术语与候选分离，跨 provider 持久保存并真正注入翻译。
- [#864](https://github.com/koharu-rs/koharu/issues/864)：采用原生 project component 加 JSON 交换，没有另建旁路数据文件，也没有额外引入角色/代词系统。

适合拆为三个 upstream PR：① CBZ 与导出设置/Job；② 项目术语数据、编辑器与共享翻译注入；③ stage-major 预设、术语分析和审核屏障。代码目前保留在工作区，未 commit、push 或创建 PR。

## 追加：AI 填写候选译文

候选列表新增“AI 填写空白译文”。复用 Pipeline 持有的同一 Translator、当前 provider/model/目标语言/生成参数及已确认术语匹配，不另建服务或凭据配置。每批最多 24 项、4096 字节原文，显示填写进度；停止后续批次会保留当前批次结果。已有译文不覆盖，建议仍需要人工确认并保存。失败时已完成批次留在表中，可再次点击继续填空。

新增 `suggest_glossary_translations` 桥接命令、Pipeline 批量建议方法与前端分批函数；项目身份和处理运行状态校验阻止跨项目应用建议。专项测试覆盖请求数量/字节限制、共享配置和确认术语、空白填充、保留人工译文及停止后续批次。

供本地测试的构建命令：`bun run --filter @koharu/app build`，然后 `cargo build -p koharu --features tauri/custom-protocol`。这是内嵌前端资源的 debug 测试构建，无需前端开发服务器；保留仓库规定的 CEF 调试端口。替换前备份原 exe，并比较部署前后的 SHA256。

### Local AI glossary test build deployment

- Built the desktop executable with `cargo build -p koharu --features tauri/custom-protocol`: passed (debug profile, embedded frontend assets).
- Staged executable `--version` smoke check: passed, version 0.82.1.
- Replaced the installed executable atomically after a transient file-lock retry; preserved the original as `koharu.exe.before-ai-glossary-20260912-213626.bak`.
- Installed executable SHA-256: `F76F6BE8003CF6882CF93EE4033A9204002041C24CCEAB7073D2D27108AC9122`.
- Verified installed executable matches the build and backup matches the original. No project data or configuration was modified.
- Live translation-provider requests remain for user testing; automated tests cover batching, preserving existing translations, stopping after the active batch, and shared translator request validation.

### Follow-up: export throughput, hybrid GPUs, and Run All integration

- CBZ page preparation now uses an ordered buffer of four pages, matching loose PNG export concurrency. Rendering and CPU encoding overlap while a single blocking ZIP writer preserves project order and partial-file cleanup.
- Native export explicitly requests a high-performance WGPU adapter unless overridden by the existing WGPU environment options. Export activity reports the selected adapter.
- CUDA discovery accepts CUDA 13.x minor-compatible drivers instead of requiring driver API 13.3. The local NVIDIA driver reports API 13.1; the previous gate excluded the discrete GPU before runtime selection. Compatibility reference: https://docs.nvidia.com/deploy/cuda-compatibility/minor-version-compatibility.html (PTX/new-feature restrictions still apply).
- Workflow settings persist `enabled` and `active_preset` alongside the existing preset list. The ordinary backend `process` command routes entire-project and multi-page full runs through the selected workflow when enabled. Single-page, entity and partial-stage actions retain their existing semantics.
- Settings → Pipeline exposes the switch and preset selector. The editor toolbar exposes Project glossary directly, and entering glossary review opens it automatically.
- Regression validation: 58 frontend tests and 17 app command tests passed; frontend typecheck, lint and production asset build passed. GPU runtime and optimized desktop verification are recorded below after completion.

#### Follow-up verification and installation

| Command | Result |
| --- | --- |
| `cargo run -p koharu-app --bin generate` | Passed; repeating the generated executable left the protocol SHA-256 unchanged |
| `cargo test -p koharu-app --lib commands::` | 17 passed |
| `cargo test -p koharu-rasterizer --test compositor -- --nocapture` | 2 passed; selected NVIDIA GeForce RTX 5070 Laptop GPU, DiscreteGpu, Vulkan |
| `cargo test -p koharu-runtime --lib cuda_13_minor_compatible_drivers_are_not_rejected` | 1 passed |
| `cargo clippy -p koharu-app -p koharu-rasterizer -p koharu-runtime -- -D warnings` | Passed |
| `cargo run -p koharu-ml --example check_device -- <runtime-store>` | CUDA0 RTX 5070 initialized; matrix multiplication result 262144 and convolution result 97200 both passed |
| `bun x tsc --noEmit -p packages/koharu/tsconfig.json` | Passed |
| `bun run --filter @koharu/app test -- tests/components/workflow-settings.test.tsx tests/components/workflow-glossary.test.tsx tests/components/glossary-ai.test.tsx tests/components/cbz-export.test.tsx tests/components/editor-components.test.tsx tests/lib/runtime.test.ts` | 58 passed |
| `bun run lint` | Passed |
| `bun run --filter @koharu/app build` | Passed |
| `cargo fmt --all --check` | Passed |
| `git diff --check` | Passed (existing line-ending notices only) |
| `cargo build -p koharu --release --features tauri/custom-protocol` | Passed; optimized desktop executable with embedded frontend |

- Installed the verified Release 0.82.1 executable atomically. Preserved the previous debug build as `koharu.exe.before-workflow-gpu-20260912-223933.bak`; the earlier original-program backup remains intact.
- Installed SHA-256: `24F4DE43AFDACAAB563CD7F01B50A342F21E4644A94D34A5EF90FA5311FF4259`. Both installed-build and original-backup hashes matched.
- Installed missing application-managed CUDA runtime packages through the existing runtime store. No system driver, project data, or user provider settings were changed.
- No representative user-project CBZ wall-clock benchmark was run. The verified changes remove strictly serial preparation, use the discrete export GPU, and replace the debug executable with an optimized build; actual throughput still depends on page complexity and archive image format.
### Follow-up: online translator throughput

- Provider-backed stages no longer take an accelerator lane, so pages no longer queue behind the GPU gate for network work. `Translator::concurrent` answers per selection: bundled and LM Studio models stay serialized, OpenAI-compatible endpoints overlap unless their `base_url` is loopback, and other hosted providers overlap.
- The page overlap limit now follows that capability instead of the scheduling mode. `page_capacity` returns `MAX_OVERLAPPING_PAGES` (4) for provider stages in both page-major and stage-major runs, and 1 for every accelerator stage, so detection, OCR and inpainting still keep exactly one job in flight per stage.
- The busy-stage decision was extracted into `saturated_stages`, covered by `only_provider_stages_overlap_pages` and `page_major_overlaps_provider_stages_but_serializes_local_models` (the latter replays the execution loop and asserts four overlapping translations for a provider and one for a local model).
- Project glossary AI fill now issues up to three batches concurrently through the shared translator. Stopping suppresses new batches while in-flight suggestions are still applied; `stops issuing new batches and keeps in-flight suggestions` asserts that three dispatched batches stay three after the stop.
- `suggest_glossary_translations` no longer waits for the pipeline lock when the selected provider overlaps requests, so glossary suggestions stay responsive during long runs.

| Command | Result |
| --- | --- |
| `cargo test -p koharu-pipeline --lib` | 65 passed |
| `cargo test -p koharu-translator --lib` | 54 passed |
| `cargo clippy -p koharu-pipeline -p koharu-translator --all-targets -- -D warnings` | Passed |
| `bun run --filter @koharu/app test -- tests/components` | 82 passed |
| `bun run --filter @koharu/app lint` | Passed |
| `bun run --filter @koharu/app build` | Passed |
| `cargo build -p koharu --release --features tauri/custom-protocol` | Passed |

Installed the Release 0.82.1 executable atomically and preserved the previous build as `koharu.exe.before-translation-concurrency-20260912-232801.bak`. Installed SHA-256: `619B6ABF7C10AE256DDD9F8D2F11CE1D75634CD0F8FEFC01EF6A1961B204068E`.

Known limit: a page-major run limited to a single provider stage (`Operation::Only { stage: Translation }`) still passes through the sliding page window, which stays at one page for a one-stage operation. Stage-major runs and multi-stage page-major runs both reach the four-page overlap. Raising the window for single-stage operations would change that invariant and the stop-after-one-page test that pins it, so it was left unchanged.

### Follow-up: translation JSON recovery and retry

- `prompt::translations` returns a `TranslationOutcome` (`translations`, `missing`, `error`) instead of failing on the first decode error. A response that does not decode strictly is narrowed to its JSON object first (markdown fences, leading prose and trailing characters are stripped), then salvaged segment by segment with a balanced-brace scanner that understands string literals and escapes, so a truncated or tail-corrupted object array still yields every complete pair.
- `Translator::translate` retries only the segments a provider never returned, at most `MAX_ATTEMPTS` (3) per batch. Repair requests keep context, glossary and image, re-send just the missing segments, and run deterministically (`temperature: 0.0`, `top_p: 1.0`, thinking disabled). Results fold back by original segment index; when segments are still missing the batch fails with the original decode error as context plus the segment-count mismatch.
- The repair path is covered by `repair_request_narrows_segments_and_keeps_the_rest`, `repair_generation_is_deterministic` and `merge_missing_folds_repairs_back_by_original_index`; the last one pins that repair positions map back onto the original batch indices, including a still-missing tail.
- Providers now return `Result<TranslationOutcome>` uniformly: generative backends wrap `prompt::translations`, while promptless backends (DeepL, Google Cloud, Caiyun) and the bundled local backend report `TranslationOutcome::complete`. This is internal to the crate: `Translator::translate` still resolves to `(&static provider id, Vec<String>)`.

| Command | Result |
| --- | --- |
| `cargo check -p koharu-translator --all-targets` | Passed |
| `cargo test -p koharu-translator --lib` | 61 passed |
| `cargo clippy -p koharu-translator --all-targets` | Passed, no warnings |
| `cargo fmt -p koharu-translator -- --check` | Passed |
| `cargo build -p koharu --release --features tauri/custom-protocol` | Passed, 2m29s incremental |

Installed the Release 0.82.1 executable and preserved the previous build as `koharu.exe.before-translation-retry-20260913-000841.bak`. Installed SHA-256: `832BE2AC38AE7EC8C23A2DF00E295FFEF19213F0E6036473C5D5A3E04DAA1A37`.

Before the build, 29.3 GB of regenerable build cache was removed from `target/` (`debug/incremental`, `release/incremental` and every `*.pdb`); the tree went from 61.2 GB to 31.9 GB. `build/` (torch/llama native output) and `release/deps` were kept so the release link stayed incremental.

Known limits: salvage cannot invent segments the model never emitted, so those depend on the repair attempt; DeepSeek keeps its upstream-recommended translation default `temperature: 1.3` for the first attempt and only the retry drops to 0. A malformed response that still fails after three attempts now surfaces both the decode error and the segment-count mismatch instead of a bare count message.
