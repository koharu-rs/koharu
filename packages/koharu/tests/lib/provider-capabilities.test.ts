import { describe, expect, it } from 'vitest'

import {
  declaresModalities,
  optionsFor,
  retainSelection,
  selectionInvalidated,
  supports,
  type EntryPoint,
} from '@/lib/providerCapabilities'
import type { Model, Provider } from '@koharu/bridge/protocol'

const text: EntryPoint = { kind: 'text' }
const image: EntryPoint = { kind: 'multimodal', modality: 'image' }

function model(provider: Provider, id: string, vision: boolean): Model {
  return {
    provider,
    model: id,
    name: id,
    quantizations: [],
    vision,
    reasoning: false,
  }
}

/**
 * A catalog shaped like the live OrcaRouter one: a text-only model, two models
 * that declare image input, and one record whose architecture block is absent so
 * its capability is unknown.
 */
const catalog: Model[] = [
  model('orcarouter', 'deepseek/deepseek-v4-pro', false),
  model('orcarouter', 'anthropic/claude-opus-4.8', true),
  model('orcarouter', 'google/gemini-3.5-flash', true),
  model('orcarouter', 'orcarouter/unknown-architecture', false),
]

describe('OrcaRouter model capability filtering', () => {
  it('offers the full catalog to a text entry point', () => {
    expect(optionsFor(catalog, text)).toHaveLength(4)
  })

  it('offers only models that declare image input to a multimodal entry point', () => {
    const offered = optionsFor(catalog, image)
    expect(offered.map((entry) => entry.model)).toEqual([
      'anthropic/claude-opus-4.8',
      'google/gemini-3.5-flash',
    ])
    // The undeclared-architecture record fails closed rather than being assumed.
    expect(offered.map((entry) => entry.model)).not.toContain('orcarouter/unknown-architecture')
  })

  it('has no options for an audio or video entry point Koharu cannot upload', () => {
    expect(optionsFor(catalog, { kind: 'multimodal', modality: 'audio' })).toEqual([])
    expect(optionsFor(catalog, { kind: 'multimodal', modality: 'video' })).toEqual([])
  })

  it('leaves other providers untouched so their picker behaviour is preserved', () => {
    const mixed: Model[] = [
      model('openrouter', 'openrouter/auto', false),
      model('openai', 'openai/gpt-5.5', true),
    ]
    for (const provider of ['openrouter', 'openai'] as const) {
      expect(declaresModalities(provider)).toBe(false)
    }
    // A provider without declared modalities is not filtered by this seam.
    expect(optionsFor(mixed, image)).toHaveLength(2)
    expect(retainSelection(mixed, image, mixed[0])).toEqual(mixed[0])
    expect(selectionInvalidated(mixed, image, mixed[0])).toBe(false)
  })

  it('drops a saved model that the current entry point cannot use', () => {
    const saved = catalog[0]
    expect(retainSelection(catalog, image, saved)).toBeNull()
    expect(selectionInvalidated(catalog, image, saved)).toBe(true)
    expect(retainSelection(catalog, text, saved)).toEqual(saved)
    expect(selectionInvalidated(catalog, text, saved)).toBe(false)
  })

  it('keeps a compatible saved model', () => {
    const saved = catalog[1]
    expect(retainSelection(catalog, image, saved)).toEqual(saved)
    expect(selectionInvalidated(catalog, image, saved)).toBe(false)
  })

  it('does not invalidate a saved model while the catalog is empty', () => {
    // Discovery is still in flight or failed; clearing the user's choice then
    // would be a regression, not a correction.
    expect(selectionInvalidated([], image, catalog[0])).toBe(false)
    expect(selectionInvalidated(catalog, image, null)).toBe(false)
    expect(selectionInvalidated(catalog, image, undefined)).toBe(false)
  })

  it('reports support only for a declared modality', () => {
    expect(supports(catalog[1], image)).toBe(true)
    expect(supports(catalog[0], image)).toBe(false)
    expect(supports(catalog[0], text)).toBe(true)
  })

  it('changes the offered options when the entry point changes', () => {
    const asText = optionsFor(catalog, text).map((entry) => entry.model)
    const asImage = optionsFor(catalog, image).map((entry) => entry.model)
    expect(asImage).not.toEqual(asText)
    expect(asText).toContain('deepseek/deepseek-v4-pro')
    expect(asImage).not.toContain('deepseek/deepseek-v4-pro')
  })
})
