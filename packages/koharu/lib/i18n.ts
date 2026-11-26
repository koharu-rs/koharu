'use client'

// FormatJS's DisplayNames in place of Chromium's, whose ICU data keeps only the names of languages
// Chromium's own interface offers and so leaves some out, such as Tibetan; FormatJS carries CLDR's
// whole set for each interface language below.
import '@formatjs/intl-displaynames/polyfill-force.js'
import '@formatjs/intl-displaynames/locale-data/ar.js'
import '@formatjs/intl-displaynames/locale-data/en.js'
import '@formatjs/intl-displaynames/locale-data/es.js'
import '@formatjs/intl-displaynames/locale-data/fr.js'
import '@formatjs/intl-displaynames/locale-data/ja.js'
import '@formatjs/intl-displaynames/locale-data/ko.js'
import '@formatjs/intl-displaynames/locale-data/pt.js'
import '@formatjs/intl-displaynames/locale-data/ru.js'
import '@formatjs/intl-displaynames/locale-data/tr.js'
import '@formatjs/intl-displaynames/locale-data/zh.js'
import '@formatjs/intl-displaynames/locale-data/zh-Hant.js'
import i18n, { type Resource } from 'i18next'
import LanguageDetector from 'i18next-browser-languagedetector'
import { initReactI18next } from 'react-i18next'

import ar from '@/public/locales/ar/translation.json'
import enUS from '@/public/locales/en-US/translation.json'
import esES from '@/public/locales/es-ES/translation.json'
import frFR from '@/public/locales/fr-FR/translation.json'
import jaJP from '@/public/locales/ja-JP/translation.json'
import koKR from '@/public/locales/ko-KR/translation.json'
import ptBR from '@/public/locales/pt-BR/translation.json'
import ruRU from '@/public/locales/ru-RU/translation.json'
import trTR from '@/public/locales/tr-TR/translation.json'
import wuu from '@/public/locales/wuu/translation.json'
import zhCN from '@/public/locales/zh-CN/translation.json'
import zhTW from '@/public/locales/zh-TW/translation.json'

export const resources = {
  'en-US': { translation: enUS },
  'zh-CN': { translation: zhCN },
  'zh-TW': { translation: zhTW },
  'ja-JP': { translation: jaJP },
  'ru-RU': { translation: ruRU },
  'es-ES': { translation: esES },
  'tr-TR': { translation: trTR },
  'ko-KR': { translation: koKR },
  'pt-BR': { translation: ptBR },
  wuu: { translation: wuu },
  'fr-FR': { translation: frFR },
  ar: { translation: ar },
} satisfies Resource

export type SupportedLanguage = keyof typeof resources

// Every key the interface translates is en-US's, which `tsc` checks.
declare module 'i18next' {
  interface CustomTypeOptions {
    resources: (typeof resources)['en-US']
  }
}

export const supportedLanguages = Object.keys(resources) as SupportedLanguage[]

/** The CLDR locale whose names an interface language shows, where its own tag doesn't find it: Wu
 * has none, and Koharu writes it in simplified Chinese; Taiwan's is Traditional Chinese. */
const cldrLocales: Partial<Record<string, string>> = { wuu: 'zh', 'zh-TW': 'zh-Hant' }

/** Language names in the interface language `display`, from CLDR. */
export function displayNames(display: string) {
  return new Intl.DisplayNames([cldrLocales[display] ?? display], { type: 'language' })
}

/** Script names in the interface language `display`, from CLDR. The Han scripts go by Chinese's
 * name in them, as Simplified Chinese, since CLDR's script names call them only Simplified and
 * Traditional. */
export function scriptNames(display: string) {
  const locale = [cldrLocales[display] ?? display]
  const scripts = new Intl.DisplayNames(locale, { type: 'script' })
  const languages = new Intl.DisplayNames(locale, { type: 'language' })
  return {
    of: (code: string) =>
      code === 'Hans' || code === 'Hant' ? languages.of(`zh-${code}`) : scripts.of(code),
  }
}

/** The variant an interface language is named as, where a reader tells it from its kin; others are
 * named by their language alone, as a language menu lists them. */
const variants: Partial<Record<SupportedLanguage, string>> = {
  'zh-CN': 'zh-Hans',
  'zh-TW': 'zh-Hant',
  'pt-BR': 'pt-BR',
}

/** Each interface language by its own name, in every interface language, capitalized as a menu's
 * item begins. */
export const nativeNames = Object.fromEntries(
  supportedLanguages.map((language) => {
    const tag = variants[language] ?? new Intl.Locale(language).language
    const name = displayNames(language).of(tag) ?? language
    return [language, name.charAt(0).toLocaleUpperCase(language) + name.slice(1)]
  }),
) as Record<string, string>

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources,
    supportedLngs: supportedLanguages,
    fallbackLng: 'en-US',
    // The language picked last, kept in the webview's storage across restarts.
    detection: {
      order: ['localStorage', 'navigator'],
      caches: ['localStorage'],
    },
    interpolation: {
      escapeValue: false, // not needed for react as it escapes by default
    },
    react: {
      useSuspense: false,
    },
  })

export default i18n
