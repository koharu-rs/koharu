/** A vendor Koharu lists models from, by rig's name, and what the user gives it. */
export type Vendor = {
  name: string
  /** The user signs in with their plan in place of giving an API key. */
  signsIn?: boolean
  /** The user names its server by a base URL: one they run, or one that speaks OpenAI's format. */
  server?: boolean
  /** The user gives the context window its server runs models with, which the server doesn't say. */
  contextWindow?: boolean
}

/** Every vendor Koharu lists models from, in Settings' order. */
export const vendors: Vendor[] = [
  { name: 'openai' },
  { name: 'anthropic' },
  { name: 'gcp.gemini' },
  { name: 'xai' },
  { name: 'deepseek' },
  { name: 'minimax' },
  { name: 'openrouter' },
  { name: 'chatgpt', signsIn: true },
  { name: 'grok', signsIn: true },
  { name: 'ollama', server: true },
  { name: 'lmstudio', server: true },
  { name: 'openai-compatible', server: true, contextWindow: true },
]
