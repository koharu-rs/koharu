import type { TFunction } from 'i18next'

/** What the user gives a vendor besides an API key, which a vendor not named here takes. */
export type VendorTraits = {
  /** The user signs in with their plan in place of giving an API key. */
  signsIn?: boolean
  /** The user names its server by a base URL: one they run, or one that speaks OpenAI's format. */
  server?: boolean
  /** The user gives the context window its server runs models with, which the server doesn't say. */
  contextWindow?: boolean
  /** The user picks how it orders the providers behind a model. */
  routing?: boolean
}

/** Each vendor's traits, by rig's name, as `list_vendors` names them. */
export const vendorTraits: Record<string, VendorTraits> = {
  chatgpt: { signsIn: true },
  grok: { signsIn: true },
  openrouter: { routing: true },
  ollama: { server: true },
  lmstudio: { server: true },
  'openai-compatible': { server: true, contextWindow: true },
}

/** Vendors by rig's names, as their makers write them; others show their own name. */
const vendorNames: Record<string, string> = {
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
