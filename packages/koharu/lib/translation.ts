import type { Model } from '@koharu/ipc/protocol'

/** The languages the translator is offered, as BCP 47 tags, with a region or a script only where
 * the variant is a real one. The pipeline takes any language CLDR names. */
export const TARGET_LANGUAGES = [
  'ar',
  'be',
  'bg',
  'bn',
  'bo',
  'cs',
  'de',
  'en',
  'es',
  'fa',
  'fil',
  'fr',
  'gu',
  'he',
  'hi',
  'hu',
  'id',
  'it',
  'ja',
  'kk',
  'km',
  'ko',
  'mn',
  'mr',
  'ms',
  'my',
  'nl',
  'pl',
  'pt',
  'pt-BR',
  'ru',
  'ta',
  'te',
  'th',
  'tr',
  'ug',
  'uk',
  'ur',
  'vi',
  'yue',
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
