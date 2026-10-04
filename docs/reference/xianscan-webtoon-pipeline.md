# XianScan 的 Webtoon 处理实现参考

对 `xianscan-rust` 的技术调查笔记。目的是在设计 Koharu 的条漫处理链路时，能直接引用已被验证的做法与已被证伪的做法，避免重复试错。

> **Koharu 侧的落地与实测结论见 [`koharu-webtoon-support.md`](./koharu-webtoon-support.md)。** 本文只描述 XianScan 怎么做，不包含"我们采纳了什么、实测结果如何"——那部分在姊妹文档里。

| 项 | 值 |
| --- | --- |
| 上游 | `https://github.com/ArbenApura/xianscan-rust` |
| 调查提交 | `075d36f359cdcad1d08ea88d2f4d927640789c26`（2026-09-30） |
| 本地路径 | `../xianscan-rust`（同级检出，非本仓库子模块） |
| 变体检出 | `../xianscan-rust-ch`（`OHW0nder/xianscan-rust-ch` @ `cd2a5b2`，i18n 分支） |
| 调查方式 | 6 路并行只读代码调查，交叉核对调用点与行号 |
| 调查日期 | 2026-10-04 |

**引用约定**：本文所有 `路径:行号` 均相对 XianScan 仓库根目录，行号对应上表提交。核对时用 `git -C ../xianscan-rust show <commit>:<路径>`。

**注意**：XianScan 的检测权重 `rfdetr-seg-2xlarge.onnx` 来自 HF `DevilishDaoSaint/koharu-layout-rfdetr`，即 Koharu 自己的模型。文中关于检测的用法，本质是我们自己的模型在别人手里的调用方式，这部分的可信度最高。

---

## 1. 摘要

XianScan 是 Rust + ONNX Runtime 的本地漫画/条漫翻译工具，双进程结构：Rust sidecar 只做重 ML，Node web 层做编排、LLM、渲染、持久化。

对 Webtoon，它的核心策略可以概括为一句话：

> **切页优先、切块备用** —— 不试图让检测器处理 20000px 长图，而是把长图在进入检测器之前就切成接近原生长宽比的标准页；然后在检测端（Rust）把全部几何语义算好落库，渲染端（Skia）只管"在给定 box 里把译文缩放塞满"。

五个环节的成熟度差异很大：

| 环节 | 成熟度 | 一句话评价 |
| --- | --- | --- |
| 区域构建 | 高 | 5 盒分离模型是这个项目最值得照搬的设计 |
| 擦除 | 高 | 规划器/执行器分离 + 拓扑孔洞回填，算法扎实 |
| 翻译 | 高 | Prompt 工程与解析鲁棒性投入最大，术语表体系完整 |
| OCR | 中高 | 领域经验沉淀最厚（竖排、双尺度阈值、救援择优） |
| 检测 | **低（对条漫）** | 暴力拉伸无 letterbox，分块实现完善但因净收益为负被关闭 |
| 嵌字 | 中 | 字号/断行/字体处理细致，但竖排未实现 |

---

## 2. 架构

### 2.1 双进程职责切分

```
┌─ Rust sidecar (axum, :8124) ────────────┐   只做重 ML，无状态
│  POST /pages/reslice   → 长条图切页      │
│  POST /pages/analyze   → 检测+OCR+区域构建│
│  POST /pages/clean     → LaMa 擦除        │
└──────────────┬─────────────────────────┘
               │ multipart HTTP（模型 Mutex 复用）
┌──────────────┴─────────────────────────┐
│ SvelteKit web (Node)                    │   编排 + 翻译 + 嵌字 + 持久化
│  chapter-pipeline.ts  阶段化流式编排      │
│  translate/ + llm.ts   LLM 翻译          │
│  typeset.ts + typeset/ @napi-rs/canvas  嵌字渲染 │
│  drizzle/SQLite + uploads|clean|output/  │
└─────────────────────────────────────────┘
```

**可直接照搬的第一件事**：重 ML 与业务编排分离。检测/OCR/擦除放在无状态 HTTP 服务里，模型用 `Mutex<Option<Model>>` 单例复用；web 侧只管流程、LLM、排版、DB。

带来的直接收益是 HTTP 层能在 analyze 结束后**立刻释放 detector 锁**——`analyzer.rs:132` 的 `analyze_with_ocr()` 只接收 `&mut Option<RapidOcr>`，签名上就不需要 detector，于是 clean 请求不必排在 analyze 后面。

### 2.2 锁模型

`src/pipeline/shared.rs:19-38`

- **每模型一把锁**，不是整引擎一把：`detector` / `ocr` / `inpainter` 各一个 `Mutex<Option<Model>>`
- **锁序铁律**：`detector → ocr → inpainter`，注释明确 *"NEVER TAKE AN EARLIER ONE WHILE HOLDING A LATER ONE"*
- **锁中毒恢复**：`lock_recover()` 返回 `(guard, poisoned)`，调用方可安排该模型后台重建（`router.rs:232-249`）
- **重载峰值**：逐个替换且先 drop 旧的再加载新的，峰值内存 = 1 个模型而非 2 个引擎

### 2.3 端到端阶段序列

```
[磁盘 uploads/<chapterId>/<uuid>.webp]
   │
   │  ① reslice（可选，仅长条漫/超长章节）
   │     纵向拼接 → 行统计画像 → 逐窗口检测文字禁区 → 选天沟切点 → 裁片
   ▼
[web] ensureWebPBuffer()  统一转 WebP + 像素上限断言
   │
   │  ② POST /pages/analyze（每页一个，页并发 3）
   ▼
┌─ RUST analyzer.rs ────────────────────────────────────────┐
│ STAGE 1  fuse_detections()                               │
│    ├─ 布局检测（RF-DETR-Seg，高长图自动切 tile）            │
│    ├─ 全文 OCR（tiled + 按语言路由多识别器）                │
│    ├─ OCR 行过滤（6 条启发式）                             │
│    └─ 裁片救援 + 批量单行识别（chunk 16）                  │
│ STAGE 2  容器候选收集 + 阅读顺序排序（analyzer.rs 约 1200 行）│
│    ├─ 水印碎片剥离（并按比例裁剪 polygon）                  │
│    ├─ 白/黑气泡像素级包络提取、竖排长破折号合并、竖列合并     │
│    ├─ deduplicate_boxes(0.40) → sort_regions_top_to_bottom │
│ STAGE 3  build_regions()                                  │
│    ├─ 行清洗 → 跨框行切分 → orphan 认领 → 方向判定 → 剪枝   │
│    ├─ cluster_lines_into_utterances（一句话聚类）           │
│    ├─ try_refine_cluster_crop（裁片精修 OCR）               │
│    ├─ should_reject_candidate_region（过滤）                │
│    ├─ deduplicate_and_unify_regions（去重合并）             │
│    └─ expand_bubble_text_boxes（阻尼扩张 + 居中）          │
└───────────────────────────────────────────────────────────┘
   │  AnalyzeResponse { regions[5 个 box + polygon + angle + vertical] }
   ▼
[web] ③ persist_regions → SQLite；④ annotated/ 标注预览
   │
   ├─ ⑤ POST /pages/clean（Rust）→ LaMa + shrinkwrap → clean/
   └─ ⑥ LLM 翻译（每本书串行，术语表单调追加）→ translations 表
   ▼
[web] ⑦ typesetPage()（Skia）
   ├─ fitFontSizeWithLines（缩放/换行/断词塞进 typeset_box）
   ├─ decollision（区域避让）
   └─ pickTextColor（背景采样定字色/描边）→ output/
```

---

## 3. Webtoon 总体策略

这是 XianScan 投入最大的部分，也是最值得学的地方。

### 3.1 reslice：拼成连续画布再按天沟切开

入口 `src/ml/reslice.rs:882` `smart_reslice_chapter_with_models()`

```
① stitch_images_vertically()            整章按最宽页缩放后纵向拼成一张连续画布
② compute_row_profile()                 逐行算灰度方差、三等分列方差、行间差分、边缘能量
③ find_optimal_cut_points_with_models() 逐窗口选切点
④ crop_imm() + rayon 并行 WebP 编码
```

切点搜索是**四级递进**，每级都是上一级失败后的兜底：

| 优先级 | 方法 | 判据 | 位置 |
| --- | --- | --- | --- |
| 1st pass | 纯白/纯黑宽天沟 | `row_var<15 && max_col_var<20 && edge<10`，连续带 ≥10px 且上下有净空；评分 `band_len*2.5 - dist*0.05` | `reslice.rs:571-619` |
| 2nd pass | **OCR 引导的禁区** | 无 1st pass 结果时才调模型检测文本/气泡，每框上下扩 35px 禁区 | `reslice.rs:621-629` |
| 2B | 回退 gutter | 非禁区 + 空白行，离理想切点最近者胜 | `reslice.rs:631-655` |
| 2B' | 最低视觉能量行 | `flatness = -(var*0.1 + diff*2.0 + edge*1.5) - dist*0.02` | `reslice.rs:657-676` |
| 3 | 向外扩张窗口 | 窗口被文字占满时逐步外扩 | `reslice.rs:678-697` |
| 兜底 | `fallback_cut()` | **也绝不允许落在禁区** | `reslice.rs:728-780` |

### 3.2 两条关键工程经验

**(a) 禁区检测优先用轻量 OCR**

`src/ml/reslice.rs:237-295`

```rust
// ULTRA-FAST OCR DETECTOR (DBNet) — FINDS ALL TEXT POLYGONS & LINES IN ~25-35MS.
if let Some(ref mut o) = ocr {
    if let Ok(boxes) = o.detect_only(&tile) { ... }      // 首选
} else if let Some(ref mut det) = detector {
    if let Ok(res) = det.detect(&tile) { ... }            // 回退 136MB 模型
}
```

禁区扫描本身也分块（tile 高 2400 / 步进 1900，重叠 500px），每块后检查取消。长图上这是数量级的差距。

**(b) 上下净空校验 —— 对任何"按视觉空白切页"都适用**

`src/ml/reslice.rs:181-201`

```rust
const CUT_AIRSPACE_PX: u32 = 3;
/// A CUT AT `y` IS ACCEPTABLE ONLY IF THERE IS A BLANK AIRSPACE OF AT LEAST
/// CUT_AIRSPACE_PX ROWS IMMEDIATELY ABOVE AND BELOW IT. ...
/// THIS CHECKS THE ROWS BEYOND IT, SO A CUT CANNOT SLICE THROUGH THE NARROW GAP
/// BETWEEN TWO TEXT LINES INSIDE ONE MISSED DIALOGUE BOX.
```

"行方差低"**不足以**证明可切——未检出的气泡内部，行间隙也是平的。必须要求切点上下各 ≥3px 都在空白带之外。

### 3.3 切片高度的取值逻辑

默认 `target=1600 / min=1200 / max=2000`（`web/src/lib/server/chapters/reslice.ts:21-25`，UI 可调）。

这不是随便定的：目标页高 1600 只有检测器输入 768 的 2 倍多。2000px 页缩到 768 时纵向压缩比 ≈2.6，可接受；再高就崩小字。

### 3.4 取消协议：用 run_id 而非 bool

`src/ml/reslice.rs:26-32`

```rust
fn is_run_cancelled(cancel: Option<&AtomicU64>, run_id: u64) -> bool {
    cancel.is_some_and(|c| c.load(Ordering::Relaxed) == run_id)
}
```

单调递增的 run id 存入 `AtomicU64`，**陈旧的取消永远杀不掉新任务**。HTTP 侧 `ResliceProgressFrame { pct, message, done, run }` 每帧打戳，前端忽略非本 run 的帧。

### 3.5 页内分块（tiling）：实现完善但默认关闭

`src/ml/detect/tiling.rs`

| 常量 | 值 |
| --- | --- |
| `TILING_TRIGGER_ASPECT` | 2.5（仅 `h > 2.5w` 才切） |
| `TILING_TILE_ASPECT` | 1.5（块高 = 1.5×宽，全宽横切） |
| `TILING_OVERLAP_FRAC` | 0.25（下限 128px） |
| `TILING_MAX_TILES` | 24（超出则块变高 1.25×） |
| `SEAM_TOUCH_PX` | 4 |
| `TILING_ENABLED_BY_DEFAULT` | **`false`** |

跨块合并三阶段（`tiling.rs:149-200`）：

1. 被接缝切断的框，若别处有完整副本（IoU ≥ 0.5）→ 丢弃半截
2. IoU ≥ 0.5 或 containment ≥ 0.85 → 高分胜
3. 相邻块 + 至少一侧贴缝（4px 容差）+ 垂直相接 + **水平重叠 ≥ 0.6×较窄者** → union 拼接

**关闭原因**（`tiling.rs:34-35`）：

> OFF BY DEFAULT UNTIL THE OWNER REVIEWS THE A/B (ADR-003): ON 77 TALL FIXTURES TILING FOUND 11 NEW BOXES BUT LOST 8 AND COST 2 TO 3 TIMES THE DETECTOR TIME.

净收益为负（+11 / −8，耗时 2–3×）。长图问题最终只靠 reslice 单点解决。

---

## 4. 检测

### 4.1 后端与签名路由

`src/ml/detect/detector.rs:55-72` 用 **ONNX 张量签名做后端路由**，没有配置开关：

```rust
let is_rtdetr = session.inputs().iter().any(|i| i.name() == "orig_target_sizes");
if is_rtdetr { return Ok(Self { engine: DetectorEngine::RtDetr(...), ... }); }

let is_rfdetr = session.inputs().iter().any(|i| i.name() == "input")
    && session.outputs().iter().any(|o| o.name() == "dets")
    && session.outputs().iter().any(|o| o.name() == "labels");
if is_rfdetr { return Ok(Self { engine: DetectorEngine::RfDetr(...), ... }); }

anyhow::bail!("UNSUPPORTED COMIC DETECTOR MODEL SIGNATURE: EXPECTED RT-DETR OR RF-DETR");
```

代价是无法在运行时同时挂载两个模型。换 `.onnx` 文件即自动切换。

### 4.2 RF-DETR Seg 2XL（默认后端）

`src/ml/detect/rfdetr.rs`

| 项 | 值 | 位置 |
| --- | --- | --- |
| 输入 | `[1,3,768,768]`，**暴力拉伸，无 letterbox** | `rfdetr.rs:14,107-113` |
| 输出 1 | `dets [1,Q,4]` = 归一化 cx,cy,w,h | `rfdetr.rs:145` |
| 输出 2 | `labels [1,Q,C]` = **logits（未 sigmoid）** | `rfdetr.rs:147` |
| 类别 | `0=Text, 1=Onomatopoeia, 2=Bubble, 3=Panel` | `rfdetr.rs:26-32` |
| 阈值 | Text/ono 0.25，Bubble/Panel 0.50 | `rfdetr.rs:15-18` |
| 归一化 | ImageNet mean/std | `rfdetr.rs:21-22` |
| NMS | **无**（DETR 系 NMS-free），sigmoid + 分类别阈值直接取 | `rfdetr.rs:232-243` |
| 边界不变式 | `x+w <= page_w`、`y+h <= page_h` | `rfdetr.rs:161-183` |

`Panel` 类被映射成 Bubble 塞进 `all_detections`（`rfdetr.rs:245`）——**分格可作为宏容器**。

边界不变式有配套回归测试（`tests/detector_ab.rs:112-118`），明确"只允许把 x/y 移到 0 或缩小 w/h，其他任何变化都标记为违规"。**把不变式写成测试比写在注释里可靠。**

### 4.3 其它后端

- **RT-DETR**（`rtdetr.rs`）：输入 `[1,3,1024,1024]` + `orig_target_sizes [1,2]`（注意是 h,w 顺序），仅 `/255.0` 无 mean/std，类别 `0=Bubble, 1=TextBubble, 2=TextFree`，阈值 0.15/0.20/0.25。**仓库里没有 RT-DETR 权重**，只有代码。
- **DBNet**（`dbnet.rs`，全文 66 行）：**不是独立后端**，是 PaddleOCR `Boxes_from_bitmap` 的 Rust 移植，被 OCR 引擎调用。参数 `thresh 0.3, box_thresh 0.5, unclip_ratio 1.6, max_candidates 1000, min_side 3`（`ocr/engine.rs:958-969`）。PP-OCRv6 det 导出时距离场分支已剪掉，只剩概率图。

### 4.4 权重管理

`models/manifest.tsv` 是唯一权威清单（`file / size_bytes / sha256 / url`）。`build.rs:406-445` 在构建期逐行校验字节数 + SHA-256，不匹配直接 `panic`（逃生口 `XIANSCAN_SKIP_MODEL_HASH=1`，仅本地实验，CI 不设）。

**权重完整性由构建系统保证，而不是运行时。** 值得抄。

`src/ml/embedded_models.rs` 的 7 个 `include_bytes!` 全部由 `--features embed-models` 门控。

### 4.5 竖排判定：看框内 OCR 行，不看框形状

`src/pipeline/region_builder/builder.rs:362-423`

```rust
let is_decidedly_horizontal_container = box_rect.w >= (box_rect.h as f32 * 1.35) as i32
    || matched_bubble.map_or(false, |b| b.w >= (b.h as f32 * 1.40) as i32);

is_container_vert = if is_decidedly_horizontal_container && h_count > 0 {
    v_area > (h_area as f32 * 2.5) as i64 && v_count >= h_count * 2
} else if v_count > 0 && h_count > 0 { ... };
```

统计框内竖行/横行的**数量与面积占比**投票决定朝向。横排容器需要 2.5× 压倒性证据才翻转成竖排，避免单个异常行导致误判。

CJK 与非 CJK 用不同的竖直阈值（`builder.rs:377`）：

```rust
let is_vert = if is_cjk { lh >= lw || lh > (lw as f32 * 1.10) as i32 }
               else        { lh >  (lw as f32 * 1.25) as i32 };
```

### 4.6 倾角估计三级

1. **SFX 碎片聚类 + 线性回归倾角**（`grouping.rs:181-184`）—— 最小二乘斜率，限制在 1.5°~35°。**此函数导出但生产代码从未调用，是死代码。**
2. **行内角度中位数**（`builder.rs:743-752`）—— 生产实际使用，**用中位数而非均值抗离群**，把 `sin/cos` 传进聚类。
3. **墨迹级旋转卡壳**（`region_builder/geometry.rs:1049-1085`）—— 对暗墨像素（`lum<120`）直接跑 `get_mini_boxes` 求最小面积矩形。护栏：暗像素 <35 个不置信，>45% 说明抓到暗背景也拒绝。

### 4.7 领域启发式（漫画味很重）

| 启发式 | 判据 | 位置 |
| --- | --- | --- |
| 气泡归属覆盖率 | **以文本框面积为分母** ≥0.50，不用气泡面积（否则几乎恒为真） | `fusion.rs:318-327` |
| 同气泡多列 vs 双瓣气泡 | 右列是否以终止标点结尾 + 间隙是否 ≥26px | `analyzer.rs:654-667` |
| 单瓣气泡确认 | 归一化椭圆距离 `(dx/rx)²+(dy/ry)² ≤ 0.85` | `analyzer.rs:637` |
| 振假名（ruby）识别 | 宽 ≤55% + 字数 ≤4 + 纵向覆盖 ≥70% | `builder.rs:614-620` |
| 背景书法剔除 | 倾角 ≥4° + 无气泡容器 + 面积 >20000px² | `dedup.rs:663-682` |
| 巨型封面标题豁免 | `kw>=350 && h<=35 && w<=200` 不被抑制 | `grouping.rs:71` |
| 阅读顺序 | 行带聚类（`row_tolerance=0.5`），行内**只有日语 x 降序（右→左）**，其他 x 升序 | `grouping.rs:284-290` |

### 4.8 智能去重（`grouping.rs:7-100`）

`deduplicate_boxes(boxes, scores, 0.40)` 四类判据：宏容器替换（面积 ≥1.15× + 双向覆盖 ≥0.65×）、被容器吞掉（竖/横/多列容器三选一）、巨幅 banner 例外、常规重叠（IoU ≥0.40 或 overlap/min_area ≥0.70）。

这是"检测器同时输出整个气泡和气泡里每一行"的消歧核心。

### 4.9 5 盒分离模型（最推荐照搬）

`src/ml/schemas.rs:35-48`

```rust
pub box_: BoxRect,                 // 文本紧致边界（去重/阅读顺序/朝向判定）
pub ocr_box: Option<BoxRect>,      // 原始 OCR 框（膨胀后与 box_ 分离，供 inspect）
pub inpaint_box: Option<BoxRect>,  // 擦除用：box_ 等距外扩 3%
pub typeset_box: Option<BoxRect>,  // 排版用：气泡内阻尼扩张 + 居中
pub bubble_box / bubble_polygon,  // 气泡外壳
pub carrier_box: Option<BoxRect>,  // 去掉尾巴后的腔体
```

**每个下游消费者只拿自己需要的那个 box。** 这让"擦太狠破坏气泡边框"和"排版空间不够"两个问题彻底解耦。强推。

### 4.10 排版空间怎么来：阻尼扩张（不看文本长度）

`src/pipeline/region_builder/expansion.rs`

**(a) carrier 交叉验证**（`:166-261`）

优先用形态学开运算从像素切尾巴（切掉厚度 <25px 的尾巴），无像素时回退几何近似（margin 偏斜 ≥1.30× 且差 ≥18px 判尾），**但必须**经 `valid_tail_cut_carrier` 验证：单边深切 + 对边几乎不动（`trim_right >= 2.2*trim_left`）+ 比例不塌陷 + 非画布边缘。

宁可不用尾巴检测，也不愿因偏心文本误切半个气球。

**(b) bubble_core 内缩 12%**（`:64-76`），clamp 8–48px，防大气泡内缩穿框：

```rust
const BUBBLE_INSET_FRAC: f32 = 0.12;
const BUBBLE_INSET_MIN: i32 = 8;
const BUBBLE_INSET_MAX: i32 = 48;
```

**(c) 阻尼扩张**（`:419-464`）

```rust
let raw_scale = max_safe_half / half;
if usable >= bw * MIN_UNUSED_RATIO /*0.10*/ && raw_scale >= MIN_SCALE /*1.10*/ {
    let damping = if is_narrow_vertical { 1.0 } else { EXPANSION_SLACK_DAMPING /*0.50*/ };
    let cap     = if is_narrow_vertical { 2.00 } else { 1.30 };
    let final_scale = (1.0 + (raw_scale - 1.0) * damping).min(cap).min(raw_scale);
}
```

**PHASE 1 只从原始几何算所有 target，PHASE 2 统一应用**——兄弟框读未缩放值，避免顺序依赖导致的不对称扩张（`expansion.rs:294-296`）。**空间不够就完全不扩张**，cramped bubble 不被扰动。

**(d) 底部填充补偿**（`:579-633`）

独占气泡 + 填充率 ≤0.35 + 短文本（≤3 字符）时，把 `typeset_box.h` 撑到最多气泡 75% 高，然后居中。居中容差 ±4px，超出就扩张/夹紧，**绝不裁字**。

`inpaint_box` 保持紧致：`expand_box(box_, 3%)` 后夹进气泡内芯（`:639-643`）。

---

## 5. OCR

### 5.1 模型与懒加载

| 用途 | 模型 | 字典 |
| --- | --- | --- |
| 行检测 | `PP-OCRv6_det_small.onnx` | — |
| 识别（默认 CJK+Latin） | `PP-OCRv6_rec_small.onnx` | `rapidocr_keys.json` |
| 韩 | `korean_mobile_v2.0_rec.onnx` | `korean_dict.txt` |
| 西里尔 | `cyrillic_mobile_v2.0_rec.onnx` | `cyrillic_dict.txt` |
| 泰 | `th_PP-OCRv5_mobile_rec.onnx` | `th_dict.txt` |

**懒加载**（`engine.rs:123-197`）：注册只存字节，真正建 session 推迟到首次使用，失败保留 pending 可重试，每语言只告警一次。zh/ja/en 工作负载零额外韩/西/泰模型 RSS。

### 5.2 单一连续目标尺度（最值钱的一条经验）

`src/ml/ocr/engine.rs:13-18`

```rust
// SINGLE CONTINUOUS DET TARGET FOR THE LONG IMAGE SIDE OF A FULL PAGE. EVERY FULL PAGE
// IS SCALED TO EXACTLY THIS LONG SIDE SO THE EFFECTIVE TEXT SCALE IS IDENTICAL ACROSS
// SOURCE RESOLUTIONS. THE FORMER THREE-TIER LADDER (960/1500/2000) JUMPED THE EFFECTIVE
// SCALE BY UP TO 33% AT THE 960 AND 1500 PX BOUNDARIES...
const OCR_DET_LIMIT_SIDE: f32 = 2000.0;
```

多档位阶梯（960/1500/2000）会让有效文字尺度在边界跳变 33%，导致"双瓣气泡"这类临界 case 随输入尺寸**翻转**。收敛为单一目标后行为稳定。尺寸对齐到 32 的倍数（`engine.rs:914-915`）。

**决策记录比代码本身更值得学。** 子图（tile / crop 重扫）反而保留阶梯（`engine.rs:24-32`），避免小图被过度放大产生碎片字形幻觉。

### 5.3 预处理

- **检测**：长边 → 2000，round 到 32 倍数，ImageNet mean/std，`FilterType::Triangle`
- **识别**：目标高固定 48，宽 clamp(16, 2048)，DirectML 时向上取整到 128 倍数；**BGR** 通道顺序写入 NCHW，`(x/255-0.5)/0.5`
- **全流程无二值化**，只有 32 倍数 padding + 按平均亮度自适应填充色（`mean_lum < 128 ? 黑 : 白`）
- **反色兜底**：首次识别为空时整图 `invert` 后重跑一次（针对浅字深底招牌）

### 5.4 竖排三级策略：不靠旋转

`src/ml/ocr/engine.rs:244-282`

```
h >= 1.3w 时：
  1) 投影竖切 → 横向条带（vertical_to_upright_horizontal_strip）
     - 估字数 round(h/w)，理想字高 h/est，在 ±ideal_h/3 窗口内找投影谷底切点
     - 每个字块横向拼成横条，避免 90° 旋转的字形拉伸
     - 命中汉字可越过分数阈值直接接受（0.60）
  2) rot270 兜底
  3) rot90 兜底
```

竖排路径**单独串行、不进批处理**（`engine.rs:1014-1022` 注释：对 TBRL 漫画是承重路径）。

横排段落切行（`slicing.rs:101-280`）含粘连行再切：行高 ≥1.8× 中位行高且 ≥45px 时，在中间 50% 找投影最小值切一刀，两侧各需 ≥12px。

### 5.5 条漫第二遍扫描

`engine.rs:1065-1117`，`h >= 960` 时用 960 高 / 760 步长（200px 重叠）滑窗：

- 合并：IoU ≥ 0.30 视为同一行
- 替换：有 CJK 字符 / 分数高 0.05 / CJK 且 ≥0.70 而原 <0.70
- 丢弃：`TILE_LINE_MIN_CN 0.50` / `TILE_LINE_MIN_OTHER 0.70`

### 5.6 阈值注册表 —— 极高性价比的收益点

`src/ml/ocr/score_thresholds.rs`（16KB）用 `thresholds!` 宏把每个绝对阈值登记成记录：`名称 / legacy值 / 尺度 / 比较符 / 使用站点 / 可达性状态`，并有 `computed_status()` 判定在量程内是永真/永假/可达。

**当前状态很有警示意义**：所有条目都写在 legacy sigmoid 尺度上（恒在 0.5~0.7311），校准尺度（`softmax_mean_v1`，∈[0,1]）无人读取，导致 registry 里明确标出一批**当前无效**的阈值：

- `NeverTrue`（永假 = 过滤形同虚设）：`TILE_LINE_MIN_CN 0.50`、`PRE_FUSION_JUNK_FLOOR 0.50`、`FALLBACK_JUNK_FLOOR 0.50`、`BUBBLE_COMPLETE_MIN 0.75`、`FULLPAGE_HIGH_QUALITY_MIN 0.78`、`FILTER_VERT_NARRATION_MIN 0.75`
- `AlwaysTrue`（永真 = 无条件通过）：`FUSED_GIANT_ART_MAX 0.75`、`CROP_ALNUM_NOISE_MAX 0.85`、`TINY_RESCUE_MIN_LATIN 0.85`、`FILTER_MARGIN_NOISE_MAX 0.75`

做一次这种"阈值体检"就能发现隐藏的过滤失效。

双尺度并存的迁移手法也值得学：`Prob` 变体用 `LazyLock` 存 `logit(literal)`，`prob_or_derived()` 在旧数据无 `prob` 时反推，迁移期不改比较运算符。

### 5.7 检测 × OCR 双向互补 + 相对边际择优

`src/pipeline/fusion.rs:305-690` 裁片救援是融合核心。

**匹配**（`:369-376`）：中心落在框内 / IoU ≥0.20 / 行被框包含 ≥0.50 / 框被覆盖 ≥0.20，且长宽比方向兼容。

**触发重识别**（`:382-397`）：`is_wider || is_taller || is_missing_lines || is_low_conf_or_degenerate`（`RESCUE_LOW_CONF_MAX 0.68`、单字符且框 ≥35px、非拉丁源被识别成拉丁）。

**回退链**（`:409-422`）：`recognize_crop(源语言) → recognize_line(源语言) → recognize_crop(无语言)`。

**择优，不是投票**（`:530-537`）：

```rust
let is_better = !is_excessive_multiline_bleed && !is_disconnected_crop_rows && (
    (!clean_c.contains('\n') || rl.text.contains('\n') || is_multiline_cb || clean_cjk > rl_cjk) && (
        clean_chars  > rl_chars
        || clean_cjk > rl_cjk
        || (clean_c.contains('…') && !rl.text.contains('…'))
        || (clean_chars == rl_chars && line_res.score > rl.score + CROP_REPLACE_MARGIN /*0.05*/));
```

**分数融合 = `max()`**，不是加权/平均（`:572-577`）。

**"断连行"守卫**（`:447-527`）非常实用，防止把跨气泡区域焊成巨型蒙版：

- 行间隙 ≥ `max(0.75×min_row_h, 25)`
- 并集面积 ≥ 3× Σ各行面积
- 存在占 crop ≥30% 面积且 ≤2 字的巨型笔画行
- SFX 与 ≥3 字句子共存
- 字符尺寸比 ≥2.5 或面积比 ≥4

### 5.8 "拒识"用几何判据而非提高分数门槛

`fusion.rs:67-159` 六条：巨型画作（w ≥60% 页宽且 h ≥120）、水印行、孤立噪笔画、倾角 ≥12° 且低分、非拉丁源中倾角 ≥10° 的纯拉丁行、贴边（x ≤5）、细条 sliver（h ≤13/25 且与正常行重叠 ≥50%/60%）。

与置信度组合使用比单纯提门槛有效得多。

### 5.9 文本后处理

- **语种过滤**（`lang.rs:251-269`）：按源语言剔除异种文字；西里尔源额外跑同形字归一（大小写感知：多数小写时 `B→ь`，大写时 `B→В`）
- **通用伪影清理**（`text_clean.rs:288-351`）：气泡尾巴数字噪声（`0oO23589` 且长度 ≤8）、指针字符（`Λ ^ ▲ ▼ △ ▽ ∧ ∨ ∠`）、省略号后的行尾圆点、尾部 `/\`、被识别成括号的气泡边框弧线（保留 `【】《》〔〕`）、`一./1./|./l./I.` → `！`
- **水印碎片剥离 + 几何等比裁剪**（`fusion.rs:257-277`）—— 思路很巧：文字层面砍掉首尾水印后，**把几何按字符比例等比裁掉**，上下边各用自己的原始宽度插值（处理梯形/旋转框）
- **行拼接**（`clustering.rs:469-606`）：CJK 同行内 `join("")` 不插空格，非 CJK `join(" ")`；CJK 行内含 `—` 且行间距 ≥80px 时补 `—`×10；竖排先按列去重碎片再 `join("\n")`

### 5.10 双置信度标度

`src/ml/ocr/confidence.rs`

- **legacy sigmoid**：`(1/(1+e^-p)) ∈ (0.5, 0.7310586]`
- **calibrated**：`softmax_mean_v1` = 每时间步 max softmax 概率的均值 ∈ [0,1]

聚合链：字符均值 → 行 → crop 取 max → region 取均值。`Region.confidence` 与 `Region.ocr_confidence` 并存，DB 用 `confScale` 列（0/1）告知前端。

---

## 6. 翻译

`PROMPT_VERSION = 'v24'`（`translate.ts:67`），参与缓存 key，改 prompt 自动全量失效。

### 6.1 一页一请求 + 严格 1:1 编号

`web/src/lib/server/translate.ts:166-301`

一页所有可翻译气泡用 `r0/r1/...` 编号合成**一次** LLM 调用，payload：

```json
[{ "id": "r0", "text": "...", "pos": "top-right", "kind": "free_text", "vertical": true }]
```

返回 `{"translations":{...},"styles":{...},"newTerms":[...]}`。`pos` 是由框中心归一化坐标算出的位置标签（`top/mid/bottom` × `-left/-right`，`dialogue-tracker.ts:40-69`），帮模型重建气泡空间关系。

**没有逐条重试降级。** 降级链是：整页重试 ×3 → `max_tokens` 翻倍升级（上限 65536）→ 部分成功容忍（只在全部成功且无 error 时写缓存）→ 标点兜底 → SFX 词典兜底 → 页面级重试 ×3 → 章节级重试 ×3。

**预分类**：每个 region 走 `classifyRegionForTranslation`，空/纯标点/水印/英文 SFX 本地定稿；**全页被预解决时零 LLM 调用**。

### 6.2 解析层五级降级

`web/src/lib/server/translate/parser.ts:90-219`

1. 剥 `<think>` 标签 → 剥 markdown 围栏 → 截取首 `{` 到末 `}` → `JSON.parse`
2. 兼容 `{translations:{...}}` 包裹、扁平 `{r0:"..."}`、`{r0:{text:"..."}}`
3. **正则 salvage**（先摘掉 `styles` 块，否则其 `"r0":"accent"` 会覆盖真实译文）
4. **编号回填**：精确 id → 原文当 key → 0-based → 1-based（`r0`/原文/`22357` 混合键都能回填）
5. artifact 清洗 + 大小写归一（1-2 词 ≤12 字符的孤立感叹 `BOOM!` 保留全大写）

**反幻觉问号**（`parser.ts:56-63`）：源以 `。`/`.` 结尾且无任何疑问词（`吗呢吧难道怎么为什么岂谁什么哪か까`）→ 把译文尾部 `?` 改回 `.`。

**退化检测**：译文长度 > `max(120, 源长×10)` 判为退化，删除该 region。

### 6.3 Prompt 分层

`web/src/lib/server/translate/prompts.ts`（35KB）

```
I.   通用不变量（9 条）
II.  <源语言> profile（ko/ja/zh/ru-uk-be/fr-es-de-it-pt/id-ms/th）
III. <目标语言> profile（en/ar/es/fr/it/pt/ru/uk/pl/de）
+ 书籍级本地化指令（books.customPrompt）
IV.  输出 schema
```

真实踩坑写成的显式规则：

- **代词锁定**：禁止单数 they/them 兜底；性别一旦确立严格锁定；怪物/非人实体用 it/its 绝不用 he/she；**1-on-1 场景后续评价必须保持第二人称**；省略宾物的攻击命令按上下文解析为敌人（用 it/him，不用 them）
- **跨气泡连续性**：一句话跨多个气泡时前一个加省略号、后一个小写开头且不重复主语连词、人称与语气完全一致
- **不要照抄源文换行**：重排成视觉平衡的倒金字塔/菱形布局，`\n` 只保留在段落/思绪/UI 列表之间
- **标点克制**：冒号只留给 RPG 属性面板、UI 标签、时间戳
- **accent 标记**：整段是招式名/技能/称号/章节标题卡 → `"styles": {"r3": "accent"}`
- **OCR 噪声恢复**：把 `Aaaargh!!` 这类尖叫字还原

目标语言 profile 也有硬规则：阿拉伯语**无中性 it**，语法性别跟随名词；`de` 名词大写 + Sie/du；西/法 T-V 呼应。

### 6.4 术语表：三层优先级 + append-only

- **三层**：系统 pack（7 主题包 × 20 语种 = 2660 组合，惰性生成 <1ms）< 用户 global < 书籍级
- **AI 只增不改**（`glossary.ts:635-679`）：source 已存在（alias 感知 + 大小写不敏感）就跳过；人工改过标 `status:'user'` 永不被覆写
- **append-only 不重排** → provider 的 prefix cache 命中率
- **匹配**（`glossary-match.ts`）：Aho-Corasick 精确（source 优先占位，alias 仅在未被占用时加入）+ 2-gram 倒排 + Damerau-Levenshtein 模糊（len≤5 距离 1，>5 距离 2）；拉丁/西里尔要求词边界，CJK 关闭词边界做子串匹配
- **绝不用英文/中文兜底**（`glossary-packs/index.ts:90-107`）：wrong-language 的术语比没有更糟
- **术语抽取接地校验**（`extraction.ts:121-129`）：source 必须逐字出现在原文（NFKC 归一后再 contains），否则丢弃，防止模型臆造
- **确定性 pin**（`extraction.ts:159-169`）：章节文本中出现 ≥2 次的多字术语自动锁定

### 6.5 跨气泡说话人一致性：双语滑动窗口

`dialogue-tracker.ts:113-148`，默认回溯 4 页（可跨章）：

```
=== DIALOGUE CONTEXT (Previous Pages) ===
Note: This is read-only background context to maintain consistent topic, speaker,
and pronoun flow. Do NOT translate or output entries for these context pages.
[Page 3 (Previous Context)]:
  - [Bubble | top-right] "原文" → "译文"
```

注入的是 **source→target 双语对照**（已翻页带译文），模型能看到自己之前的译法，术语/敬语/人称自然一致。比只注入原文有效得多。

### 6.6 每本书的 LLM 调用串行

`chapter-pipeline.ts:476-483`

```ts
// PER-BOOK SERIAL QUEUE: ONE LLM TRANSLATE AT A TIME WITHIN THIS BOOK, SO A TERM DISCOVERED ON PAGE N
// IS APPENDED TO `chapterTerms` BEFORE PAGE N+1 SNAPSHOTS IT. DIFFERENT BOOKS STILL OVERLAP.
```

第 N 页发现的术语在第 N+1 页翻译前已进术语表，零跨页竞态。不同书仍可并行。

### 6.7 本地能确定性处理的绝不吃 LLM

`translate/filter.ts:147-213` 预分类：空 / 纯标点（本地转目标语言标点，阿拉伯语转 `؟ ، ؛`）/ CJK 源里的短字母数字碎片 / 水印域名（20+ TLD）/ 已是英文的 SFX。

SFX 兜底是纯本地多语词典（zh 32 / ru 32 / ja 24 / ko 39 / fr 12 / es 9 条 → 英文 ALL-CAPS），但有策略约束（`translate.ts:275-284`）：

```ts
// THE SFX DICTIONARY IS ENGLISH: FOR OTHER TARGETS AN UNTRANSLATED REGION BEATS
// ENGLISH ON AN ARABIC PAGE (FEAT-007 ADR-010)
const sfxFallback = pair.targetLang.startsWith('en') ? getKnownSfxTranslation(...) : null;
```

### 6.8 推理抑制与 token 预算

`llm.ts:96-184` 跨 provider 映射思考抑制参数：Ollama `extra_body.think`、OpenRouter `extra_body.reasoning.exclude`、默认 `reasoning_effort: 'none'`。400 且错误含 `reasoning_effort` 时改 `low` 重发一次。

重试时 `max_tokens × 2^attempt`（上限 65536），下限按源文本长度 `max(4096, 源字符数×4+2048)`。

三层重试：调用级 ×3 → 页面级 ×3（`1000×1.5^n`）→ 章节级 ×3（`min(2000×2^(n-1), 15000)`）。

### 6.9 缓存 key

`sha256(regionId:text | 术语表指纹 | model | PROMPT_VERSION | 语言对 | providerSalt | customPrompt)`

术语表指纹**只含 prompt 相关字段**（source/target/gender/context/pinned/aliases），不含 category/status/firstChapterId。改 prompt 或术语表自动全量失效。

双层查找：章节锁外先查一次，**进串行链后用最新术语表再查一次**——等锁期间术语表可能已被前页扩充（`chapter-pipeline.ts:1039, 1105`）。

### 6.10 人工审校

- `regions.originalTarget` 永远保留 AI 原译供回退
- `role` / `roleSource('llm'|'glossary'|'user')` 记录来源
- 编辑后立即 `retypesetPage` 重排整页
- **无自动置信度阈值重译**，质量控制完全靠人工审校 + 缓存指纹间接保障

---

## 7. 擦除

### 7.1 LaMa 输入构造

`src/ml/inpaint/lama.rs:110-145`

```rust
let mut img_tensor  = vec![0.0_f32; 3 * padded_h * padded_w];   // NCHW
let mut mask_tensor = vec![0.0_f32;     padded_h * padded_w];   // 单通道
plane[row_offset + x] = raw_rgb[... ] as f32 / 255.0;           // 只 /255，无 mean/std
row_slice[x] = if raw_mask[... ] > 0 { 1.0 } else { 0.0 };       // 硬二值
```

- **双输入张量**（RGB + mask），不是 4 通道 concat
- **只 `/255.0` 无 ImageNet 归一化** → 这意味着 `lama.onnx` 是按 `[0,1]` 直喂重导出的变体，**换官方权重会直接崩**
- **只回写 mask 内像素**（`:78-87`），测试断言未 mask 像素 byte-identical（`tests/degenerate_inputs.rs:353-367`）

### 7.2 Mask 生成：行级矩形，不是字形级

- polygon 优先（≥3 点），box 仅兜底（再内缩 5%，clamp(2,8)/(2,6) 保护描边）
- 旋转文本（|angle| ≥1.5°）用整块文本行的**最小外接平行四边形**，padding 按字号缩放：`p = (font_scale*0.90).clamp(18,35)`、`v_pad = (font_scale*0.35).clamp(8,18)`
- 扫描线填充 + **3px 圆盘膨胀**（`geometry.rs:614`，欧氏结构元不是方形）
- **没有羽化、没有软概率**

### 7.3 规划器 / 执行器分离

`src/ml/inpaint/plan.rs` 是纯函数 `plan_inpaint(w, h, mask, mode) -> Vec<Pass{src, scale, own}>`，可被单元测试完整覆盖——`tests/degenerate_inputs.rs:306-331` 用 1000×20000 极端页断言三种模式都不超预算、不越界。`lama.rs` 只管按 pass 循环。

常量：

| 常量 | 值 |
| --- | --- |
| `LAMA_MAX_PIXELS` | 2048² ≈ 4.19 MP |
| `LAMA_MAX_SIDE` | 4096 |
| `LAMA_BAND_OVERLAP` | 64 |
| `LAMA_SCALED_TARGET` | 512 |
| `LAMA_PATCH_PAD` | 24 |

三模式：`patch`（默认，8 邻接连通域 BFS，每域一个 pass）/ `scaled`（长边缩到 512）/ `full`（整页，超预算分带）。

**保守降级链**：`full` 宽度 >65536px → 自动降级 `patch`；模式字符串无法识别 → `patch`；CUDA 失败 → sticky 记录 + 后续模型走 CPU；无模型 → 原图返回。每层都留 `tracing` 日志。

### 7.4 维度分桶 —— 投入产出比最高的一条

`src/ml/inpaint/lama.rs:96-108`

```rust
#[cfg(feature = "directml")]
let (padded_w, padded_h) = { let bw = (w as usize).div_ceil(64)*64; ... };  // 64px 分桶
#[cfg(not(feature = "directml"))]
let (padded_w, padded_h) = { let pad_w = (8 - (w % 8)) % 8; ... };          // 8px 分桶
```

`DEVELOPMENT.md:70`：**64px 分桶把 inpaint 从 7s 降到 0.11s（9 个 patch）**——避免 DirectX 12 PSO 重复编译。**任何用动态尺寸网络的地方都该做分桶。**

### 7.5 shrinkwrap：全项目最有算法的部分

名字有误导性。它**不是 mask 精修**，而是 LaMa **之后**的"纯白气泡内腔二次纯色回填"，清掉 LaMa 残留的灰尘/污渍/内部水印，同时保住气泡描边。

`src/ml/inpaint/shrinkwrap.rs:101-682`：

| 步骤 | 内容 |
| --- | --- |
| 0 背景色采样 | 7×7 种子邻域，剔暗字（lum≥180 && sat≤20），三通道独立中位数。阈值 `WHITE_BUBBLE_MIN_LUM 220` / `MAX_SAT 14` |
| 1 裁剪 | 外扩 6px 上下文 |
| 2 白地板 flood fill | 种子 5×5 搜 `score = lum - sat*2.0` 最优起点；4 邻域扩散，**限制在气泡框内**（防从不连续边框缝隙漏到外侧 gutter）；判据 `lum>=210 && sat<=14 && 与fill色通道差<=35` |
| 3 形态学闭运算 r=2 | 膨胀时不跨实心墨线（`lum>=110` 视为屏障），腐蚀无条件下压 |
| 4 **拓扑孔洞填充** | 从 crop 边界向内 BFS 标 `can_reach_edge`；**到不了边缘且不在屏障上 = 内腔**（文字/水印残留） |
| 5 描边保护 | `lum<195` 且能触达"外部世界"的暗像素 → `protected_stroke`，沿连通描边/速度线/阴影扩散，在白地板处停止 |
| 6 孤立描边二次豁免 | 未被保护的暗连通域，若不邻近 `clean_boxes` 判定为文字则豁免（四向 18px / 列对齐 ±28px / 行对齐 ±28px）→ 不翻译的省略号、另一叶气泡里的 `…` 会被保留 |
| 7 内腔装配 + 5 道闸门 | 见下 |
| 8 欧氏距离变换 | 两遍 8 邻域 chamfer（对角 1.4142），`1e6` 初始化，<0.2ms/bubble |
| 9 Hermite 羽化 | radius 2.5px，`alpha = 3t²-2t³`（smoothstep 保证 C1 连续）。**全项目唯一有软 alpha 的地方** |

**5 道闸门**（`:562-590`），任一不过就整块放弃：

```rust
if total_cavity_count == 0 || white_floor_count == 0 { return false; }
if white_floor_count as f32 / total_cavity_count as f32 < 0.80 { return false; }
if lum_spread > 12 || cav_mean_sat > 4.5 || cav_p90_sat > 8.0 || cav_mean_lum < 246.0 { return false; }
```

取舍写在注释里：

> BUBBLES WITH BACKGROUND ARTWORK BLEED-THROUGH HAVE HIGH LUMINANCE SPREAD OR SATURATION, AND MUST BE PRESERVED AS NEURAL INPAINTED ARTWORK INSTEAD OF BEING OVERWRITTEN WITH SOLID WHITE.

即**纯色气泡 → 纯白回填；渐变/网点/纹理气泡 → 完全交给 LaMa 神经重建**。"宁可不做，也不要做错"。

**"从图像外部 flood fill，到不了的区域就是内腔"** 这个思想比 Otsu/颜色聚类稳得多：天然处理渐变、网点、抗锯齿边缘。配合两遍 chamfer 距离变换 + smoothstep 羽化得到无硬边填充。

### 7.6 边缘 padding 的四层

| 层 | 值 |
| --- | --- |
| 前端百分比（UI 5 档） | 0% / 3% / 6% / 9% / 12%，默认 3% |
| 等向换算 | `ref_dim = min(w,h).max(max(w,h)*0.4)`，`pad = ref_dim * pct * 1.5` |
| mask 圆盘膨胀 | 固定 3px |
| LaMa pass 上下文 | 固定 24px |

各司其职，参数全部硬编码在单一位置便于调优。

### 7.7 多区域不打包

`lama.rs:64-88` 严格串行，每个连通域一次 `session.run`，batch 维恒为 1。相邻行间距 ≤6px（3+3）会合并成一个连通域。

---

## 8. 嵌字

### 8.1 渲染栈

**纯服务端 Skia**（`@napi-rs/canvas`），`canvas.encode('webp', 90)`。无 SVG、无浏览器 canvas、无 CSS 排版。前端的 `@font-face` 只是让 CSS 预览 ≈ Skia 输出。

### 8.2 字号：拒绝二分，线性降序扫描

`web/src/lib/server/typeset/layout.ts:669-673` 的注释是设计文档：

```ts
// LINEAR SCAN FROM THE CAP DOWNWARDS: DISCRETE TEXT REFLOW WRAPPING IS
// NOT MONOTONIC IN FONT SIZE (A PARAGRAPH MAY OVERFLOW A LINE AT SIZE N
// BUT WRAP INTO A SHORTER CLEAN LINE AT SIZE N+1). A BINARY SEARCH FALSELY
// PRUNES LARGER VALID SIZES WHEN A SINGLE INTERMEDIATE SIZE OVERFLOWS.
for (let mid = hi; mid >= lo; mid--) {
```

离散文本重排对字号**非单调**，二分会误剪掉更大的合法字号。用 `minTheoreticalLines` 剪枝 + 短行特殊 pass 控制 O(n·cap) 开销。

四个 pass：clean → tall-narrow floor → vertical-fill hyphenation → narrow-vertical 保护。

**竖长框的几何字号下限**（`:739-769`）专治"竖长气泡被宽度瓶颈压成 6px"：

```ts
const geometricCandidate = Math.min(effectiveCap, Math.max(MIN_FONT_SIZE,
    Math.round(maxW * 0.28), Math.round(Math.sqrt(maxW * maxH) * 0.10)));
```

触发条件 `aspectRatio >= 2.0`，`vertical tolerance = 1.0`（允许垂直溢出、严禁水平溢出）。

### 8.3 全页字号中位数基线

`web/src/lib/server/typeset.ts:229-274`

每个气泡独立 fit 会导致同页字号跳变。取全页对白字号**中位数**当基线，短句（≤2 词无感叹号）封顶 `baseline × 1.25`、地板 18px：

```ts
const isShortNonShout = text.split(/\s+/).length <= 2 && !/[!！]/.test(text);
cap = Math.min(cap, Math.max(18, Math.round(pageDialogueBaseline * 1.25)));
```

简单但极大提升"同一页像同一个人排的"观感。**条漫上尤其重要**（一页几十个气泡）。

### 8.4 断行 5 条分支

`layout.ts:202-388`

| 分支 | 触发 | 做法 |
| --- | --- | --- |
| 泰语 | `hasThai()` | `Intl.Segmenter('th')` 词典切分，词间**不加空格** |
| CJK | 无拉丁字母 | **逐字断行** |
| 拉丁 | 默认 | 空白分词 + 连字符复合词拆分 + 尾随标点分离 |
| 复杂脚本 | grapheme | `Intl.Segmenter` **只在字素簇边界断，绝不加连字符** |
| 阿拉伯语 | RTL 脚本 | 保持整词靠字号收缩；MIN=6 仍溢出才按字素簇兜底切 |

英文音节断字（`:38-116`）做了漫画化调校：非拉丁直接 `[]`、词长 <7 不断、32 前缀 / 34 后缀 / 双辅音 / VC-CV 五类规则、过滤 `p>=3 && len-p>=3`、**V-CV 兜底仅在其他规则全未命中时启用**、永不做音节连字符给非拉丁。

**段落硬断** 5 条规则（`:428-496`）：上行以 `:：)）】》!？؛` 结尾 / 句号+下行大写 / **独立标题**（`FLOOR 3` `RANK B` `SYSTEM MESSAGE` 等，≤4 词 ≤30 字符）/ 下行以结构前缀开头 / **独立人名**（≤3 词 ≤25 字符无连词）。注释明确反例：`"MYSTIC" / "HEAVEN" / "NOTICE HOW" / "JUST 2"` 不许硬断。

**没有真正的 CJK 避头尾（禁则）**。只有标点悬挂：句末标点行可超 `maxW × 1.05`，独立标点行可超额 15% 粘回上行。

### 8.5 字体：渲染探针判覆盖

`web/src/lib/server/typeset/coverage.ts:55-72`，零依赖跨平台：

```ts
export function probeFamilyScript(family: string, script: string): boolean {
	const [a, b] = PROBE_PAIRS[script];            // han: ['的人国','中文字'] ...
	ctx.font = `32px "${family.replace(/"/g,'')}"`;  // 只此一族，不带回退栈
	ctx.fillText(word, 4, 36);
	return !render(a).equals(render(b));            // 相同 ⇒ 都是豆腐块 ⇒ 不覆盖
}
```

对只有名字没有文件的系统字体，画两个不同短词比对像素是否相同。比维护一张 OS 字体表可靠得多。

`font-parser.ts`（575 行）是**零依赖 TTF/OTF/TTC 二进制解析器**：`splitFontCollection` 解 `.ttc`、cmap fmt4/fmt12 展开成区间、`CodepointSet` 二分查找（CJK 2 万+ 码点 O(log n)）、解析 `name`/`OS/2`/`fvar` 表。

**脚本字体链**（`script-fonts.ts:43-65`）：用户槽位 → 对话字体本身（若覆盖）→ bundled 或系统。`SYSTEM_BEFORE_BUNDLED = {han, kana, hangul}`（CJK 先系统保字形），其余先 bundled（保跨平台一致）。

**不支持 WOFF/WOFF2 上传**，只放行 `.ttf/.otf`。

### 8.6 描边与颜色

描边随字号与底色双向调节（`typeset.ts:307-323`）：

```ts
// 白字（深底）0.18em，黑字（浅底）0.10em，且都有绝对下限
isBlackOnLight ? Math.max(1.8, size*0.10) : Math.max(3.0, size*0.18)
```

**深色气泡上细描边容易"糊"，所以白字描边更粗。** 配合"描边带 shadow、fill 前清 shadow"（`fonts.ts:1360-1406`），`shadowBlur = max(2.5, size*0.18)`。

颜色用 sRGB→线性→Rec.709 亮度阈值 0.18 判黑白；背景采样取框内 **20%~80% 中心区**避开气泡描边。

**没有 WCAG 对比度计算**，只做二值黑白反转。

### 8.7 无重音降级（É→E）

`typeset.ts:96-104` + `web/src/lib/diacritics.ts:82-106`

```ts
/**
 * LATIN LETTERS WITH DIACRITICS THE FAMILY HAS NO GLYPH FOR ARE DRAWN PLAIN (É -> E), SO A WORD NEVER SWITCHES TO A FALLBACK
 * FONT MID-WORD (FEAT-011). A FAMILY WHOSE CODE POINTS ARE UNKNOWN KEEPS EVERY LETTER. RUNS AFTER CASING, BECAUSE A FONT
 * CAN HAVE É BUT NOT é. STORED TRANSLATIONS ARE NEVER CHANGED.
 */
```

逐字符处理、只碰 Latin 脚本（假名浊点/西里尔 Й/天城文/泰文/阿拉伯标记全保留）、码点集未知时保守放行。NFD + `LETTER_MAP`（`ß→ss æ→ae ø→o þ→th…`，在 NFD **之后**应用所以 `ǽ→æ→ae` 也覆盖）。**只在渲染时降级，绝不改存储译文。**

### 8.8 旋转与对齐

`typeset.ts:351-378`：`translate(cx, cy)` → `rotate(θ)`，**变换原点 = 气泡框中心**，旋转后仍按视觉高度垂直居中。`align` 默认 `'center'`，`'right'` 被强制映射为 `'center'`（防退化），`'left'` = start-align（RTL 时锚右边缘）。

角度来自 Rust 侧：优先取 OCR 行多边形角度的**中位数**；气泡内 <5° 且框角度 ≈0 → 归零；短韩文 <10° → 归零；竖排容器 → 归零；框角度 0 且不在气泡内 → 从暗墨像素用旋转卡壳估计。

RTL 一行 = 一个 run，一次 `fillText`，把 bidi + Arabic shaping 全交给 Skia（`fonts.ts:1352-1376`）。方向检测用**多数派**而非 Unicode first-strong（`web/src/lib/text-direction.ts:28-37`），因为以拉丁名开头的阿拉伯语句子会被 first-strong 误判。

### 8.9 区域去碰撞

`web/src/lib/server/typeset/decollision.ts:4-53`：O(n²) 两两比较，重叠 >50% 视为同一气泡放弃；否则按**重叠较小的轴**拆分，各承担一半 + 2px margin，每边最小 10px。

### 8.10 明确未实现

`letterSpacing`、`autoFit`（只在 zod schema）、**多行竖排**（`vertical` 字段从 Rust 传到 TS 但 `typeset.ts` 从未读取）、CJK 避头尾、文字沿路径排布、PNG/SVG 输出、渲染结果缓存、WCAG 对比度强制、移动端 DPI 感知字号下限。

---

## 9. 编排与运行时

### 9.1 并发层次

| 层 | 机制 | 位置 |
| --- | --- | --- |
| HTTP | tokio multi_thread + `run_blocking()` 丢阻塞池 | `router.rs:283` |
| 引擎锁 | 每模型一把 `Mutex`，锁序 `detector→ocr→inpainter` | `shared.rs:19-38` |
| detector/OCR | `std::thread::scope` 并行 | `fusion.rs:214-251` |
| detector/OCR（CPU） | **故意串行** | `fusion.rs:178` |
| rayon | 行画像 / 张量填充 / WebP 编码，全是数据并行 | 多处 |
| 页并发 | `PIPELINE_PAGE_CONCURRENCY` 默认 3 | `chapter-pipeline.ts:143` |
| 翻译 | 每本书 promise 链串行 | `chapter-pipeline.ts:476-483` |

CPU 上 detector 与 OCR 串行的注释：

> AVOIDS 16-THREAD THRASHING ACROSS 8 CORES

### 9.2 设备抽象

`src/ml/device.rs`（54KB）

- 优先级：env `MT_DEVICE` 覆盖 → 自动探测 CUDA → CoreML → DirectML → CPU
- **失败是"粘性"的**：一旦 `record_gpu_failure(CUDA)`，后续 session 直接走 CPU，直到用户换设备
- GPU 枚举按平台多路径（`nvidia-smi` / `/sys/bus/pci` / `drm` / `wmic`），全部经 `run_with_timeout(3s)` 防驱动挂死，结果缓存 15s
- CPU 线程 `num_cpus().min(8).max(1)`，env `ONNX_THREADS` 可覆盖
- CUDA 显存按模型 tag + 实测 VRAM 三档：detector 8GB / lama 4GB / ocr 1GB（@≥14GB 显存）
- CUDA arena 用 `NextPowerOfTwo`——`SameAsRequested` 会让模型权重在加载期吃满 arena 预算导致推理期 OOM（`device.rs:1023-1030` 注释）
- 全局分配器 mimalloc；空闲 `SetProcessWorkingSetSize` / `malloc_trim(0)`
- **Windows 上 OCR 走 CPU，检测与 inpaint 走 GPU**

### 9.3 增量与断点续跑

4 道闸：

1. `status='done'` 跳过
2. 启动时残留 `processing` 重置为 `pending`
3. `pageIds` 支持只跑指定页
4. 4 个 `*Rev` 单调修订号（`cleanedRev`/`outputRev`/`annotatedRev`/`originalRev`），URL 带 `?rev=N` 实现不可变缓存

失败重试 `1000 × 1.5^attempt`（上限 3 次）；一页失败只标自己 error，作业继续。

### 9.4 编排的两个亮点

- **阶段化流式**：谁先 OCR 完谁先翻译，不等其他页；clean + typeset 不上链，与下一页翻译重叠（`chapter-pipeline.ts:1229` `Promise.all([inpaintTask, translateTask])`）
- **原始 prompt/响应留档**：`pages.llmPrompt` / `pages.llmResponse` 存完整请求响应，调试时能直接看模型见了什么

---

## 10. 持久化

```
books (text id PK)
  │ 1:N cascade
  ├─► chapters (int id, uuid UNIQUE, book_id FK, seq, status, resliced, …)
  │      │ UNIQUE(book_id, seq)
  │      │ 1:N cascade
  │      └─► pages (seq, file_path, width, height, status,
  │                cleaned_path, output_path, annotated_path,
  │                cleaned_rev, output_rev, annotated_rev, original_rev,
  │                panels, llm_prompt, llm_response, ocr_stats, error)
  │             │ UNIQUE(chapter_id, seq)
  │             │ 1:N cascade
  │             ├─► regions (seq, box JSON, inpaint_box, typeset_box, polygon,
  │             │            text_source, text_target, original_target, status,
  │             │            conf, conf_scale, role, role_source)
  │             └─► translations (cache_key, content_target JSON, model,
  │                           prompt_tokens, cached_tokens, completion_tokens)
  │                           UNIQUE(page_id, cache_key)
  ├─► glossary (scope: global|book, book_id FK, src/tgt lang, source, target,
  │             gender, context, category, pinned, status, aliases, first_chapter_id)
  │             两个部分唯一索引 + scope 完整性 CHECK
  ├─► reading_history
  └─► aiProviders / appSettings(kv) / customFonts / customFontFiles
ai_usage (kind: extract|title|term|repair, page_id FK nullable, tokens…)
```

**关联方式**：全部通过 `pages.id` 外键 + 相对路径字符串，没有二进制入库。

- 原图 → 页：`pages.file_path = "uploads/<chapterId>/<uuid>.webp"`
- 页 → 区域：`regions.page_id` + `regions.seq`（阅读顺序）
- 区域几何：`regions.box` 是 **JSON 字符串**，内含全部 6 个 box + `centroid/kind/angle/vertical`
- 成品：`pages.output_path = "output/<chapterId>/<seq>.webp"` + `output_rev`

磁盘目录（`DATA_ROOT`）：

```
xianscan.db / -wal / -shm
ml-secret
uploads/<chapterId>/<uuid>.webp      ← 原图（reslice 后重写）
clean/<chapterId>/<seq>.webp         ← 擦除后
output/<chapterId>/<seq>.webp        ← 最终成品
annotated/<chapterId>/<seq>.webp     ← OCR 标注预览（完成后删除）
covers/<bookId>.jpg
cache/{thumbs,covers,translate}/
```

导出：`fflate.zipSync`，目录 `{章节标题}/{seq.padStart(3,'0')}.webp`（Mihon/Tachiyomi 本地源格式），图片 level 0 store，缺失页写 `MISSING_PAGES.txt` + `x-missing-pages` 响应头。

---

## 11. 借鉴清单

### 11.1 强烈建议直接照搬

| # | 做法 | XianScan 位置 | 为什么值得 |
| --- | --- | --- | --- |
| 1 | **5 盒分离模型** | `schemas.rs:35-48` | 下游各取所需，擦除与排版彻底解耦 |
| 2 | **reslice 三级递进切点搜索 + 上下净空校验** | `reslice.rs:181-201, 552-780` | "行方差低不足以证明可切"是硬经验 |
| 3 | **禁区检测用轻量 OCR 而非主检测器** | `reslice.rs:237-295` | 25-35ms vs 136MB 模型，长图上数量级差距 |
| 4 | **取消用 run_id 而非 bool** | `reslice.rs:26-32` | 陈旧取消永不会杀掉新任务 |
| 5 | **规划器 / 执行器分离** | `inpaint/plan.rs` 全体 | 纯函数可完整单测（含 1000×20000 极端输入） |
| 6 | **维度分桶** | `inpaint/lama.rs:96-108` | DirectML 7s → 0.11s |
| 7 | **只回写 mask 内像素 + 测试断言其余 byte-identical** | `lama.rs:78-87` | 擦除幂等且局部 |
| 8 | **拓扑孔洞提取**（从外部 flood fill，到不了的就是内腔） | `shrinkwrap.rs:285-331` | 比 Otsu/颜色聚类稳，天然处理渐变与抗锯齿 |
| 9 | **多道统计闸门后整块放弃** | `shrinkwrap.rs:562-590` | "宁可不做，也不要做错" |
| 10 | **阈值全外置 + 可达性体检** | `ocr/score_thresholds.rs` | 能发现永假/永真的失效过滤 |
| 11 | **竖排判定看框内 OCR 行朝向投票** | `builder.rs:362-423` | 不看框形状；CJK 阈值 1.10 宽于非 CJK 1.25 |
| 12 | **覆盖率以文本框面积为分母** | `fusion.rs:318-327` | 用气泡面积几乎恒为真 |
| 13 | **全页字号中位数基线** | `typeset.ts:229-274` | 简单但极大提升整页一致性 |
| 14 | **拒绝二分、线性扫描字号 + 注释写清 why** | `layout.ts:669-673` | 离散重排对字号非单调 |
| 15 | **渲染探针判字体覆盖** | `coverage.ts:55-72` | 零依赖跨平台，比维护 OS 字体表可靠 |
| 16 | **cmap 驱动无重音降级（只渲染不改数据）** | `typeset.ts:96-104` | 避免单词中途跳字体 |
| 17 | **水印碎片剥离后按字符比例等比裁剪多边形** | `fusion.rs:257-277` | 文字层与几何层联动 |
| 18 | **断连行守卫** | `fusion.rs:447-527` | 防止跨气泡区域被焊成巨型蒙版 |
| 19 | **解析五级降级 + 编号回填** | `translate/parser.ts:90-219` | LLM 结构化输出不稳定性最实用的兜底 |
| 20 | **术语表 append-only + 双语滑动窗口** | `glossary.ts`, `dialogue-tracker.ts:113-148` | prefix cache 命中 + 跨页人称一致 |
| 21 | **每本书 LLM 串行链** | `chapter-pipeline.ts:476-483` | 术语跨页零竞态 |
| 22 | **fit 与 draw 共享同一套 run 拆分** | `accent.ts:105-136` | 避免"测量用 A 拆分、绘制用 B 拆分" |
| 23 | **carrier 交叉验证** | `expansion.rs:166-261` | 宁可不用尾巴检测，也不误切半个气球 |
| 24 | **质心锚定不变式 + 两阶段阻尼扩张** | `expansion.rs:419-468` | 空间不够就完全不扩张 |
| 25 | **权重完整性由构建系统保证** | `build.rs:406-445` | 校验字节数 + SHA-256，不匹配 panic |
| 26 | **边界不变式写成测试** | `tests/detector_ab.rs:112-118` | 比写在注释里可靠 |
| 27 | **`*Rev` 单调修订号 + 目标页集合** | `schema.ts:111-120` | 不可变缓存 + 增量重跑 |
| 28 | **两阶段解码：先 probe header 再 decode，整批原子** | `intake.rs:96-182` | `NOTHING IS DECODED UNLESS THE WHOLE SET FITS` |

### 11.2 建议不要照抄（XianScan 的坑）

| # | 问题 | 位置 | 建议 |
| --- | --- | --- | --- |
| 1 | **检测器暴力拉伸 768×768，无 letterbox 无 padding** | `rfdetr.rs:107-113` | 800×2000 页纵向压缩 2.6 倍，小字必然丢失。改用 **letterbox 保比例 + 多尺度分块 + 跨块 NMS**，比继续在合并启发式上打补丁更直接 |
| 2 | **3% 擦除 margin 配置项实际不生效** | `chapter-pipeline.ts:819` 传 `inpaint_box`，但 `cleaner.rs:25-30` 优先用 polygon | 有 polygon 时 3% 被绕过，真正生效的是 `text_polygon` padding + 3px dilation。**配置项存在但不生效是隐蔽 bug** |
| 3 | **`vertical` 字段贯通全链路但排版端从未消费** | `typeset.ts` 从不读 `r.vertical` | 日漫/韩漫竖排译文目前被横排渲染 |
| 4 | **纯标点/纯省略号气泡的 6 道闸门数据耦合脆弱** | `shrinkwrap.rs` | 白色圆点气泡（`lum>240, sat<5`）会被整块抹白 |
| 5 | **`is_solid_background_patch` 是死代码** | `patch.rs:71-135` | 纯色检测 + 返回均值色，全仓库无调用 |
| 6 | **LaMa 权重只 `/255.0` 无 ImageNet 归一化** | `lama.rs:110-145` | 换官方权重会直接崩，代码里没写这个约束 |
| 7 | **无 LaMa 失败兜底**（纯色/模糊填充） | `cleaner.rs:51-55` | 无模型时直接返回原图；有模型但输出质量差时无兜底 |
| 8 | **无 warm-up、无推理结果缓存、单 Session 串行** | `shared.rs:25` | 所有 clean 请求排队；首次推理含 kernel autotune 延迟 |
| 9 | **RT-DETR 只有代码没有权重；`cluster_adjacent_sfx_boxes` 死代码** | `rtdetr.rs`, `grouping.rs` | 分支代码是维护负担 |
| 10 | **`MIN_FONT_SIZE = 6px` 是图片像素单位** | `layout.ts:11` | 1600px 切片在 390px 手机上满屏 → 6px 缩到 1.5 CSS px，**完全不可读**。可读性下限需与移动端显示尺寸挂钩 |
| 11 | **划词检测切块后不融合** | `plan.rs:59-80` own 中点归属，零 blending | 理论上仍有 64px 硬接缝，缩放模式下更明显 |
| 12 | **条漫阅读顺序只有日语 RTL** | `grouping.rs:284-290` | 阿拉伯语/希伯来语条漫需要扩展 |
| 13 | **不区分跨页气泡** | 全局 | reslice 反而是为了**避免**切断气泡而设计；无跨双页文本合并 |

---

## 附录 A：关键文件索引

### Rust 侧

| 职责 | 路径 |
| --- | --- |
| 流水线入口 / 锁 / 模型加载 | `src/pipeline/engine.rs` |
| 每模型锁 + 重载 | `src/pipeline/shared.rs` |
| STAGE2/3 编排（80KB） | `src/pipeline/analyzer.rs` |
| 检测 × OCR 融合（40KB） | `src/pipeline/fusion.rs` |
| 图像修补 | `src/pipeline/cleaner.rs` |
| 区域组装主控（71KB） | `src/pipeline/region_builder/builder.rs` |
| 气泡内阻尼扩张 | `src/pipeline/region_builder/expansion.rs` |
| 一句话聚类 | `src/pipeline/region_builder/clustering.rs` |
| 区域去重合并 | `src/pipeline/region_builder/dedup.rs` |
| 伪影过滤 | `src/pipeline/region_builder/filter.rs` |
| 裁片精修 | `src/pipeline/region_builder/refine.rs` |
| 像素级包络 / 角度工具 | `src/pipeline/region_builder/geometry.rs` |
| 长图重切（39KB） | `src/ml/reslice.rs` |
| 输入限额 / 两阶段解码 | `src/ml/intake.rs` |
| 核心数据结构 | `src/ml/schemas.rs` |
| 设备抽象（54KB） | `src/ml/device.rs` |
| CV 几何库 | `src/ml/geometry.rs` |
| 长图 tile 检测 | `src/ml/detect/tiling.rs` |
| 检测后端路由 | `src/ml/detect/detector.rs` |
| RF-DETR / RT-DETR | `src/ml/detect/rfdetr.rs` / `rtdetr.rs` |
| DB 后处理 | `src/ml/detect/dbnet.rs` |
| 框合并 / 阅读顺序 | `src/ml/detect/grouping.rs` |
| 文本串清洗（49KB） | `src/ml/detect/text_clean.rs` |
| 语系正则与归一 | `src/ml/detect/lang.rs` |
| OCR 引擎（52KB） | `src/ml/ocr/engine.rs` |
| 行切片 | `src/ml/ocr/slicing.rs` |
| CTC 解码 | `src/ml/ocr/decode.rs` |
| 双尺度置信度 | `src/ml/ocr/confidence.rs` |
| 阈值注册表 | `src/ml/ocr/score_thresholds.rs` |
| LaMa 执行 | `src/ml/inpaint/lama.rs` |
| 修补规划 | `src/ml/inpaint/plan.rs` |
| mask 构建 / 连通域 | `src/ml/inpaint/patch.rs` |
| 白气泡内腔回填 | `src/ml/inpaint/shrinkwrap.rs` |
| HTTP 路由 | `src/server/router.rs` |
| 权重清单 | `models/manifest.tsv` |

### TypeScript 侧

| 职责 | 路径 |
| --- | --- |
| 章节流水线（57KB） | `web/src/lib/server/chapter-pipeline.ts` |
| sidecar 客户端 | `web/src/lib/server/pipeline-client.ts` |
| 重切编排 | `web/src/lib/server/chapters/reslice.ts` |
| 批量调度 | `web/src/lib/server/batch-service.ts` |
| 嵌字主渲染器 | `web/src/lib/server/typeset.ts` |
| 断行 / 字号 / 断字 | `web/src/lib/server/typeset/layout.ts` |
| 字体注册 / run 拆分 | `web/src/lib/server/typeset/fonts.ts` |
| TTF/OTF/TTC 解析 | `web/src/lib/server/typeset/font-parser.ts` |
| 字体覆盖探针 | `web/src/lib/server/typeset/coverage.ts` |
| 脚本字体链 | `web/src/lib/server/typeset/script-fonts.ts` |
| 强调字体 | `web/src/lib/server/typeset/accent.ts` |
| 亮度 / 字色 | `web/src/lib/server/typeset/color.ts` |
| 区域去碰撞 | `web/src/lib/server/typeset/decollision.ts` |
| 无重音降级 | `web/src/lib/diacritics.ts` |
| LTR/RTL 多数派检测 | `web/src/lib/text-direction.ts` |
| 翻译主流程 | `web/src/lib/server/translate.ts` |
| Prompt | `web/src/lib/server/translate/prompts.ts` |
| 解析鲁棒性 | `web/src/lib/server/translate/parser.ts` |
| 预过滤 | `web/src/lib/server/translate/filter.ts` |
| SFX 词典 | `web/src/lib/server/translate/sfx.ts` |
| 对话上下文窗口 | `web/src/lib/server/translate/dialogue-tracker.ts` |
| 术语表 | `web/src/lib/server/glossary.ts` + `glossary-match.ts` |
| 术语包 | `web/src/lib/server/glossary-packs/` |
| LLM 运行时 | `web/src/lib/server/llm.ts` |
| Provider | `web/src/lib/server/providers.ts` |
| 翻译缓存 | `web/src/lib/server/cache.ts` |
| SQLite schema | `web/src/lib/server/db/schema.ts` |
| 存储与清理 | `web/src/lib/server/storage-service.ts` |

---

## 附录 B：关键常量速查

| 常量 | 值 | 位置 |
| --- | --- | --- |
| `OCR_DET_LIMIT_SIDE` | 2000.0 | `ocr/engine.rs:18` |
| OCR rec 目标高 | 48 | `ocr/engine.rs` |
| `REC_MAX_WIDTH` | 2048 | `ocr/engine.rs:52` |
| `RFDETR_INPUT_SIZE` | 768 | `detect/rfdetr.rs:14` |
| `RTDETR_INPUT_SIZE` | 1024 | `detect/rtdetr.rs:13` |
| DB `unclip_ratio` | 1.6 | `ocr/engine.rs:958-969` |
| `TILING_TRIGGER_ASPECT` | 2.5 | `detect/tiling.rs:12` |
| `SEAM_TOUCH_PX` | 4 | `detect/tiling.rs:22` |
| reslice 默认页高 | 1600（1200~2000） | `chapters/reslice.ts:21-25` |
| `CUT_AIRSPACE_PX` | 3 | `ml/reslice.rs:181` |
| 禁区安全边距 | 35px | `ml/reslice.rs` |
| 禁区扫描 tile | 高 2400 / 步进 1900 | `ml/reslice.rs:310-311` |
| `BUBBLE_INSET_FRAC` | 0.12（clamp 8~48px） | `region_builder/expansion.rs:11-13` |
| `SIBLING_GAP` | 5px | `region_builder/expansion.rs` |
| `MIN_UNUSED_RATIO` / `MIN_SCALE` | 0.10 / 1.10 | `region_builder/expansion.rs` |
| `EXPANSION_SLACK_DAMPING` | 0.50 | `region_builder/expansion.rs` |
| 扩张上限 | 1.30（竖排 2.00） | `region_builder/expansion.rs` |
| `LAMA_MAX_PIXELS` | 2048² | `inpaint/plan.rs:13` |
| `LAMA_PATCH_PAD` | 24 | `inpaint/plan.rs:21` |
| `LAMA_BAND_OVERLAP` | 64 | `inpaint/plan.rs` |
| mask 膨胀 | 3px 圆盘 | `cleaner.rs:50` |
| shrinkwrap 白地板阈值 | lum≥220 / sat≤14 | `shrinkwrap.rs:26-94` |
| shrinkwrap 闸门 | white≥0.80 / spread≤12 / sat≤4.5 / p90_sat≤8.0 / lum≥246 | `shrinkwrap.rs:562-590` |
| shrinkwrap 羽化 | 2.5px smoothstep | `shrinkwrap.rs:647` |
| `MIN_FONT_SIZE` | 6 | `typeset/layout.ts:11` |
| `LINE_HEIGHT` | 1.2 | `typeset/layout.ts:13` |
| `BOX_INSET` | 0.05 | `typeset/layout.ts:10` |
| 描边系数 | 0.06 / 0.10 / 0.18 / 0.26 | `typeset.ts:307-323` |
| 字色亮度阈值 | 0.18（Rec.709） | `typeset/color.ts:5-17` |
| 背景采样区 | 框内 20%~80% | `typeset/color.ts:29-32` |
| 对话字号上限 | `max(24, 页宽 × 0.035)` | `typeset.ts:218-226` |
| 短句封顶 | `baseline × 1.25`，地板 18px | `typeset.ts:271-274` |
| `PROMPT_VERSION` | v24 | `translate.ts:67` |
| 对话上下文页数 | 默认 4（clamp 0~30） | `settings-service.ts:75-78` |
| 术语包规模 | 7 主题 × 20 语种 = 2660 组合 | `glossary-packs/index.ts` |
| 页并发 | 3 | `chapter-pipeline.ts:143` |
| LLM 全局并发 | 64 | `llm.ts:11` |
| CPU 线程上限 | 8 | `device.rs:645-654` |
