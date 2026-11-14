import type { Model } from '@koharu/bridge/protocol'

/** The languages the translator is offered, as BCP 47 tags. The pipeline takes any language CLDR
 * names; these are the ones manga is commonly translated into, with a region or a script only
 * where the variant is a real one. */
export const TARGET_LANGUAGES = [
  'ar',
  'de',
  'en',
  'es',
  'fr',
  'id',
  'it',
  'ja',
  'ko',
  'ms',
  'nl',
  'pl',
  'pt',
  'pt-BR',
  'ru',
  'th',
  'tr',
  'uk',
  'vi',
  'zh-Hans',
  'zh-Hant',
] as const

/** The target languages named in the interface's language, in its alphabetical order. */
export function orderedLanguageChoices(display: string): Array<{ tag: string; name: string }> {
  const names = new Intl.DisplayNames([display], { type: 'language' })
  return TARGET_LANGUAGES.map((tag) => ({ tag, name: names.of(tag) ?? tag })).sort((left, right) =>
    left.name.localeCompare(right.name, display, { numeric: true, sensitivity: 'base' }),
  )
}

/** rig's portable reasoning levels, lowest first. */
export const REASONING = ['minimal', 'low', 'medium', 'high', 'xhigh', 'max'] as const

/** A model's name, or its reference when the list doesn't have it. */
export function modelName(models: Model[], reference: string | null | undefined): string {
  if (!reference) return ''
  return models.find((model) => model.reference === reference)?.name ?? reference
}

/** A model's vendor, from its `vendor/model` reference. */
export function vendorOf(reference: string): string {
  return reference.split('/', 1)[0] ?? reference
}
