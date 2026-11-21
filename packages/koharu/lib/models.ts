import type { Detector, Inpainter, Model, Recognizer } from '@koharu/ipc/protocol'

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

/** A model's name, or its reference when the list doesn't have it. */
export function modelName(models: Model[], reference: string | null | undefined): string {
  if (!reference) return ''
  return models.find((model) => model.reference === reference)?.name ?? reference
}

/** A model's vendor, from its `vendor/model` reference. */
export function vendorOf(reference: string): string {
  return reference.split('/', 1)[0] ?? reference
}
