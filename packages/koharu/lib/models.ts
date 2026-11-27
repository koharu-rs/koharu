import type { TFunction } from 'i18next'

import type {
  Detector,
  Inpainter,
  ModelCatalog,
  ModelVariant,
  PipelineConfig,
  Recognizer,
  Stage,
  TranslateModelEntry,
} from '@koharu/ipc/protocol'

/** The pipeline's models by id, as the backend names them; product names stay untranslated. */
export const detectors: Record<Detector, string> = {
  'koharu-layout-highres': 'Koharu Layout HighRes',
}

export const recognizers: Record<Recognizer, string> = {
  'manga-ocr-ar': 'Manga OCR AR',
  'baberu-ocr': 'Baberu OCR',
  'hayai-ocr': 'Hayai OCR',
}

export const inpainters: Record<Inpainter, string> = {
  lama: 'LaMa',
  'flux3-klein': 'FLUX.3 Klein',
  'qwen-image': 'Qwen-Image 2.1 Turbo',
}

/** Off, then rig's portable reasoning levels, lowest first, as the settings write them. */
export const REASONING = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'] as const

export type VramFit = 'fits' | 'swaps' | 'tooLarge'

export interface CatalogSelection {
  model: TranslateModelEntry
  variant: ModelVariant | null
  explicitQuant: boolean
}

/** Resolves a `vendor/model` or `koharu/<id>:<quant>` reference against catalog entries
 * without altering hosted model references containing `:` (e.g. `ollama/llama3:8b`). */
export function findCatalogSelection(
  models: TranslateModelEntry[],
  reference: string | null | undefined,
): CatalogSelection | null {
  if (!reference) return null
  const exact = models.find((model) => model.reference === reference)
  if (exact) {
    return { model: exact, variant: exact.variants[0] ?? null, explicitQuant: false }
  }
  if (reference.startsWith('koharu/')) {
    for (const model of models) {
      if (!model.reference.startsWith('koharu/')) continue
      const variant = model.variants.find((candidate) => candidate.reference === reference)
      if (variant) {
        return { model, variant, explicitQuant: true }
      }
    }
  }
  return null
}

/** A model's display name (including explicit quantization if chosen), or its reference
 * when the catalog doesn't have it. */
export function formatModelName(
  models: TranslateModelEntry[],
  reference: string | null | undefined,
  t?: TFunction,
): string {
  if (!reference) return ''
  const selection = findCatalogSelection(models, reference)
  if (!selection) return reference
  if (selection.explicitQuant && selection.variant) {
    return t
      ? t('modelPicker.quantizationLabel', {
          model: selection.model.name,
          quantization: selection.variant.name,
        })
      : `${selection.model.name} (${selection.variant.name})`
  }
  return selection.model.name
}

/** Classifies whether `candidateVramBytes` fits in `availableVramBytes` alongside
 * `otherStagesVramBytes`, returning `null` when either measurement is unavailable. */
export function classifyVramFit(
  candidateVramBytes: number | null | undefined,
  otherStagesVramBytes: number,
  availableVramBytes: number | null | undefined,
): VramFit | null {
  if (candidateVramBytes == null || availableVramBytes == null || availableVramBytes <= 0) {
    return null
  }
  if (candidateVramBytes > availableVramBytes) {
    return 'tooLarge'
  }
  if (candidateVramBytes + otherStagesVramBytes <= availableVramBytes) {
    return 'fits'
  }
  return 'swaps'
}

/** Sums measured VRAM bytes across the currently selected models in every stage except
 * `excludeStage` (unmeasured or hosted models contribute 0). */
export function computeOtherStagesVram(
  catalog: ModelCatalog | undefined,
  pipeline: PipelineConfig | undefined,
  excludeStage: Stage,
): number {
  if (!catalog) return 0
  let total = 0
  if (excludeStage !== 'detect') {
    const id = pipeline?.detector?.model ?? 'koharu-layout-highres'
    total += catalog.detect.find((entry) => entry.id === id)?.measured_vram_bytes ?? 0
  }
  if (excludeStage !== 'recognize') {
    const id = pipeline?.recognizer?.model ?? 'hayai-ocr'
    total += catalog.recognize.find((entry) => entry.id === id)?.measured_vram_bytes ?? 0
  }
  if (excludeStage !== 'inpaint') {
    const id = pipeline?.inpainter?.model ?? 'lama'
    total += catalog.inpaint.find((entry) => entry.id === id)?.measured_vram_bytes ?? 0
  }
  if (excludeStage !== 'translate') {
    const selection = findCatalogSelection(catalog.translate, pipeline?.translator?.model)
    total += selection?.variant?.measured_vram_bytes ?? selection?.model.measured_vram_bytes ?? 0
  }
  return total
}

function formatScaledBytes(
  bytes: number,
  base: 1000 | 1024,
  t: TFunction,
  locale?: string,
): string {
  const units = ['b', 'kb', 'mb', 'gb'] as const
  let value = Math.max(0, bytes)
  let index = 0
  while (value >= base && index < units.length - 1) {
    value /= base
    index += 1
  }
  const formatted = new Intl.NumberFormat(locale, {
    maximumFractionDigits: index === 0 ? 0 : 1,
    minimumFractionDigits: index === 0 ? 0 : 1,
  }).format(value)
  return t(`modelPicker.units.${units[index]}`, { value: formatted })
}

/** Formats model file sizes using base-1000 units (B/KB/MB/GB) matching Hugging Face. */
export function formatDiskSize(bytes: number, t: TFunction, locale?: string): string {
  return formatScaledBytes(bytes, 1000, t, locale)
}

/** Formats GPU VRAM sizes using base-1024 units (GiB displayed as GB). */
export function formatVramSize(bytes: number, t: TFunction, locale?: string): string {
  return formatScaledBytes(bytes, 1024, t, locale)
}

/** A model's vendor, from its `vendor/model` reference. */
export function vendorOf(reference: string): string {
  return reference.split('/', 1)[0] ?? reference
}
