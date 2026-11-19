'use client'

import { openUrl } from '@tauri-apps/plugin-opener'
import { Check, ExternalLink, Pencil, Plus, Search, Trash2 } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import type { GlossaryEntry, GlossaryKind, TermCandidate, TermLocation } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'
import { NativeSelect, NativeSelectOption } from '@koharu/ui/components/native-select'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@koharu/ui/components/tabs'

type View = 'terms' | 'blocked' | 'candidates'

const kinds: GlossaryKind[] = ['person', 'place', 'term', 'preserve', 'preserve_block']
const blocked = (entry: GlossaryEntry) =>
  entry.kind === 'preserve' || entry.kind === 'preserve_block'

export function GlossaryManager({
  entries,
  candidates,
  disabled,
  onSave,
  onLocation,
}: {
  entries: GlossaryEntry[]
  candidates?: TermCandidate[]
  disabled?: boolean
  onSave: (entries: GlossaryEntry[]) => Promise<void>
  onLocation?: (location: TermLocation) => void
}) {
  const { t } = useTranslation()
  const [view, setView] = useState<View>('terms')
  const [search, setSearch] = useState('')
  const [draft, setDraft] = useState<GlossaryEntry | null>(null)
  const [editing, setEditing] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const locked = busy || disabled
  const query = search.trim().toLocaleLowerCase()
  const matches = (entry: GlossaryEntry) =>
    [entry.source, entry.translation, entry.note].some((text) =>
      text.toLocaleLowerCase().includes(query),
    )
  const duplicate =
    draft &&
    entries.some((entry) => entry.source === draft.source.trim() && entry.source !== editing)

  const save = async (next: GlossaryEntry[], done?: () => void) => {
    setBusy(true)
    try {
      await onSave(next)
      done?.()
    } finally {
      setBusy(false)
    }
  }

  const replace = (previous: string, entry: GlossaryEntry) =>
    entries.some((item) => item.source === previous)
      ? entries.map((item) => (item.source === previous ? entry : item))
      : [...entries, entry]

  const toggleLocation = (entry: GlossaryEntry, location: TermLocation) => {
    const excluded = new Set(entry.excluded_layers)
    if (!excluded.delete(location.layer)) excluded.add(location.layer)
    void save(replace(entry.source, { ...entry, excluded_layers: [...excluded] })).catch(
      () => undefined,
    )
  }

  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (
      !draft ||
      locked ||
      duplicate ||
      !draft.source.trim() ||
      (!blocked(draft) && !draft.translation.trim())
    )
      return
    const entry = {
      ...draft,
      source: draft.source.trim(),
      translation: blocked(draft) ? '' : draft.translation.trim(),
      note: draft.note.trim(),
      wiki_url: draft.wiki_url?.trim() || undefined,
    }
    void save(replace(editing ?? entry.source, entry), () => {
      setDraft(null)
      setView(blocked(entry) ? 'blocked' : 'terms')
    }).catch(() => undefined)
  }

  const entryRows = (protectedEntries: boolean) => {
    const rows = entries.filter((entry) => blocked(entry) === protectedEntries && matches(entry))
    if (!rows.length)
      return <p className='p-5 text-sm text-muted-foreground'>{t('glossary.noEntries')}</p>
    return (
      <ul className='divide-y'>
        {rows.map((entry) => (
          <li className='flex items-start gap-3 px-4 py-3' key={entry.source}>
            <div className='grid min-w-0 flex-1 gap-1 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)] sm:gap-3'>
              <span className='font-medium wrap-anywhere'>{entry.source}</span>
              <div className='min-w-0'>
                <span className='wrap-anywhere'>
                  {entry.translation || t(`glossary.kinds.${entry.kind}`)}
                </span>
                {entry.note && (
                  <p className='text-xs wrap-anywhere text-muted-foreground'>{entry.note}</p>
                )}
                {entry.wiki_url && /^https?:\/\//i.test(entry.wiki_url) && (
                  <button
                    type='button'
                    className='inline-flex items-center gap-1 text-xs text-primary underline'
                    onClick={() => void openUrl(entry.wiki_url!).catch(() => undefined)}
                  >
                    <ExternalLink className='size-3' /> {t('review.wiki')}
                  </button>
                )}
              </div>
            </div>
            <Button
              type='button'
              size='icon-xs'
              variant='ghost'
              disabled={locked}
              aria-label={t('glossary.edit')}
              onClick={() => {
                setEditing(entry.source)
                setDraft(entry)
              }}
            >
              <Pencil />
            </Button>
            <Button
              type='button'
              size='icon-xs'
              variant='ghost'
              disabled={locked}
              aria-label={t('glossary.remove')}
              onClick={() =>
                void save(entries.filter((item) => item.source !== entry.source)).catch(
                  () => undefined,
                )
              }
            >
              <Trash2 />
            </Button>
          </li>
        ))}
      </ul>
    )
  }

  const visibleCandidates = candidates?.filter((group) => group.entries.some(matches))
  const activeEntries = new Map(entries.map((entry) => [entry.source, entry]))

  return (
    <Tabs
      value={view}
      onValueChange={(next) => {
        setView(next as View)
        setDraft(null)
      }}
      className='min-h-0 flex-1 gap-3 text-sm'
    >
      <div className='flex shrink-0 flex-wrap items-center justify-between gap-2'>
        <TabsList variant='line' className='px-0'>
          <TabsTrigger value='terms' className='px-2.5'>
            {t('glossary.title')}
          </TabsTrigger>
          <TabsTrigger value='blocked' className='px-2.5'>
            {t('glossary.blockedTable')}
          </TabsTrigger>
          {candidates && (
            <TabsTrigger value='candidates' className='px-2.5'>
              {t('glossary.candidates')}
            </TabsTrigger>
          )}
        </TabsList>
        <div className='flex min-w-0 gap-2'>
          <label className='relative min-w-0'>
            <Search className='pointer-events-none absolute top-2 left-2 size-4 text-muted-foreground' />
            <Input
              type='search'
              aria-label={t('glossary.search')}
              placeholder={t('glossary.search')}
              className='w-40 ps-8 sm:w-52'
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </label>
          {view !== 'candidates' && (
            <Button
              size='sm'
              disabled={locked}
              onClick={() => {
                setEditing(null)
                setDraft({
                  source: '',
                  translation: '',
                  kind: view === 'blocked' ? 'preserve' : 'term',
                  note: '',
                })
              }}
            >
              <Plus />
              {t('glossary.add')}
            </Button>
          )}
        </div>
      </div>
      {draft && (
        <form
          onSubmit={submit}
          className='grid shrink-0 gap-2 rounded-lg border bg-muted/25 p-3 sm:grid-cols-2'
        >
          <Input
            autoFocus
            aria-label={t('glossary.source')}
            placeholder={t('glossary.source')}
            value={draft.source}
            onChange={(event) => setDraft({ ...draft, source: event.target.value })}
          />
          <NativeSelect
            aria-label={t('glossary.kind')}
            className='w-full'
            value={draft.kind}
            onChange={(event) => setDraft({ ...draft, kind: event.target.value as GlossaryKind })}
          >
            {kinds.map((kind) => (
              <NativeSelectOption key={kind} value={kind}>
                {t(`glossary.kinds.${kind}`)}
              </NativeSelectOption>
            ))}
          </NativeSelect>
          {!blocked(draft) && (
            <Input
              aria-label={t('glossary.translation')}
              placeholder={t('glossary.translation')}
              value={draft.translation}
              onChange={(event) => setDraft({ ...draft, translation: event.target.value })}
            />
          )}
          <Input
            aria-label={t('glossary.note')}
            placeholder={t('glossary.note')}
            value={draft.note}
            onChange={(event) => setDraft({ ...draft, note: event.target.value })}
          />
          <Input
            type='url'
            pattern='[Hh][Tt][Tt][Pp][Ss]?://.+'
            aria-label={t('review.wiki')}
            placeholder={t('review.wiki')}
            value={draft.wiki_url ?? ''}
            onChange={(event) => setDraft({ ...draft, wiki_url: event.target.value })}
          />
          <div className='flex flex-wrap items-center justify-end gap-2 sm:col-span-2'>
            {duplicate && (
              <span className='me-auto text-xs text-destructive'>{t('glossary.duplicate')}</span>
            )}
            <Button type='button' size='sm' variant='ghost' onClick={() => setDraft(null)}>
              {t('glossary.cancel')}
            </Button>
            <Button
              type='submit'
              size='sm'
              disabled={
                locked ||
                Boolean(duplicate) ||
                !draft.source.trim() ||
                (!blocked(draft) && !draft.translation.trim())
              }
            >
              {t(editing ? 'glossary.save' : 'glossary.add')}
            </Button>
          </div>
        </form>
      )}
      <TabsContent value='terms' className='min-h-0 overflow-y-auto rounded-lg border'>
        {entryRows(false)}
      </TabsContent>
      <TabsContent value='blocked' className='min-h-0 overflow-y-auto rounded-lg border'>
        {entryRows(true)}
      </TabsContent>
      {candidates && (
        <TabsContent value='candidates' className='min-h-0 overflow-y-auto rounded-lg border'>
          {!visibleCandidates?.length && (
            <p className='p-5 text-muted-foreground'>{t('glossary.noCandidates')}</p>
          )}
          {visibleCandidates?.map(({ entries: choices, locations }) => {
            const source = choices[0].source
            const active = activeEntries.get(source)
            return (
              <details key={source} className='border-b px-4 py-3 last:border-b-0'>
                <summary className='cursor-pointer'>
                  <span className='font-medium wrap-anywhere'>{source}</span>
                  <span className='ms-2 text-xs text-muted-foreground'>
                    {t('glossary.occurrences', { count: locations.length })}
                  </span>
                </summary>
                <div className='mt-3 flex flex-wrap gap-2'>
                  {choices.map((entry) => {
                    const selected =
                      active?.kind === entry.kind && active.translation === entry.translation
                    return (
                      <Button
                        key={`${entry.kind}:${entry.translation}`}
                        size='sm'
                        variant={selected ? 'secondary' : 'outline'}
                        disabled={locked}
                        aria-pressed={selected}
                        className='h-auto min-h-8 wrap-anywhere whitespace-normal'
                        onClick={() => {
                          if (!selected) {
                            void save(
                              replace(source, {
                                ...entry,
                                excluded_layers: active?.excluded_layers,
                              }),
                            ).catch(() => undefined)
                          }
                        }}
                      >
                        {selected && <Check aria-hidden='true' />}
                        {entry.translation || t(`glossary.kinds.${entry.kind}`)}
                      </Button>
                    )
                  })}
                </div>
                {onLocation &&
                  locations.map((location) => {
                    const excluded = active?.excluded_layers?.includes(location.layer)
                    return (
                      <div key={location.layer} className='mt-2 flex items-start gap-2'>
                        <button
                          type='button'
                          onClick={() => onLocation(location)}
                          className='min-w-0 flex-1 px-1 py-1 text-start text-xs wrap-anywhere text-muted-foreground hover:text-foreground'
                        >
                          {t('glossary.page', { page: location.number })} · {location.text}
                        </button>
                        {active && (
                          <Button
                            type='button'
                            size='xs'
                            variant='outline'
                            disabled={locked}
                            onClick={() => toggleLocation(active, location)}
                          >
                            {t(excluded ? 'glossary.restoreLine' : 'glossary.excludeLine')}
                          </Button>
                        )}
                      </div>
                    )
                  })}
              </details>
            )
          })}
        </TabsContent>
      )}
    </Tabs>
  )
}
