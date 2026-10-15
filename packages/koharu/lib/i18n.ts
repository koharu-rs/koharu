'use client'

import i18n, { type Resource } from 'i18next'
import LanguageDetector from 'i18next-browser-languagedetector'
import LocalStorageBackend from 'i18next-localstorage-backend'
import { initReactI18next } from 'react-i18next'

import enUS from '@/public/locales/en-US/translation.json'
import esES from '@/public/locales/es-ES/translation.json'
import frFR from '@/public/locales/fr-FR/translation.json'
import jaJP from '@/public/locales/ja-JP/translation.json'
import koKR from '@/public/locales/ko-KR/translation.json'
import ptBR from '@/public/locales/pt-BR/translation.json'
import ruRU from '@/public/locales/ru-RU/translation.json'
import trTR from '@/public/locales/tr-TR/translation.json'
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
  'fr-FR': { translation: frFR },
} satisfies Resource

export type SupportedLanguage = keyof typeof resources

export const supportedLanguages = Object.keys(resources) as SupportedLanguage[]

i18n
  .use(LocalStorageBackend)
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources,
    supportedLngs: supportedLanguages,
    fallbackLng: 'en-US',
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
