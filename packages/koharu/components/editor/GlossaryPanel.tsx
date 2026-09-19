'use client'

import {
  BookOpenText,
  CheckSquare2,
  Download,
  FileUp,
  Languages,
  MoreHorizontal,
  Plus,
  ScanSearch,
  Search,
  Trash2,
  X,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { GlossaryEntryRow } from '@/components/editor/GlossaryEntryRow'
import { GlossaryImportDialog } from '@/components/editor/GlossaryImportDialog'
import {
  eligibleGlossaryTranslationIds,
  filterGlossaryEntries,
  glossaryKinds,
  glossaryStatus,
  parseGlossaryDocument,
  type GlossaryStateFilter,
} from '@/lib/glossary'
import {
  useAddGlossaryEntry,
  useApplyGlossaryImport,
  useDeleteGlossaryEntries,
  useExportGlossary,
  useGlossary,
  usePreviewGlossaryImport,
  useProject,
  useScanGlossary,
  useSetGlossaryEnabled,
  useTranslateGlossaryEntries,
  useUpdateGlossaryEntry,
} from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import type {
  GlossaryEntryDraft,
  GlossaryImportStrategy,
  GlossaryKind,
  Job,
} from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import { Checkbox } from '@koharu/ui/components/checkbox'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@koharu/ui/components/dropdown-menu'
import { Input } from '@koharu/ui/components/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'
import { Switch } from '@koharu/ui/components/switch'

export function GlossaryPanel() {
  const { t } = useTranslation()
  const project = useProject().data
  const glossaryQuery = useGlossary(Boolean(project))
  const glossary = glossaryQuery.data
  const jobs = useKoharuStore((state) => state.jobs)
  const [query, setQuery] = useState('')
  const [kind, setKind] = useState<GlossaryKind | 'all'>('all')
  const [state, setState] = useState<GlossaryStateFilter>('all')
  const [selected, setSelected] = useState<Set<string>>(() => new Set())
  const [adding, setAdding] = useState(false)
  const [importError, setImportError] = useState<string | null>(null)
  const [importDocument, setImportDocument] = useState<string | null>(null)
  const [importOpen, setImportOpen] = useState(false)
  const fileInput = useRef<HTMLInputElement>(null)

  const scan = useScanGlossary()
  const toggle = useSetGlossaryEnabled()
  const add = useAddGlossaryEntry()
  const update = useUpdateGlossaryEntry(glossary?.revision ?? 0, project?.name ?? '')
  const remove = useDeleteGlossaryEntries()
  const translate = useTranslateGlossaryEntries()
  const previewImport = usePreviewGlossaryImport()
  const applyImport = useApplyGlossaryImport()
  const exportGlossary = useExportGlossary()

  const activeJob = Object.values(jobs).find((job) => job.state === 'running')
  const activeGlossaryJob = Object.values(jobs).find(
    (job) =>
      job.state === 'running' &&
      (job.kind === 'glossary_scan' || job.kind === 'glossary_translation'),
  )
  const mutationPending =
    scan.isPending ||
    toggle.isPending ||
    add.isPending ||
    update.isPending ||
    remove.isPending ||
    translate.isPending ||
    applyImport.isPending ||
    exportGlossary.isPending
  const processing = Boolean(activeJob) || mutationPending

  useEffect(() => {
    if (!glossary) return
    const ids = new Set(glossary.entries.map((entry) => entry.id))
    setSelected((current) => new Set([...current].filter((id) => ids.has(id))))
  }, [glossary])

  const filtered = useMemo(
    () =>
      filterGlossaryEntries(glossary?.entries ?? [], {
        query,
        kind,
        state,
      }),
    [glossary?.entries, kind, query, state],
  )
  const selectedEligible = glossary
    ? eligibleGlossaryTranslationIds(glossary.entries, selected)
    : []
  const allEligible = glossary ? eligibleGlossaryTranslationIds(glossary.entries) : []

  if (!project) {
    return <Unavailable />
  }
  if (glossaryQuery.isError) {
    return (
      <div role='alert' className='p-4 text-xs text-destructive'>
        {t('glossary.loadError')}
      </div>
    )
  }
  if (glossaryQuery.isLoading || !glossary) {
    return (
      <div
        role='status'
        className='flex h-full items-center justify-center text-xs text-muted-foreground'
      >
        {t('common.loading')}
      </div>
    )
  }

  const status = glossaryStatus(glossary)
  const statusLabel = t(`glossary.status.${status}`)
  const scanLabel = status === 'unscanned' ? t('glossary.scan') : t('glossary.rescan')

  const deleteSelected = () => {
    const ids = [...selected]
    if (ids.length === 0) return
    remove.mutate({ revision: glossary.revision, ids }, { onSuccess: () => setSelected(new Set()) })
  }

  const translateIds = (ids: string[]) => {
    if (ids.length === 0) return
    translate.mutate({ revision: glossary.revision, ids })
  }

  const readImport = async (file: File | undefined) => {
    if (!file) return
    setImportError(null)
    let document: string
    try {
      document = parseGlossaryDocument(await file.text())
    } catch {
      setImportError(t('glossary.importMalformed'))
      if (fileInput.current) fileInput.current.value = ''
      return
    }
    try {
      await previewImport.mutateAsync(document)
      setImportDocument(document)
      setImportOpen(true)
    } catch {
      setImportError(t('glossary.importFailed'))
    } finally {
      if (fileInput.current) fileInput.current.value = ''
    }
  }

  const apply = async (strategy: GlossaryImportStrategy, confirmLanguageMismatch: boolean) => {
    if (!importDocument) return
    await applyImport.mutateAsync({
      revision: glossary.revision,
      document: importDocument,
      strategy,
      confirmLanguageMismatch,
    })
    setImportOpen(false)
    setImportDocument(null)
  }

  return (
    <section className='flex h-full min-h-0 flex-col' aria-label={t('glossary.title')}>
      <header className='shrink-0 border-b border-border/70 p-2.5'>
        <div className='flex items-center gap-2'>
          <Switch
            size='sm'
            aria-label={t('glossary.enable')}
            checked={glossary.enabled}
            disabled={processing}
            onCheckedChange={(enabled) => toggle.mutate({ revision: glossary.revision, enabled })}
          />
          <div className='min-w-0 flex-1'>
            <p className='truncate text-[11px] font-semibold'>{t('glossary.title')}</p>
            <p
              className={`text-[9px] ${status === 'stale' ? 'text-amber-600 dark:text-amber-400' : 'text-muted-foreground'}`}
            >
              {statusLabel}
            </p>
          </div>
          <Button
            size='sm'
            variant={status === 'stale' ? 'secondary' : 'outline'}
            className='h-7 px-2 text-[10px]'
            disabled={processing}
            aria-label={scanLabel}
            onClick={() => scan.mutate()}
          >
            <ScanSearch className='size-3' />
            {scanLabel}
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button
                  size='icon-sm'
                  variant='ghost'
                  aria-label={t('glossary.actions')}
                  disabled={processing}
                />
              }
            >
              <MoreHorizontal className='size-3.5' />
            </DropdownMenuTrigger>
            <DropdownMenuContent align='end' className='w-52'>
              <DropdownMenuItem onClick={() => setAdding(true)}>
                <Plus /> {t('glossary.add')}
              </DropdownMenuItem>
              <DropdownMenuItem
                disabled={allEligible.length === 0}
                onClick={() => translateIds(allEligible)}
              >
                <Languages /> {t('glossary.translateUntranslated')}
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem onClick={() => fileInput.current?.click()}>
                <FileUp /> {t('glossary.import')}
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => exportGlossary.mutate()}>
                <Download /> {t('glossary.export')}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <input
            ref={fileInput}
            hidden
            type='file'
            accept='application/json,.json'
            onChange={(event) => void readImport(event.currentTarget.files?.[0])}
          />
        </div>
        <WorkflowStatus job={activeGlossaryJob} scanned={status === 'available'} />
        {importError ? (
          <div role='alert' className='mt-2 flex items-start gap-1.5 text-[10px] text-destructive'>
            <span className='min-w-0 flex-1'>{importError}</span>
            <Button
              size='icon-xs'
              variant='ghost'
              aria-label={t('glossary.dismissImportError')}
              onClick={() => setImportError(null)}
            >
              <X className='size-3' />
            </Button>
          </div>
        ) : null}
      </header>

      <div className='shrink-0 space-y-2 border-b border-border/70 p-2.5'>
        <div className='relative'>
          <Search className='pointer-events-none absolute top-1/2 left-2 size-3 -translate-y-1/2 text-muted-foreground' />
          <Input
            type='search'
            aria-label={t('glossary.search')}
            placeholder={t('glossary.searchPlaceholder')}
            value={query}
            className='h-7 pr-2 pl-7 text-[11px]'
            onChange={(event) => setQuery(event.currentTarget.value)}
          />
        </div>
        <div className='grid grid-cols-2 gap-2'>
          <Select
            value={kind}
            items={{
              all: t('glossary.allCategories'),
              ...Object.fromEntries(
                glossaryKinds.map((item) => [item, t(`glossary.kinds.${item}`)]),
              ),
            }}
            onValueChange={(value) => value && setKind(value as GlossaryKind | 'all')}
          >
            <SelectTrigger size='sm' className='w-full' aria-label={t('glossary.categoryFilter')}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value='all'>{t('glossary.allCategories')}</SelectItem>
              {glossaryKinds.map((item) => (
                <SelectItem key={item} value={item}>
                  {t(`glossary.kinds.${item}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={state}
            items={Object.fromEntries(
              ['all', 'translated', 'untranslated', 'disabled', 'not-present'].map((item) => [
                item,
                t(`glossary.filters.${item}`),
              ]),
            )}
            onValueChange={(value) => value && setState(value as GlossaryStateFilter)}
          >
            <SelectTrigger size='sm' className='w-full' aria-label={t('glossary.stateFilter')}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(['all', 'translated', 'untranslated', 'disabled', 'not-present'] as const).map(
                (item) => (
                  <SelectItem key={item} value={item}>
                    {t(`glossary.filters.${item}`)}
                  </SelectItem>
                ),
              )}
            </SelectContent>
          </Select>
        </div>
      </div>

      {selected.size > 0 ? (
        <div className='flex shrink-0 items-center gap-1.5 border-b bg-muted/30 px-2.5 py-1.5'>
          <span className='min-w-0 flex-1 text-[10px] text-muted-foreground'>
            {t('glossary.selectedCount', { count: selected.size })}
          </span>
          <Button
            size='sm'
            variant='ghost'
            className='h-6 px-1.5 text-[9px]'
            disabled={processing || selectedEligible.length === 0}
            onClick={() => translateIds(selectedEligible)}
          >
            <Languages className='size-3' /> {t('glossary.translateSelected')}
          </Button>
          <Button
            size='sm'
            variant='ghost'
            className='h-6 px-1.5 text-[9px] text-destructive'
            disabled={processing}
            aria-label={t('glossary.deleteSelected', { count: selected.size })}
            onClick={deleteSelected}
          >
            <Trash2 className='size-3' />
          </Button>
        </div>
      ) : null}

      <div className='min-h-0 flex-1 overflow-y-auto p-2.5'>
        {adding ? (
          <ManualEntryForm
            disabled={processing}
            onCancel={() => setAdding(false)}
            onSave={(draft) =>
              add.mutate(
                { revision: glossary.revision, draft },
                { onSuccess: () => setAdding(false) },
              )
            }
          />
        ) : null}
        {glossary.entries.length > 0 ? (
          <label className='mb-2 flex items-center gap-2 px-0.5 text-[10px] text-muted-foreground'>
            <Checkbox
              checked={filtered.length > 0 && filtered.every((entry) => selected.has(entry.id))}
              disabled={processing || filtered.length === 0}
              onCheckedChange={(checked) => {
                setSelected((current) => {
                  const next = new Set(current)
                  for (const entry of filtered) {
                    if (checked === true) next.add(entry.id)
                    else next.delete(entry.id)
                  }
                  return next
                })
              }}
            />
            <CheckSquare2 className='size-3' /> {t('glossary.selectVisible')}
          </label>
        ) : null}
        <div className='space-y-2'>
          {filtered.map((entry) => (
            <GlossaryEntryRow
              key={entry.id}
              entry={entry}
              selected={selected.has(entry.id)}
              disabled={processing}
              onSelect={(checked) =>
                setSelected((current) => {
                  const next = new Set(current)
                  if (checked) next.add(entry.id)
                  else next.delete(entry.id)
                  return next
                })
              }
              onUpdate={(patch) => update.mutate({ id: entry.id, patch })}
              onDelete={() => remove.mutate({ revision: glossary.revision, ids: [entry.id] })}
            />
          ))}
        </div>
        {filtered.length === 0 && !adding ? (
          <div className='flex min-h-40 flex-col items-center justify-center gap-2 px-4 text-center'>
            <BookOpenText className='size-5 text-muted-foreground' />
            <p className='text-[11px] font-medium'>
              {glossary.entries.length === 0 ? t('glossary.empty') : t('glossary.noResults')}
            </p>
            <p className='text-[10px] text-muted-foreground'>
              {glossary.entries.length === 0
                ? t('glossary.emptyDescription')
                : t('glossary.noResultsDescription')}
            </p>
          </div>
        ) : null}
      </div>

      <GlossaryImportDialog
        open={importOpen}
        preview={previewImport.data ?? null}
        pending={applyImport.isPending}
        onOpenChange={setImportOpen}
        onApply={(strategy, confirm) => void apply(strategy, confirm)}
      />
    </section>
  )
}

function WorkflowStatus({ job, scanned }: { job: Job | undefined; scanned: boolean }) {
  const { t } = useTranslation()
  if (!job) {
    return scanned ? (
      <p className='mt-2 text-[10px] text-muted-foreground'>{t('glossary.waitingConfirmation')}</p>
    ) : null
  }
  const phase =
    job.kind === 'glossary_translation'
      ? t('glossary.phase.translating')
      : job.phase.kind === 'preparing_ocr'
        ? t('glossary.phase.preparingOcr')
        : t('glossary.phase.extractingTerms')
  const percent =
    job.total > 0 ? Math.min(100, Math.round((job.completed / job.total) * 100)) : null
  return (
    <div role='status' className='mt-2 space-y-1'>
      <div className='flex items-center justify-between text-[10px]'>
        <span>{phase}</span>
        <span className='text-muted-foreground tabular-nums'>
          {percent === null ? null : `${percent}%`}
        </span>
      </div>
      <div
        role='progressbar'
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        className='h-1 overflow-hidden rounded-full bg-muted'
      >
        <div
          className={`h-full bg-primary ${percent === null ? 'w-1/2' : ''}`}
          style={percent === null ? undefined : { width: `${percent}%` }}
        />
      </div>
    </div>
  )
}

function ManualEntryForm({
  disabled,
  onCancel,
  onSave,
}: {
  disabled: boolean
  onCancel: () => void
  onSave: (draft: GlossaryEntryDraft) => void
}) {
  const { t } = useTranslation()
  const [source, setSource] = useState('')
  const [translation, setTranslation] = useState('')
  const [kind, setKind] = useState<GlossaryKind>('term')
  const sourceInput = useRef<HTMLInputElement>(null)
  useEffect(() => sourceInput.current?.focus(), [])

  return (
    <form
      className='mb-2 rounded-lg border border-primary/30 bg-primary/5 p-2.5'
      onSubmit={(event) => {
        event.preventDefault()
        if (!source.trim()) return
        onSave({
          source: source.trim(),
          translation: translation.trim() || null,
          kind,
          enabled: true,
        })
      }}
    >
      <div className='grid grid-cols-2 gap-1.5'>
        <Input
          ref={sourceInput}
          aria-label={t('glossary.newSource')}
          value={source}
          disabled={disabled}
          className='h-7 text-[11px]'
          onChange={(event) => setSource(event.currentTarget.value)}
        />
        <Input
          aria-label={t('glossary.newTranslation')}
          value={translation}
          disabled={disabled}
          className='h-7 text-[11px]'
          onChange={(event) => setTranslation(event.currentTarget.value)}
        />
      </div>
      <div className='mt-2 flex items-center gap-1.5'>
        <Select
          value={kind}
          items={Object.fromEntries(
            glossaryKinds.map((item) => [item, t(`glossary.kinds.${item}`)]),
          )}
          onValueChange={(value) => value && setKind(value as GlossaryKind)}
        >
          <SelectTrigger size='sm' className='mr-auto' aria-label={t('glossary.newCategory')}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {glossaryKinds.map((item) => (
              <SelectItem key={item} value={item}>
                {t(`glossary.kinds.${item}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button
          type='button'
          size='sm'
          variant='ghost'
          className='h-6 text-[10px]'
          onClick={onCancel}
        >
          {t('common.cancel')}
        </Button>
        <Button
          type='submit'
          size='sm'
          className='h-6 text-[10px]'
          disabled={disabled || !source.trim()}
        >
          {t('glossary.saveTerm')}
        </Button>
      </div>
    </form>
  )
}

function Unavailable() {
  const { t } = useTranslation()
  return (
    <div className='flex h-full flex-col items-center justify-center gap-2 p-6 text-center'>
      <BookOpenText className='size-5 text-muted-foreground' />
      <p className='text-xs font-medium'>{t('glossary.unavailable')}</p>
      <p className='text-[10px] text-muted-foreground'>{t('glossary.unavailableDescription')}</p>
    </div>
  )
}
