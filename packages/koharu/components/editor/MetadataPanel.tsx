'use client'

import { BookText, ChevronDown, Languages, LoaderCircle } from 'lucide-react'
import { useEffect, useRef, useState, type ComponentProps } from 'react'
import { useTranslation } from 'react-i18next'
import { useDebouncedCallback } from 'use-debounce'

import { Choice } from '@/components/controls/Choice'
import { CommitTextarea } from '@/components/controls/CommitTextarea'
import { NumberField } from '@/components/preferences/PreferenceFields'
import { useComicInfo, usePreferences, useProject, useSaveComicInfo } from '@/lib/queries'
import { isActiveJob, useKoharuStore } from '@/lib/store'
import {
  commands,
  type AgeRating,
  type ComicInfo,
  type LocalizedField,
  type Manga,
  type RunId,
  type YesNo,
} from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@koharu/ui/components/collapsible'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@koharu/ui/components/dialog'
import { Input } from '@koharu/ui/components/input'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { cn } from '@koharu/ui/lib/utils'

const EMPTY_COMIC_INFO: ComicInfo = {
  title: null,
  series: null,
  number: null,
  count: null,
  volume: null,
  alternate_series: null,
  alternate_number: null,
  alternate_count: null,
  summary: null,
  notes: null,
  year: null,
  month: null,
  day: null,
  writer: null,
  penciller: null,
  inker: null,
  colorist: null,
  letterer: null,
  cover_artist: null,
  editor: null,
  translator: null,
  publisher: null,
  imprint: null,
  genre: null,
  tags: null,
  web: null,
  page_count: null,
  language_iso: null,
  format: null,
  black_and_white: null,
  manga: null,
  characters: null,
  teams: null,
  locations: null,
  scan_information: null,
  story_arc: null,
  story_arc_number: null,
  series_group: null,
  age_rating: null,
  pages: [],
  community_rating: null,
  main_character_or_team: null,
  review: null,
  gtin: null,
}

type ProseKey =
  | 'title'
  | 'series'
  | 'alternate_series'
  | 'summary'
  | 'notes'
  | 'genre'
  | 'tags'
  | 'characters'
  | 'teams'
  | 'locations'
  | 'story_arc'
  | 'series_group'
  | 'main_character_or_team'
  | 'review'

type TextKey =
  | 'number'
  | 'alternate_number'
  | 'writer'
  | 'penciller'
  | 'inker'
  | 'colorist'
  | 'letterer'
  | 'cover_artist'
  | 'editor'
  | 'translator'
  | 'publisher'
  | 'imprint'
  | 'web'
  | 'language_iso'
  | 'format'
  | 'scan_information'
  | 'story_arc_number'
  | 'gtin'

type IntKey = 'volume' | 'count' | 'alternate_count' | 'year' | 'month' | 'day'

const PROSE_KEYS: readonly ProseKey[] = [
  'title',
  'series',
  'alternate_series',
  'summary',
  'notes',
  'genre',
  'tags',
  'characters',
  'teams',
  'locations',
  'story_arc',
  'series_group',
  'main_character_or_team',
  'review',
]

export function MetadataDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        onKeyDown={(event) => event.stopPropagation()}
        className='flex max-h-[85vh] flex-col gap-0 overflow-hidden p-0 sm:max-w-3xl'
      >
        {open && <MetadataPanel inDialog />}
      </DialogContent>
    </Dialog>
  )
}

export function MetadataPanel({ inDialog = false }: { inDialog?: boolean }) {
  const { t } = useTranslation()
  const project = useProject().data
  const { data: comicInfo, isLoading } = useComicInfo(Boolean(project))
  const { data: preferences } = usePreferences(Boolean(project))
  const saveComicInfo = useSaveComicInfo()
  const jobs = useKoharuStore((state) => state.jobs)
  const [moreOpen, setMoreOpen] = useState(false)
  const [activeRunId, setActiveRunId] = useState<RunId | null>(null)
  const [startingTranslation, setStartingTranslation] = useState(false)

  const info = comicInfo ?? EMPTY_COMIC_INFO
  const latestRef = useRef<ComicInfo>(info)
  latestRef.current = info
  const pendingSaveRef = useRef<Promise<unknown> | null>(null)
  const flushersRef = useRef(new Set<() => void>())

  const activeMetadataJob = Object.values(jobs).find(
    (job) =>
      isActiveJob(job) &&
      (job.id === activeRunId ||
        (job.stage === 'translate' && job.page === null) ||
        job.scope?.kind === 'metadata'),
  )
  const translating = startingTranslation || Boolean(activeMetadataJob)

  useEffect(() => {
    if (activeRunId !== null) {
      const tracked = jobs[String(activeRunId)]
      if (tracked && !isActiveJob(tracked)) {
        setActiveRunId(null)
      }
    }
  }, [activeRunId, jobs])

  const hasTranslatableFields = PROSE_KEYS.some(
    (key) => (info[key]?.original ?? '').trim().length > 0,
  )

  const commit = (next: ComicInfo) => {
    latestRef.current = next
    const promise = saveComicInfo.mutateAsync(next).catch(() => undefined)
    pendingSaveRef.current = promise
  }

  const updateProse = (key: ProseKey, part: 'original' | 'translated', nextValue: string) => {
    const current = latestRef.current
    const prev = current[key]
    const prevOriginal = prev?.original ?? ''
    const prevTranslated = prev?.translated ?? null
    let nextField: LocalizedField | null
    if (part === 'original') {
      if (nextValue === prevOriginal) return
      // Clear stale translation whenever the original text changes.
      nextField = nextValue.trim() === '' ? null : { original: nextValue, translated: null }
    } else {
      const translated = nextValue.trim() === '' ? null : nextValue
      if (prevOriginal.trim() === '' && translated === null) {
        nextField = null
      } else {
        nextField = { original: prevOriginal, translated }
      }
      if (translated === prevTranslated && prev) return
    }
    commit({ ...current, [key]: nextField })
  }

  const updateText = (key: TextKey, nextValue: string) => {
    const current = latestRef.current
    const normalized = nextValue.trim() === '' ? null : nextValue
    if (current[key] === normalized) return
    commit({ ...current, [key]: normalized })
  }

  const updateInt = (key: IntKey, nextValue: number | null) => {
    const current = latestRef.current
    const normalized = nextValue === null || Number.isNaN(nextValue) ? null : Math.trunc(nextValue)
    if (current[key] === normalized) return
    commit({ ...current, [key]: normalized })
  }

  const handleTranslate = async () => {
    setStartingTranslation(true)
    try {
      if (document.activeElement instanceof HTMLElement) {
        document.activeElement.blur()
      }
      for (const flush of flushersRef.current) flush()
      if (pendingSaveRef.current) {
        await pendingSaveRef.current
      }
      const runId = await commands.runPipeline('metadata', ['translate'], 'normal')
      setActiveRunId(runId)
    } catch {
      // Failure toast is handled by MutationCache / error handler.
    } finally {
      setStartingTranslation(false)
    }
  }

  const registerFlush = (flush: () => void) => {
    flushersRef.current.add(flush)
    return () => {
      flushersRef.current.delete(flush)
    }
  }

  const yesNoChoices: Record<YesNo, string> = {
    Unknown: t('metadata.yesNo.unknown'),
    No: t('metadata.yesNo.no'),
    Yes: t('metadata.yesNo.yes'),
  }

  const mangaChoices: Record<Manga, string> = {
    Unknown: t('metadata.manga.unknown'),
    No: t('metadata.manga.no'),
    Yes: t('metadata.manga.yes'),
    YesAndRightToLeft: t('metadata.manga.yesAndRightToLeft'),
  }

  const ageRatingChoices: Record<AgeRating, string> = {
    Unknown: t('metadata.ageRating.unknown'),
    'Adults Only 18+': t('metadata.ageRating.adultsOnly18Plus'),
    'Early Childhood': t('metadata.ageRating.earlyChildhood'),
    Everyone: t('metadata.ageRating.everyone'),
    'Everyone 10+': t('metadata.ageRating.everyone10Plus'),
    G: t('metadata.ageRating.g'),
    'Kids to Adults': t('metadata.ageRating.kidsToAdults'),
    M: t('metadata.ageRating.m'),
    'MA15+': t('metadata.ageRating.ma15Plus'),
    'Mature 17+': t('metadata.ageRating.mature17Plus'),
    PG: t('metadata.ageRating.pg'),
    'R18+': t('metadata.ageRating.r18Plus'),
    'Rating Pending': t('metadata.ageRating.ratingPending'),
    Teen: t('metadata.ageRating.teen'),
    'X18+': t('metadata.ageRating.x18Plus'),
  }

  const pageCount = project?.pages.length ?? info.page_count ?? 0
  const targetLanguage = preferences?.pipeline.translator?.target ?? 'en'

  return (
    <div className='flex min-h-0 flex-1 flex-col overflow-hidden'>
      <header className='flex shrink-0 items-start justify-between gap-4 border-b border-border px-5 py-4 pr-12'>
        <div className='min-w-0'>
          <div className='flex items-center gap-2'>
            <BookText className='size-4 shrink-0 text-muted-foreground' aria-hidden='true' />
            {inDialog ? (
              <DialogTitle className='text-[15px] font-semibold'>{t('metadata.title')}</DialogTitle>
            ) : (
              <h2 className='text-[15px] font-semibold'>{t('metadata.title')}</h2>
            )}
          </div>
          {inDialog ? (
            <DialogDescription className='mt-0.5 text-[11px] leading-4 text-muted-foreground'>
              {t('metadata.description')}
            </DialogDescription>
          ) : (
            <p className='mt-0.5 text-[11px] leading-4 text-muted-foreground'>
              {t('metadata.description')}
            </p>
          )}
        </div>
        <Button
          type='button'
          size='sm'
          disabled={!hasTranslatableFields || translating || isLoading}
          aria-busy={translating}
          onClick={() => void handleTranslate()}
          className='shrink-0 gap-1.5'
        >
          {translating ? (
            <LoaderCircle className='size-3.5 animate-spin' aria-hidden='true' />
          ) : (
            <Languages className='size-3.5' aria-hidden='true' />
          )}
          {translating ? t('metadata.translating') : t('metadata.translate')}
        </Button>
      </header>

      <ScrollArea
        className='flex min-h-0 flex-1 flex-col overflow-hidden'
        viewportClassName='h-auto min-h-0 flex-1'
      >
        <div className='flex flex-col gap-5 px-5 py-4'>
          {isLoading ? (
            <p className='py-6 text-[11px] text-muted-foreground'>{t('common.loading')}</p>
          ) : (
            <>
              <Section title={t('metadata.sections.common')}>
                <ProseRow
                  label={t('metadata.fields.series')}
                  field={info.series}
                  registerFlush={registerFlush}
                  onCommit={(part, value) => updateProse('series', part, value)}
                />
                <ProseRow
                  label={t('metadata.fields.title')}
                  field={info.title}
                  registerFlush={registerFlush}
                  onCommit={(part, value) => updateProse('title', part, value)}
                />
                <ProseRow
                  label={t('metadata.fields.summary')}
                  field={info.summary}
                  multiline
                  registerFlush={registerFlush}
                  onCommit={(part, value) => updateProse('summary', part, value)}
                />
                <div className='grid grid-cols-2 gap-3'>
                  <TextRow
                    label={t('metadata.fields.number')}
                    value={info.number ?? ''}
                    registerFlush={registerFlush}
                    onCommit={(value) => updateText('number', value)}
                  />
                  <NumberField
                    label={t('metadata.fields.volume')}
                    value={info.volume ?? null}
                    integer
                    min={0}
                    onChange={(value) => updateInt('volume', value)}
                  />
                  <TextRow
                    label={t('metadata.fields.writer')}
                    value={info.writer ?? ''}
                    registerFlush={registerFlush}
                    onCommit={(value) => updateText('writer', value)}
                  />
                  <div className='flex flex-col gap-1'>
                    <TextRow
                      label={t('metadata.fields.languageIso')}
                      value={info.language_iso ?? ''}
                      registerFlush={registerFlush}
                      onCommit={(value) => updateText('language_iso', value)}
                    />
                    <p className='text-[10px] leading-3.5 text-muted-foreground'>
                      {t('metadata.languageIsoHint', { target: targetLanguage })}
                    </p>
                  </div>
                  <label className='grid gap-1 text-[10px] text-muted-foreground'>
                    {t('metadata.fields.manga')}
                    <Choice<Manga>
                      value={info.manga ?? 'Unknown'}
                      choices={mangaChoices}
                      label={t('metadata.fields.manga')}
                      onChange={(next) =>
                        commit({ ...latestRef.current, manga: next === 'Unknown' ? null : next })
                      }
                    />
                  </label>
                </div>
                {!hasTranslatableFields && (
                  <p className='text-[10px] text-muted-foreground'>
                    {t('metadata.noTranslatableFields')}
                  </p>
                )}
              </Section>

              <Collapsible
                open={moreOpen}
                onOpenChange={setMoreOpen}
                className='flex flex-col gap-3'
              >
                <CollapsibleTrigger
                  render={
                    <Button
                      type='button'
                      variant='outline'
                      size='sm'
                      className='w-full justify-between text-[11px]'
                    />
                  }
                >
                  <span>{t('metadata.moreFields')}</span>
                  <ChevronDown
                    className={cn(
                      'size-3.5 transition-transform duration-150',
                      moreOpen && 'rotate-180',
                    )}
                    aria-hidden='true'
                  />
                </CollapsibleTrigger>

                <CollapsibleContent className='flex flex-col gap-5 pt-1'>
                  <Section title={t('metadata.sections.seriesAndStory')}>
                    <ProseRow
                      label={t('metadata.fields.alternateSeries')}
                      field={info.alternate_series}
                      registerFlush={registerFlush}
                      onCommit={(part, value) => updateProse('alternate_series', part, value)}
                    />
                    <div className='grid grid-cols-3 gap-3'>
                      <TextRow
                        label={t('metadata.fields.alternateNumber')}
                        value={info.alternate_number ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('alternate_number', value)}
                      />
                      <NumberField
                        label={t('metadata.fields.alternateCount')}
                        value={info.alternate_count ?? null}
                        integer
                        min={0}
                        onChange={(value) => updateInt('alternate_count', value)}
                      />
                      <NumberField
                        label={t('metadata.fields.count')}
                        value={info.count ?? null}
                        integer
                        min={0}
                        onChange={(value) => updateInt('count', value)}
                      />
                    </div>
                    <ProseRow
                      label={t('metadata.fields.storyArc')}
                      field={info.story_arc}
                      registerFlush={registerFlush}
                      onCommit={(part, value) => updateProse('story_arc', part, value)}
                    />
                    <TextRow
                      label={t('metadata.fields.storyArcNumber')}
                      value={info.story_arc_number ?? ''}
                      registerFlush={registerFlush}
                      onCommit={(value) => updateText('story_arc_number', value)}
                    />
                    <ProseRow
                      label={t('metadata.fields.seriesGroup')}
                      field={info.series_group}
                      registerFlush={registerFlush}
                      onCommit={(part, value) => updateProse('series_group', part, value)}
                    />
                  </Section>

                  <Section title={t('metadata.sections.publishing')}>
                    <div className='grid grid-cols-3 gap-3'>
                      <NumberField
                        label={t('metadata.fields.year')}
                        value={info.year ?? null}
                        integer
                        min={0}
                        onChange={(value) => updateInt('year', value)}
                      />
                      <NumberField
                        label={t('metadata.fields.month')}
                        value={info.month ?? null}
                        integer
                        min={1}
                        max={12}
                        onChange={(value) => updateInt('month', value)}
                      />
                      <NumberField
                        label={t('metadata.fields.day')}
                        value={info.day ?? null}
                        integer
                        min={1}
                        max={31}
                        onChange={(value) => updateInt('day', value)}
                      />
                    </div>
                    <div className='grid grid-cols-2 gap-3'>
                      <TextRow
                        label={t('metadata.fields.publisher')}
                        value={info.publisher ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('publisher', value)}
                      />
                      <TextRow
                        label={t('metadata.fields.imprint')}
                        value={info.imprint ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('imprint', value)}
                      />
                      <TextRow
                        label={t('metadata.fields.format')}
                        value={info.format ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('format', value)}
                      />
                      <TextRow
                        label={t('metadata.fields.web')}
                        value={info.web ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('web', value)}
                      />
                      <TextRow
                        label={t('metadata.fields.gtin')}
                        value={info.gtin ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('gtin', value)}
                      />
                    </div>
                  </Section>

                  <Section title={t('metadata.sections.credits')}>
                    <div className='grid grid-cols-2 gap-3'>
                      <TextRow
                        label={t('metadata.fields.penciller')}
                        value={info.penciller ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('penciller', value)}
                      />
                      <TextRow
                        label={t('metadata.fields.inker')}
                        value={info.inker ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('inker', value)}
                      />
                      <TextRow
                        label={t('metadata.fields.colorist')}
                        value={info.colorist ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('colorist', value)}
                      />
                      <TextRow
                        label={t('metadata.fields.letterer')}
                        value={info.letterer ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('letterer', value)}
                      />
                      <TextRow
                        label={t('metadata.fields.coverArtist')}
                        value={info.cover_artist ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('cover_artist', value)}
                      />
                      <TextRow
                        label={t('metadata.fields.editor')}
                        value={info.editor ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('editor', value)}
                      />
                      <TextRow
                        label={t('metadata.fields.translator')}
                        value={info.translator ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('translator', value)}
                      />
                    </div>
                  </Section>

                  <Section title={t('metadata.sections.classification')}>
                    <ProseRow
                      label={t('metadata.fields.genre')}
                      field={info.genre}
                      registerFlush={registerFlush}
                      onCommit={(part, value) => updateProse('genre', part, value)}
                    />
                    <ProseRow
                      label={t('metadata.fields.tags')}
                      field={info.tags}
                      registerFlush={registerFlush}
                      onCommit={(part, value) => updateProse('tags', part, value)}
                    />
                    <div className='grid grid-cols-2 gap-3'>
                      <label className='grid gap-1 text-[10px] text-muted-foreground'>
                        {t('metadata.fields.ageRating')}
                        <Choice<AgeRating>
                          value={info.age_rating ?? 'Unknown'}
                          choices={ageRatingChoices}
                          label={t('metadata.fields.ageRating')}
                          onChange={(next) =>
                            commit({
                              ...latestRef.current,
                              age_rating: next === 'Unknown' ? null : next,
                            })
                          }
                        />
                      </label>
                      <label className='grid gap-1 text-[10px] text-muted-foreground'>
                        {t('metadata.fields.blackAndWhite')}
                        <Choice<YesNo>
                          value={info.black_and_white ?? 'Unknown'}
                          choices={yesNoChoices}
                          label={t('metadata.fields.blackAndWhite')}
                          onChange={(next) =>
                            commit({
                              ...latestRef.current,
                              black_and_white: next === 'Unknown' ? null : next,
                            })
                          }
                        />
                      </label>
                      <NumberField
                        label={t('metadata.fields.communityRating')}
                        value={info.community_rating ?? null}
                        min={0}
                        max={5}
                        step={0.5}
                        onChange={(value) => {
                          const normalized = value === null || Number.isNaN(value) ? null : value
                          if (latestRef.current.community_rating !== normalized) {
                            commit({ ...latestRef.current, community_rating: normalized })
                          }
                        }}
                      />
                      <TextRow
                        label={t('metadata.fields.scanInformation')}
                        value={info.scan_information ?? ''}
                        registerFlush={registerFlush}
                        onCommit={(value) => updateText('scan_information', value)}
                      />
                    </div>
                  </Section>

                  <Section title={t('metadata.sections.charactersAndWorld')}>
                    <ProseRow
                      label={t('metadata.fields.characters')}
                      field={info.characters}
                      registerFlush={registerFlush}
                      onCommit={(part, value) => updateProse('characters', part, value)}
                    />
                    <ProseRow
                      label={t('metadata.fields.teams')}
                      field={info.teams}
                      registerFlush={registerFlush}
                      onCommit={(part, value) => updateProse('teams', part, value)}
                    />
                    <ProseRow
                      label={t('metadata.fields.locations')}
                      field={info.locations}
                      registerFlush={registerFlush}
                      onCommit={(part, value) => updateProse('locations', part, value)}
                    />
                    <ProseRow
                      label={t('metadata.fields.mainCharacterOrTeam')}
                      field={info.main_character_or_team}
                      registerFlush={registerFlush}
                      onCommit={(part, value) => updateProse('main_character_or_team', part, value)}
                    />
                  </Section>

                  <Section title={t('metadata.sections.notesAndReview')}>
                    <ProseRow
                      label={t('metadata.fields.notes')}
                      field={info.notes}
                      multiline
                      registerFlush={registerFlush}
                      onCommit={(part, value) => updateProse('notes', part, value)}
                    />
                    <ProseRow
                      label={t('metadata.fields.review')}
                      field={info.review}
                      multiline
                      registerFlush={registerFlush}
                      onCommit={(part, value) => updateProse('review', part, value)}
                    />
                  </Section>

                  <Section title={t('metadata.sections.pages')}>
                    <div className='grid grid-cols-2 gap-3 text-[11px]'>
                      <div className='flex flex-col gap-0.5'>
                        <span className='text-[10px] text-muted-foreground'>
                          {t('metadata.fields.pageCount')}
                        </span>
                        <span className='font-medium tabular-nums'>{pageCount}</span>
                      </div>
                      <div className='flex flex-col gap-0.5'>
                        <span className='text-[10px] text-muted-foreground'>
                          {t('metadata.fields.pages')}
                        </span>
                        <span className='text-muted-foreground'>
                          {t('metadata.pagesAutoComputed', { count: pageCount })}
                        </span>
                      </div>
                    </div>
                  </Section>
                </CollapsibleContent>
              </Collapsible>
            </>
          )}
        </div>
      </ScrollArea>
    </div>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className='flex flex-col gap-3 rounded-xl border border-border/80 bg-[var(--surface-panel)] p-4'>
      <h3 className='text-[11px] font-semibold tracking-wide text-muted-foreground uppercase'>
        {title}
      </h3>
      <div className='flex flex-col gap-3'>{children}</div>
    </section>
  )
}

function ProseRow({
  label,
  field,
  multiline = false,
  registerFlush,
  onCommit,
}: {
  label: string
  field?: LocalizedField | null
  multiline?: boolean
  registerFlush: (flush: () => void) => () => void
  onCommit: (part: 'original' | 'translated', value: string) => void
}) {
  const { t } = useTranslation()
  const original = field?.original ?? ''
  const translated = field?.translated ?? ''

  return (
    <div className='flex flex-col gap-1.5'>
      <span className='text-[11px] font-medium'>{label}</span>
      <div className='grid grid-cols-2 gap-3'>
        <label className='grid gap-1 text-[10px] text-muted-foreground'>
          <span>{t('metadata.original')}</span>
          {multiline ? (
            <CommitTextarea
              aria-label={`${label} — ${t('metadata.original')}`}
              value={original}
              rows={3}
              className='text-[12px] text-foreground'
              onCommit={(value) => onCommit('original', value)}
            />
          ) : (
            <CommitInput
              aria-label={`${label} — ${t('metadata.original')}`}
              value={original}
              registerFlush={registerFlush}
              onCommit={(value) => onCommit('original', value)}
            />
          )}
        </label>
        <label className='grid gap-1 text-[10px] text-muted-foreground'>
          <span>{t('metadata.translated')}</span>
          {multiline ? (
            <CommitTextarea
              aria-label={`${label} — ${t('metadata.translated')}`}
              value={translated}
              rows={3}
              className='text-[12px] text-foreground'
              onCommit={(value) => onCommit('translated', value)}
            />
          ) : (
            <CommitInput
              aria-label={`${label} — ${t('metadata.translated')}`}
              value={translated}
              registerFlush={registerFlush}
              onCommit={(value) => onCommit('translated', value)}
            />
          )}
        </label>
      </div>
    </div>
  )
}

function TextRow({
  label,
  value,
  registerFlush,
  onCommit,
}: {
  label: string
  value: string
  registerFlush: (flush: () => void) => () => void
  onCommit: (value: string) => void
}) {
  return (
    <label className='grid gap-1 text-[10px] text-muted-foreground'>
      <span>{label}</span>
      <CommitInput value={value} registerFlush={registerFlush} onCommit={onCommit} />
    </label>
  )
}

type CommitInputProps = Omit<ComponentProps<typeof Input>, 'value' | 'onChange'> & {
  value: string
  delay?: number
  registerFlush: (flush: () => void) => () => void
  onCommit: (value: string) => void
}

function CommitInput({
  value,
  delay = 360,
  registerFlush,
  onCommit,
  className,
  ...props
}: CommitInputProps) {
  const [draft, setDraft] = useState(value)
  const composing = useRef(false)
  const commit = useDebouncedCallback(
    (next: string) => {
      if (next !== value) onCommit(next)
    },
    delay,
    { flushOnExit: true },
  )

  useEffect(() => {
    if (!composing.current && !commit.isPending()) setDraft(value)
  }, [commit, value])

  useEffect(() => {
    return registerFlush(() => {
      commit(draft)
      commit.flush()
    })
  }, [commit, draft, registerFlush])

  return (
    <Input
      {...props}
      value={draft}
      className={cn('h-8 text-[12px] text-foreground', className)}
      onChange={(event) => {
        const next = event.currentTarget.value
        setDraft(next)
        if (!composing.current) commit(next)
      }}
      onCompositionStart={() => {
        composing.current = true
      }}
      onCompositionEnd={(event) => {
        composing.current = false
        const next = event.currentTarget.value
        setDraft(next)
        commit(next)
      }}
      onBlur={() => {
        commit(draft)
        commit.flush()
      }}
    />
  )
}
