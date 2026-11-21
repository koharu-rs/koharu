/** What the user gives a vendor besides an API key, which a vendor not named here takes. */
export type VendorTraits = {
  /** The user signs in with their plan in place of giving an API key. */
  signsIn?: boolean
  /** The user names its server by a base URL: one they run, or one that speaks OpenAI's format. */
  server?: boolean
  /** The user gives the context window its server runs models with, which the server doesn't say. */
  contextWindow?: boolean
}

/** Each vendor's traits, by rig's name, as `list_vendors` names them. */
export const vendorTraits: Record<string, VendorTraits> = {
  chatgpt: { signsIn: true },
  grok: { signsIn: true },
  ollama: { server: true },
  lmstudio: { server: true },
  'openai-compatible': { server: true, contextWindow: true },
}
