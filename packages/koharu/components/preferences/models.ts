import type { Detector, Inpainter, Recognizer } from '@koharu/bridge/protocol'

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
  'rorem-mixed': 'RORem',
  'flux3-klein': 'FLUX.3 Klein',
  'moss-image': 'MOSS-Image',
  'qwen-image': 'Qwen-Image 2.1',
}

/** LaMa, which takes no prompt, ignores the inpainter's. */
export function takesPrompt(inpainter: Inpainter): boolean {
  return inpainter !== 'lama'
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
  ollama: 'Ollama',
  lmstudio: 'LM Studio',
  koharu: 'Koharu',
}

export function vendorName(vendor: string): string {
  return vendorNames[vendor] ?? vendor
}
