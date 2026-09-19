import { describe, expect, it, vi } from 'vitest'

import {
  createGlossaryEntryUpdateQueue,
  eligibleGlossaryTranslationIds,
  filterGlossaryEntries,
  glossaryStatus,
  parseGlossaryDocument,
} from '@/lib/glossary'
import type { GlossaryEntryPatch, GlossaryEntryView, GlossaryView } from '@koharu/bridge/protocol'

const entries: GlossaryEntryView[] = [
  {
    revision: 1,
    id: 'haruka',
    source: '春香',
    translation: 'Haruka',
    kind: 'person',
    enabled: true,
    confidence: 0.98,
    occurrenceCount: 4,
    examples: ['春香さん'],
    sourceOrigin: 'detected',
    translationOrigin: 'user',
    presentInLastScan: true,
  },
  {
    revision: 1,
    id: 'academy',
    source: '月影学園',
    translation: null,
    kind: 'organization',
    enabled: true,
    confidence: 0.82,
    occurrenceCount: 2,
    examples: [],
    sourceOrigin: 'detected',
    translationOrigin: null,
    presentInLastScan: false,
  },
  {
    revision: 1,
    id: 'blade',
    source: '星斬り',
    translation: null,
    kind: 'ability',
    enabled: false,
    confidence: null,
    occurrenceCount: 0,
    examples: [],
    sourceOrigin: 'user',
    translationOrigin: null,
    presentInLastScan: true,
  },
]

const view = (revision: number, source = '春香'): GlossaryView => ({
  revision,
  enabled: true,
  stale: false,
  sourceLanguage: 'ja',
  targetLanguage: 'en',
  savedSourceFingerprint: 'saved',
  currentSourceFingerprint: 'saved',
  entries: entries.map((entry) => (entry.id === 'haruka' ? { ...entry, revision, source } : entry)),
})

describe('glossary helpers', () => {
  it('filters source and translation text case-insensitively', () => {
    expect(filterGlossaryEntries(entries, { query: 'HARUKA', kind: 'all', state: 'all' })).toEqual([
      entries[0],
    ])
    expect(filterGlossaryEntries(entries, { query: '月影', kind: 'all', state: 'all' })).toEqual([
      entries[1],
    ])
  })

  it.each([
    ['translated', ['haruka']],
    ['untranslated', ['academy', 'blade']],
    ['disabled', ['blade']],
    ['not-present', ['academy']],
  ] as const)('filters the %s state', (state, ids) => {
    expect(
      filterGlossaryEntries(entries, { query: '', kind: 'all', state }).map((entry) => entry.id),
    ).toEqual(ids)
  })

  it('combines category and state filters', () => {
    expect(
      filterGlossaryEntries(entries, {
        query: '',
        kind: 'organization',
        state: 'untranslated',
      }).map((entry) => entry.id),
    ).toEqual(['academy'])
  })

  it('selects only enabled untranslated entries for automatic translation', () => {
    expect(eligibleGlossaryTranslationIds(entries)).toEqual(['academy'])
    expect(
      eligibleGlossaryTranslationIds(entries, new Set(['haruka', 'academy', 'blade'])),
    ).toEqual(['academy'])
  })

  it('distinguishes unscanned, available, and stale glossaries', () => {
    expect(glossaryStatus({ ...view(1), savedSourceFingerprint: null })).toBe('unscanned')
    expect(glossaryStatus(view(1))).toBe('available')
    expect(glossaryStatus({ ...view(1), stale: true })).toBe('stale')
  })

  it('accepts JSON import text and rejects malformed files before preview', () => {
    const document = '{"format":"koharu-glossary","version":1,"entries":[]}'
    expect(parseGlossaryDocument(document)).toBe(document)
    expect(() => parseGlossaryDocument('{oops')).toThrow('malformed')
    expect(() => parseGlossaryDocument('[]')).toThrow('document')
  })

  it('serializes inline edits and marks only the newest response as current', async () => {
    const first = Promise.withResolvers<GlossaryView>()
    const second = Promise.withResolvers<GlossaryView>()
    const execute = vi
      .fn<(revision: number, id: string, patch: GlossaryEntryPatch) => Promise<GlossaryView>>()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    const accepted: string[] = []
    const queue = createGlossaryEntryUpdateQueue({
      initialRevision: 1,
      execute,
      onResponse: (response, current) => {
        if (current) accepted.push(response.entries[0]!.source)
      },
    })
    const base = { ...entries[0]!, source: 'first' }
    const firstUpdate = queue.enqueue('haruka', base)
    const secondUpdate = queue.enqueue('haruka', { ...base, source: 'second' })

    expect(execute).toHaveBeenCalledExactlyOnceWith(1, 'haruka', {
      source: 'first',
      translation: 'Haruka',
      kind: 'person',
      enabled: true,
    })
    first.resolve(view(2, 'first'))
    await firstUpdate
    expect(execute).toHaveBeenLastCalledWith(2, 'haruka', {
      source: 'second',
      translation: 'Haruka',
      kind: 'person',
      enabled: true,
    })
    expect(accepted).toEqual([])

    second.resolve(view(3, 'second'))
    await secondUpdate
    expect(accepted).toEqual(['second'])
  })

  it('drops queued edits after a revision conflict instead of replaying them', async () => {
    const conflict = Promise.withResolvers<GlossaryView>()
    const execute = vi
      .fn<(revision: number, id: string, patch: GlossaryEntryPatch) => Promise<GlossaryView>>()
      .mockReturnValue(conflict.promise)
    const queue = createGlossaryEntryUpdateQueue({
      initialRevision: 1,
      execute,
      onResponse: vi.fn(),
    })
    const patch = entries[0]!
    const firstUpdate = queue.enqueue('haruka', { ...patch, source: 'first' })
    const queuedUpdate = queue.enqueue('haruka', { ...patch, source: 'second' })

    conflict.reject(new Error('revision conflict'))
    await expect(firstUpdate).rejects.toThrow('revision conflict')
    await expect(queuedUpdate).rejects.toThrow('revision conflict')
    expect(execute).toHaveBeenCalledTimes(1)
  })
})
