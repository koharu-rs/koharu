import { describe, expect, it } from 'vitest'

import { resources } from '@/lib/i18n'
import { orderedLanguageChoices } from '@/lib/translation'

function keys(value: object, prefix = ''): string[] {
  return Object.entries(value).flatMap(([key, item]) => {
    const path = prefix ? `${prefix}.${key}` : key
    return item && typeof item === 'object' ? keys(item, path) : [path]
  })
}

function strings(value: object, prefix = ''): Array<[string, string]> {
  return Object.entries(value).flatMap(([key, item]) => {
    const path = prefix ? `${prefix}.${key}` : key
    return typeof item === 'string' ? [[path, item]] : strings(item, path)
  })
}

function placeholders(value: string): string[] {
  return Array.from(value.matchAll(/{{\s*([^},\s]+).*?}}/g), ([, name]) => name).sort()
}

describe('editor localization', () => {
  it('defines the same flattened translation schema in every locale', () => {
    const expected = keys(resources['en-US'].translation).sort()
    for (const [locale, resource] of Object.entries(resources)) {
      expect('native' in resource.translation, locale).toBe(false)
      expect(keys(resource.translation).sort(), locale).toEqual(expected)
    }
  })

  it('preserves interpolation variables and valid text in every locale', () => {
    const english = new Map(strings(resources['en-US'].translation))

    for (const [locale, resource] of Object.entries(resources)) {
      for (const [key, value] of strings(resource.translation)) {
        expect(value.trim(), `${locale}: ${key}`).not.toBe('')
        expect(placeholders(value), `${locale}: ${key}`).toEqual(
          placeholders(english.get(key) ?? ''),
        )
        expect(value, `${locale}: ${key}`).not.toMatch(/[ÃƒÃ‚]|Ã¢(?:â‚¬|â„¢|Å“)|Ã¯Â¿Â½|ï¿½/)
      }
    }
  })

  it('describes the available update and emphasizes its version in every locale', () => {
    for (const [locale, resource] of Object.entries(resources)) {
      expect(resource.translation.updater.available.description, locale).toContain(
        '<strong>{{version}}</strong>',
      )
    }
  })

  it('distinguishes a font origin from source text', () => {
    for (const [locale, resource] of Object.entries(resources)) {
      expect(resource.translation.fontPicker.source, locale).not.toBe(
        resource.translation.inspector.source,
      )
    }
  })

  it('names the target languages in the interface language, in its order', () => {
    const names = orderedLanguageChoices('en-US').map((language) => language.name)
    expect(names).toContain('English')
    expect(names).toContain('Japanese')
    expect(names).toEqual(
      [...names].sort((left, right) =>
        left.localeCompare(right, 'en-US', { numeric: true, sensitivity: 'base' }),
      ),
    )
    expect(orderedLanguageChoices('ja-JP').map((language) => language.name)).toContain('英語')
  })
})
