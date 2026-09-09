import type {
  DetectionModel,
  InpaintingModel,
  OcrModel,
  PipelineConfig,
  Stage,
} from '@koharu/bridge/protocol'

export type PipelineModel = DetectionModel | OcrModel | InpaintingModel
export type ModelStage = Exclude<Stage, 'translation'>
export type ModelName = PipelineModel['model']

export const modelOptions = {
  detection: ['koharu-layout-rfdetr-seg-2xl'],
  ocr: ['paddleocr-vl-1.6', 'manga-ocr', 'baberu-ocr', 'hayai-ocr'],
  inpainting: ['lama', 'aot-inpainting', 'flux2-klein', 'rorem-mixed'],
} satisfies Record<ModelStage, ModelName[]>

export const modelNames: Record<ModelName, string> = {
  'koharu-layout-rfdetr-seg-2xl': 'Koharu Layout RF-DETR Seg 2XL',
  'paddleocr-vl-1.6': 'PaddleOCR-VL 1.6',
  'manga-ocr': 'Manga OCR',
  'baberu-ocr': 'Baberu OCR',
  'hayai-ocr': 'Hayai OCR',
  lama: 'LaMa',
  'aot-inpainting': 'AOT Inpainting',
  'flux2-klein': 'FLUX.2 Klein',
  'rorem-mixed': 'RORem Mixed',
}

export function defaultModel(model: ModelName): PipelineModel {
  switch (model) {
    case 'koharu-layout-rfdetr-seg-2xl':
      return { model, text_threshold: null, bubble_threshold: null, panel_threshold: null }
    case 'paddleocr-vl-1.6':
    case 'manga-ocr':
    case 'baberu-ocr':
    case 'hayai-ocr':
    case 'lama':
    case 'aot-inpainting':
      return { model }
    case 'flux2-klein':
      return { model, prompt: 'Remove the text and reconstruct the background.' }
    case 'rorem-mixed':
      return { model }
  }
}

export function stageModel(config: PipelineConfig, stage: ModelStage): PipelineModel {
  const selected =
    stage === 'detection' ? config.detection : stage === 'ocr' ? config.ocr : config.inpainting
  if (selected) {
    const profile = config.processor?.[selected.model as keyof typeof config.processor]
    return profile ? ({ model: selected.model, ...profile } as PipelineModel) : selected
  }
  return defaultModel(modelOptions[stage][0]!)
}

export function replaceStage(
  config: PipelineConfig,
  stage: ModelStage,
  model: PipelineModel,
): PipelineConfig {
  switch (stage) {
    case 'detection':
      return {
        ...config,
        detection: model as DetectionModel,
        processor: {
          ...config.processor,
          'koharu-layout-rfdetr-seg-2xl':
            model.model === 'koharu-layout-rfdetr-seg-2xl'
              ? {
                  text_threshold: model.text_threshold ?? null,
                  bubble_threshold: model.bubble_threshold ?? null,
                  panel_threshold: model.panel_threshold ?? null,
                }
              : (config.processor?.['koharu-layout-rfdetr-seg-2xl'] ?? null),
        },
      }
    case 'ocr':
      return { ...config, ocr: model as OcrModel }
    case 'inpainting':
      return {
        ...config,
        inpainting: model as InpaintingModel,
        processor: {
          ...config.processor,
          ...(model.model === 'flux2-klein'
            ? { 'flux2-klein': { prompt: model.prompt ?? undefined } }
            : {}),
          ...(model.model === 'rorem-mixed'
            ? {
                'rorem-mixed': {
                  prompt: model.prompt ?? undefined,
                  negative_prompt: model.negative_prompt ?? undefined,
                },
              }
            : {}),
        },
      }
  }
}
