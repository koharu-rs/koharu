import { describe, expect, it, vi } from 'vitest'

import {
  createGlossaryEntryUpdateQueue,
  eligibleGlossaryTranslationIds,
  filterGlossaryEntries,
  glossaryStatus,
  normalizeGlossaryTranslation,
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

  it('normalizes whitespace-only translation input to null', () => {
    expect(normalizeGlossaryTranslation(' \u2003 ')).toBeNull()
    expect(normalizeGlossaryTranslation('  Alice  ')).toBe('Alice')
  })

  it('accepts JSON import text and rejects malformed files before preview', () => {
    const document = '{"format":"koharu-glossary","version":1,"entries":[]}'
    expect(parseGlossaryDocument(document)).toBe(document)
    expect(() => parseGlossaryDocument('{oops')).toThrow('malformed')
    expect(() => parseGlossaryDocument('[]')).toThrow('document')
  })

  it('publishes only the newest full view while serializing edits to different entries', async () => {
    const first = Promise.withResolvers<GlossaryView>()
    const second = Promise.withResolvers<GlossaryView>()
    const execute = vi
      .fn<(revision: number, id: string, patch: GlossaryEntryPatch) => Promise<GlossaryView>>()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    const published: GlossaryView[] = []
    const queue = createGlossaryEntryUpdateQueue({
      initialRevision: 1,
      owner: 'Book',
      isOwnerCurrent: () => true,
      execute,
      onResponse: (response, current) => {
        if (current) published.push(response)
      },
    })
    const firstUpdate = queue.enqueue('haruka', { ...entries[0]!, source: '春香 first' })
    const secondUpdate = queue.enqueue('academy', {
      ...entries[1]!,
      translation: 'Moonlight Academy',
    })

    expect(execute).toHaveBeenCalledExactlyOnceWith(1, 'haruka', {
      source: '春香 first',
      translation: 'Haruka',
      kind: 'person',
      enabled: true,
    })
    const firstView = view(2, '春香 first')
    first.resolve(firstView)
    await firstUpdate
    expect(execute).toHaveBeenLastCalledWith(2, 'academy', {
      source: '月影学園',
      translation: 'Moonlight Academy',
      kind: 'organization',
      enabled: true,
    })
    expect(published).toEqual([])

    const secondView = {
      ...firstView,
      revision: 3,
      entries: firstView.entries.map((entry) =>
        entry.id === 'academy'
          ? { ...entry, revision: 3, translation: 'Moonlight Academy' }
          : { ...entry, revision: 3 },
      ),
    }
    second.resolve(secondView)
    await secondUpdate
    expect(published).toEqual([secondView])
  })

  it('preserves a newer external revision for the next serialized edit', async () => {
    const first = Promise.withResolvers<GlossaryView>()
    const execute = vi
      .fn<(revision: number, id: string, patch: GlossaryEntryPatch) => Promise<GlossaryView>>()
      .mockReturnValueOnce(first.promise)
      .mockResolvedValueOnce(view(12, 'latest'))
    const queue = createGlossaryEntryUpdateQueue({
      initialRevision: 7,
      owner: 'Book',
      isOwnerCurrent: () => true,
      execute,
      onResponse: vi.fn(),
    })

    const firstUpdate = queue.enqueue('haruka', { ...entries[0]!, source: 'first' })
    const secondUpdate = queue.enqueue('haruka', { ...entries[0]!, source: 'latest' })
    queue.setRevision(11)
    first.resolve(view(8, 'first'))
    await firstUpdate
    await secondUpdate

    expect(execute).toHaveBeenNthCalledWith(2, 11, 'haruka', {
      source: 'latest',
      translation: 'Haruka',
      kind: 'person',
      enabled: true,
    })
  })

  it('drops queued edits after a revision conflict instead of replaying them', async () => {
    const conflict = Promise.withResolvers<GlossaryView>()
    const execute = vi
      .fn<(revision: number, id: string, patch: GlossaryEntryPatch) => Promise<GlossaryView>>()
      .mockReturnValue(conflict.promise)
    const queue = createGlossaryEntryUpdateQueue({
      initialRevision: 1,
      owner: 'Book',
      isOwnerCurrent: () => true,
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
