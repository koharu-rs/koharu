import type {
  GlossaryEntryPatch,
  GlossaryEntryView,
  GlossaryKind,
  GlossaryView,
} from '@koharu/bridge/protocol'

export const glossaryKinds: readonly GlossaryKind[] = [
  'person',
  'place',
  'organization',
  'item',
  'ability',
  'term',
  'other',
]

export type GlossaryStateFilter = 'all' | 'translated' | 'untranslated' | 'disabled' | 'not-present'

export interface GlossaryFilters {
  query: string
  kind: GlossaryKind | 'all'
  state: GlossaryStateFilter
}

export function filterGlossaryEntries(
  entries: readonly GlossaryEntryView[],
  filters: GlossaryFilters,
): GlossaryEntryView[] {
  const query = filters.query.trim().toLocaleLowerCase()
  return entries.filter((entry) => {
    if (filters.kind !== 'all' && entry.kind !== filters.kind) return false
    if (
      query &&
      !`${entry.source}\n${entry.translation ?? ''}`.toLocaleLowerCase().includes(query)
    ) {
      return false
    }
    switch (filters.state) {
      case 'translated':
        return Boolean(entry.translation)
      case 'untranslated':
        return !entry.translation
      case 'disabled':
        return !entry.enabled
      case 'not-present':
        return !entry.presentInLastScan
      default:
        return true
    }
  })
}

export function eligibleGlossaryTranslationIds(
  entries: readonly GlossaryEntryView[],
  selected?: ReadonlySet<string>,
): string[] {
  return entries
    .filter(
      (entry) =>
        entry.enabled && !entry.translation && (selected === undefined || selected.has(entry.id)),
    )
    .map((entry) => entry.id)
}

export function glossaryStatus(view: GlossaryView): 'unscanned' | 'available' | 'stale' {
  if (view.savedSourceFingerprint === null) return 'unscanned'
  return view.stale ? 'stale' : 'available'
}

export function parseGlossaryDocument(text: string): string {
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    throw new Error('The glossary file is malformed JSON.')
  }
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error('The glossary file must contain a JSON document.')
  }
  return text
}

function entryPatch(entry: GlossaryEntryPatch): GlossaryEntryPatch {
  return {
    source: entry.source,
    translation: entry.translation,
    kind: entry.kind,
    enabled: entry.enabled,
  }
}

interface UpdateQueueOptions {
  initialRevision: number
  execute: (revision: number, id: string, patch: GlossaryEntryPatch) => Promise<GlossaryView>
  onResponse: (view: GlossaryView, current: boolean) => void | Promise<void>
}

interface PendingUpdate {
  id: string
  patch: GlossaryEntryPatch
  generation: number
  resolve: (view: GlossaryView) => void
  reject: (error: unknown) => void
}

export interface GlossaryEntryUpdateQueue {
  enqueue: (id: string, patch: GlossaryEntryPatch) => Promise<GlossaryView>
  setRevision: (revision: number) => void
}

export function createGlossaryEntryUpdateQueue({
  initialRevision,
  execute,
  onResponse,
}: UpdateQueueOptions): GlossaryEntryUpdateQueue {
  let revision = initialRevision
  let running = false
  let generation = 0
  const pending: PendingUpdate[] = []

  const run = async () => {
    if (running) return
    running = true
    while (pending.length > 0) {
      const update = pending.shift()!
      try {
        const response = await execute(revision, update.id, update.patch)
        revision = Math.max(revision, response.revision)
        const current = generation === update.generation
        await onResponse(response, current)
        update.resolve(response)
      } catch (error) {
        update.reject(error)
        for (const queued of pending.splice(0)) queued.reject(error)
        break
      }
    }
    running = false
  }

  return {
    enqueue(id, patch) {
      generation += 1
      const promise = new Promise<GlossaryView>((resolve, reject) => {
        pending.push({ id, patch: entryPatch(patch), generation, resolve, reject })
      })
      void run()
      return promise
    },
    setRevision(nextRevision) {
      revision = Math.max(revision, nextRevision)
    },
  }
}
