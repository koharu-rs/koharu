'use client'

import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { glossaryTranslationBatches } from '@/lib/glossary'
import { pageKey, pagesKey, projectKey, refresh } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import {
  commands,
  type GlossaryCategory,
  type GlossaryDocument,
  type GlossaryEntry,
  type ProjectGlossary,
} from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@koharu/ui/components/dialog'
import { Input } from '@koharu/ui/components/input'

const categories: GlossaryCategory[] = [
  'person',
  'place',
  'organization',
  'title',
  'skill',
  'item',
  'terminology',
  'other',
]
const pageSize = 50
// Batches are independent requests; a few in flight keeps hosted models busy.
const aiTranslationWorkers = 3

export function ProjectGlossaryDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const jobs = useKoharuStore((state) => state.jobs)
  const awaiting = Object.values(jobs).find((job) => job.state === 'awaiting_review')
  const running = Object.values(jobs).some(
    (job) => job.kind !== 'export' && job.state === 'running',
  )
  const [document, setDocument] = useState<GlossaryDocument | null>(null)
  const [draft, setDraft] = useState<Required<ProjectGlossary> | null>(null)
  const [tab, setTab] = useState<'confirmed' | 'candidates'>('confirmed')
  const [page, setPage] = useState(0)
  const [busy, setBusy] = useState(false)
  const [closeWarning, setCloseWarning] = useState(false)
  const [aiProgress, setAiProgress] = useState<{ completed: number; total: number } | null>(null)
  const [aiStopping, setAiStopping] = useState(false)
  const aiStop = useRef(false)
  const mounted = useRef(true)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      aiStop.current = true
    }
  }, [])
  const dirty = Boolean(
    document && draft && JSON.stringify(document.glossary) !== JSON.stringify(draft),
  )
  const receive = (value: GlossaryDocument) => {
    setDocument(value)
    setDraft({ entries: [], candidates: [], ignored: [], ...value.glossary })
    setCloseWarning(false)
  }
  useEffect(() => {
    if (!open) return
    let active = true
    void call(commands.getProjectGlossary)
      .then((value) => {
        if (active) {
          receive(value)
          setTab((value.glossary.candidates?.length ?? 0) ? 'candidates' : 'confirmed')
          setPage(0)
        }
      })
      .catch(() => undefined)
    return () => {
      active = false
    }
  }, [open, awaiting?.id])
  const perform = (action: () => Promise<unknown>) => {
    setBusy(true)
    void action()
      .catch(() => undefined)
      .finally(() => setBusy(false))
  }
  const translateCandidates = async () => {
    if (!document || !draft) return
    const batches = glossaryTranslationBatches(draft.candidates)
    const total = batches.reduce((sum, batch) => sum + batch.length, 0)
    let completed = 0
    aiStop.current = false
    setAiStopping(false)
    setAiProgress({ completed, total })
    const queue = [...batches]
    const worker = async () => {
      while (!aiStop.current && mounted.current) {
        const batch = queue.shift()
        if (!batch) return
        const translations = await call(
          commands.suggestGlossaryTranslations,
          document.project,
          batch.map((entry) => entry.source),
          draft.entries,
        )
        if (!mounted.current) return
        setDraft((current) =>
          current
            ? {
                ...current,
                candidates: current.candidates.map((candidate, index) => {
                  const position = batch.findIndex(
                    (entry) => entry.index === index && entry.source === candidate.source,
                  )
                  return position >= 0 &&
                    !candidate.suggested_target.trim() &&
                    translations[position]?.trim()
                    ? { ...candidate, suggested_target: translations[position].trim() }
                    : candidate
                }),
              }
            : current,
        )
        completed += batch.length
        setAiProgress({ completed, total })
      }
    }
    try {
      await Promise.allSettled(
        Array.from({ length: Math.min(aiTranslationWorkers, batches.length) }, worker),
      )
    } finally {
      if (mounted.current) setAiProgress(null)
    }
  }
  const save = async () => {
    if (!document || !draft) return
    if (dirty) receive(await call(commands.saveProjectGlossary, document, draft))
    await refresh(projectKey, pagesKey, pageKey)
  }
  const edit = (index: number, value: Partial<GlossaryEntry>) => {
    if (draft)
      setDraft({
        ...draft,
        entries: draft.entries.map((entry, item) =>
          item === index ? { ...entry, ...value } : entry,
        ),
      })
  }
  const confirm = (indices: number[]) => {
    if (!draft) return
    const selected = new Set(
      indices.filter((index) => draft.candidates[index].suggested_target.trim()),
    )
    setDraft({
      ...draft,
      entries: [
        ...draft.entries,
        ...draft.candidates
          .filter((_, index) => selected.has(index))
          .map((candidate) => ({
            source: candidate.source,
            target: candidate.suggested_target,
            category: candidate.category,
            notes: '',
            enabled: true,
          })),
      ],
      candidates: draft.candidates.filter((_, index) => !selected.has(index)),
    })
  }
  const count = draft ? (tab === 'confirmed' ? draft.entries.length : draft.candidates.length) : 0
  const currentPage = Math.min(page, Math.max(0, Math.ceil(count / pageSize) - 1))
  const category = (
    value: GlossaryCategory,
    onChange: (category: GlossaryCategory) => void,
    label: string,
  ) => (
    <select
      className='w-full rounded border bg-background p-1 text-xs'
      value={value}
      aria-label={label}
      onChange={(event) => onChange(event.target.value as GlossaryCategory)}
    >
      {categories.map((category) => (
        <option key={category} value={category}>
          {t(`glossary.categories.${category}`)}
        </option>
      ))}
    </select>
  )
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && (dirty || busy)) {
          setCloseWarning(true)
          return
        }
        onOpenChange(next)
      }}
    >
      <DialogContent className='max-h-[85vh] w-[95vw] max-w-5xl overflow-y-auto'>
        <DialogHeader>
          <DialogTitle>{t('glossary.title')}</DialogTitle>
          <DialogDescription>{t('glossary.description')}</DialogDescription>
        </DialogHeader>
        {running && <p className='text-xs text-muted-foreground'>{t('glossary.running')}</p>}
        {closeWarning && (
          <p role='alert' className='text-xs text-destructive'>
            {t('glossary.unsaved')}
          </p>
        )}
        {draft && (
          <>
            <div className='flex flex-wrap gap-2'>
              {(['confirmed', 'candidates'] as const).map((value) => (
                <Button
                  key={value}
                  variant={tab === value ? 'default' : 'outline'}
                  onClick={() => {
                    setTab(value)
                    setPage(0)
                  }}
                >
                  {t(`glossary.${value}`)} (
                  {value === 'confirmed' ? draft.entries.length : draft.candidates.length})
                </Button>
              ))}
              <Button
                variant='ghost'
                className='ml-auto'
                disabled={busy || dirty}
                onClick={() =>
                  perform(async () => receive(await call(commands.getProjectGlossary)))
                }
              >
                {t('glossary.reload')}
              </Button>
            </div>
            {aiProgress && (
              <div role='status' className='flex items-center justify-between gap-3 text-xs'>
                <span>{t('glossary.aiProgress', aiProgress)}</span>
                <Button
                  variant='outline'
                  size='sm'
                  disabled={aiStopping}
                  onClick={() => {
                    aiStop.current = true
                    setAiStopping(true)
                  }}
                >
                  {t(aiStopping ? 'glossary.aiStopping' : 'glossary.aiStop')}
                </Button>
              </div>
            )}
            <fieldset disabled={busy || running} className='min-w-0'>
              <div className='overflow-x-auto'>
                {tab === 'confirmed' ? (
                  <table className='w-full text-left text-xs'>
                    <thead>
                      <tr>
                        {['source', 'target', 'category', 'notes', 'enabled', 'actions'].map(
                          (key) => (
                            <th key={key} className='p-1 font-medium'>
                              {t(`glossary.${key}`)}
                            </th>
                          ),
                        )}
                      </tr>
                    </thead>
                    <tbody>
                      {draft.entries
                        .slice(currentPage * pageSize, (currentPage + 1) * pageSize)
                        .map((entry, offset) => {
                          const index = currentPage * pageSize + offset
                          return (
                            <tr key={index}>
                              <td className='p-1'>
                                <Input
                                  aria-label={`${t('glossary.source')} ${index + 1}`}
                                  value={entry.source}
                                  onChange={(event) => edit(index, { source: event.target.value })}
                                />
                              </td>
                              <td className='p-1'>
                                <Input
                                  aria-label={`${t('glossary.target')} ${index + 1}`}
                                  value={entry.target}
                                  onChange={(event) => edit(index, { target: event.target.value })}
                                />
                              </td>
                              <td className='p-1'>
                                {category(
                                  entry.category,
                                  (category) => edit(index, { category }),
                                  `${t('glossary.category')} ${index + 1}`,
                                )}
                              </td>
                              <td className='p-1'>
                                <Input
                                  aria-label={`${t('glossary.notes')} ${index + 1}`}
                                  value={entry.notes}
                                  onChange={(event) => edit(index, { notes: event.target.value })}
                                />
                              </td>
                              <td className='p-1'>
                                <input
                                  type='checkbox'
                                  aria-label={`${t('glossary.enabled')} ${index + 1}`}
                                  checked={entry.enabled}
                                  onChange={(event) =>
                                    edit(index, { enabled: event.target.checked })
                                  }
                                />
                              </td>
                              <td className='p-1'>
                                <Button
                                  size='sm'
                                  variant='ghost'
                                  onClick={() =>
                                    setDraft({
                                      ...draft,
                                      entries: draft.entries.filter((_, item) => item !== index),
                                    })
                                  }
                                >
                                  {t('glossary.delete')}
                                </Button>
                              </td>
                            </tr>
                          )
                        })}
                    </tbody>
                  </table>
                ) : (
                  <table className='w-full text-left text-xs'>
                    <thead>
                      <tr>
                        {['source', 'count', 'pages', 'suggestion', 'category', 'actions'].map(
                          (key) => (
                            <th key={key} className='p-1 font-medium'>
                              {t(`glossary.${key}`)}
                            </th>
                          ),
                        )}
                      </tr>
                    </thead>
                    <tbody>
                      {draft.candidates
                        .slice(currentPage * pageSize, (currentPage + 1) * pageSize)
                        .map((candidate, offset) => {
                          const index = currentPage * pageSize + offset
                          const update = (value: Partial<typeof candidate>) =>
                            setDraft({
                              ...draft,
                              candidates: draft.candidates.map((entry, item) =>
                                item === index ? { ...entry, ...value } : entry,
                              ),
                            })
                          return (
                            <tr key={index}>
                              <td className='p-1'>
                                <Input
                                  aria-label={`${t('glossary.source')} ${index + 1}`}
                                  value={candidate.source}
                                  onChange={(event) => update({ source: event.target.value })}
                                />
                              </td>
                              <td className='p-1 tabular-nums'>{candidate.occurrences}</td>
                              <td className='p-1 tabular-nums'>{candidate.page_count}</td>
                              <td className='p-1'>
                                <Input
                                  aria-label={`${t('glossary.target')} ${index + 1}`}
                                  value={candidate.suggested_target}
                                  onChange={(event) =>
                                    update({ suggested_target: event.target.value })
                                  }
                                />
                              </td>
                              <td className='p-1'>
                                {category(
                                  candidate.category,
                                  (category) => update({ category }),
                                  `${t('glossary.category')} ${index + 1}`,
                                )}
                              </td>
                              <td className='flex gap-1 p-1'>
                                <Button
                                  size='sm'
                                  variant='outline'
                                  disabled={!candidate.suggested_target.trim()}
                                  onClick={() => confirm([index])}
                                >
                                  {t('glossary.confirm')}
                                </Button>
                                <Button
                                  size='sm'
                                  variant='ghost'
                                  onClick={() =>
                                    setDraft({
                                      ...draft,
                                      ignored: [...draft.ignored, candidate.source],
                                      candidates: draft.candidates.filter(
                                        (_, item) => item !== index,
                                      ),
                                    })
                                  }
                                >
                                  {t('glossary.ignore')}
                                </Button>
                              </td>
                            </tr>
                          )
                        })}
                    </tbody>
                  </table>
                )}
              </div>
              {!count && (
                <p className='py-6 text-center text-xs text-muted-foreground'>
                  {t('glossary.empty')}
                </p>
              )}
              <div className='mt-3 flex flex-wrap gap-2'>
                {tab === 'candidates' && (
                  <Button
                    variant='outline'
                    disabled={
                      !draft.candidates.some(
                        (candidate) =>
                          !candidate.suggested_target.trim() && candidate.source.trim(),
                      )
                    }
                    onClick={() => perform(translateCandidates)}
                  >
                    {t('glossary.aiTranslate')}
                  </Button>
                )}
                {tab === 'confirmed' ? (
                  <Button
                    variant='outline'
                    onClick={() => {
                      setDraft({
                        ...draft,
                        entries: [
                          ...draft.entries,
                          { source: '', target: '', category: 'other', notes: '', enabled: true },
                        ],
                      })
                      setPage(Math.floor(draft.entries.length / pageSize))
                    }}
                  >
                    {t('glossary.add')}
                  </Button>
                ) : (
                  <Button
                    variant='outline'
                    disabled={
                      !draft.candidates.some((candidate) => candidate.suggested_target.trim())
                    }
                    onClick={() => confirm(draft.candidates.map((_, index) => index))}
                  >
                    {t('glossary.bulkConfirm')}
                  </Button>
                )}
              </div>
            </fieldset>
            {count > pageSize && (
              <div className='flex items-center justify-end gap-2 text-xs'>
                <Button
                  variant='ghost'
                  disabled={currentPage === 0}
                  onClick={() => setPage(currentPage - 1)}
                >
                  {t('glossary.previous')}
                </Button>
                <span>
                  {currentPage + 1} / {Math.ceil(count / pageSize)}
                </span>
                <Button
                  variant='ghost'
                  disabled={(currentPage + 1) * pageSize >= count}
                  onClick={() => setPage(currentPage + 1)}
                >
                  {t('glossary.next')}
                </Button>
              </div>
            )}
            {tab === 'candidates' && (
              <p className='text-xs text-muted-foreground'>{t('glossary.aiDescription')}</p>
            )}
            <div className='flex flex-wrap gap-2 border-t pt-3'>
              <Button
                variant='outline'
                disabled={busy || running || dirty || !document}
                onClick={() =>
                  perform(async () => {
                    if (!document) return
                    const imported = await call(commands.importProjectGlossary, document)
                    if (imported) receive(imported)
                  })
                }
              >
                {t('glossary.import')}
              </Button>
              <Button
                variant='outline'
                disabled={busy || dirty}
                onClick={() => perform(() => call(commands.exportProjectGlossary))}
              >
                {t('glossary.export')}
              </Button>
              {dirty && (
                <Button
                  variant='ghost'
                  disabled={busy}
                  onClick={() => {
                    if (document) receive(document)
                    onOpenChange(false)
                  }}
                >
                  {t('glossary.discard')}
                </Button>
              )}
              <Button
                className='ml-auto'
                disabled={busy || running || !dirty}
                onClick={() => perform(save)}
              >
                {t('glossary.save')}
              </Button>
              {awaiting && (
                <Button
                  disabled={busy || running}
                  onClick={() =>
                    perform(async () => {
                      await save()
                      await call(commands.resumeWorkflow, awaiting.id)
                      onOpenChange(false)
                    })
                  }
                >
                  {t('glossary.continue')}
                </Button>
              )}
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
