import type { TFunction } from 'i18next'

import type { Detector, Inpainter, Recognizer } from '@koharu/ipc/protocol'

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

/** Vendors by rig's names, as their makers write them; others show their own name. */
export const vendorNames: Record<string, string> = {
  openai: 'OpenAI',
  anthropic: 'Anthropic',
  'gcp.gemini': 'Google Gemini',
  xai: 'xAI',
  deepseek: 'DeepSeek',
  minimax: 'MiniMax',
  openrouter: 'OpenRouter',
  chatgpt: 'ChatGPT',
  grok: 'Grok',
  ollama: 'Ollama',
  lmstudio: 'LM Studio',
  koharu: 'Koharu',
}

/** The vendor's name: its maker's, or for an OpenAI-compatible server, which no one makes, a
 * description in the interface's language. */
export function vendorName(vendor: string, t: TFunction): string {
  if (vendor === 'openai-compatible') return t('settings.providers.openaiCompatible')
  return vendorNames[vendor] ?? vendor
}
