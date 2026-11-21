import { displayNames } from './i18n'

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

/** A target language's name in the interface's language `display`. */
export function languageName(tag: string, display: string): string {
  return displayNames(display).of(tag) ?? tag
}

/** The target languages' names by tag, in the interface's language and its alphabetical order. */
export function languageNames(display: string): Record<string, string> {
  const names = displayNames(display)
  const collator = new Intl.Collator(display, { numeric: true, sensitivity: 'base' })
  return Object.fromEntries(
    TARGET_LANGUAGES.map((tag) => [tag, names.of(tag) ?? tag]).sort(([, left], [, right]) =>
      collator.compare(left, right),
    ),
  )
}
