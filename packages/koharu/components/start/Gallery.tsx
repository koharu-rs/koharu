'use client'

import { useInfiniteQuery, useMutation, useQueries, useQuery } from '@tanstack/react-query'
import { ArrowLeft, ChevronRight, Library, LoaderCircle, Lock, Search, X } from 'lucide-react'
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useGallery } from '@/lib/gallery'
import { pageKey, projectKey, queryClient, refresh } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import {
  commands,
  type Catalog,
  type Listing,
  type Opened,
  type SeriesLink,
  type Site,
} from '@koharu/ipc/protocol'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
} from '@koharu/ui/components/alert-dialog'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { cn } from '@koharu/ui/lib/utils'

const sitesKey = ['sites'] as const
/** A site's answers are kept a few minutes, so going back or searching again doesn't ask it
 * again. */
const kept = { staleTime: 5 * 60_000, gcTime: 15 * 60_000 } as const

/** A list a site row shows: the site's search, or one of its catalogs. */
interface Shown {
  site: Site
  catalog: Catalog | null
}

/** The query for the page of `shown` that `next` starts. A site that fails answers nothing, which
 * leaves its row out instead of reporting it. */
function listing(shown: Shown, query: string, next: string | null): Promise<Listing | null> {
  const asked = shown.catalog
    ? commands.browseSite(shown.site.id, shown.catalog, next)
    : commands.searchSite(shown.site.id, query, next)
  return asked.catch(() => null)
}

function firstPageKey(shown: Shown, query: string) {
  return ['site', shown.site.id, shown.catalog ?? 'search', shown.catalog ? '' : query] as const
}

/** Catalog pages can overlap. Keep each series where the site first listed it. */
function listedSeries({ pages }: { pages: Listing[] }): SeriesLink[] {
  const series = new Map<string, SeriesLink>()
  for (const page of pages) {
    for (const item of page.series) {
      if (!series.has(item.url)) series.set(item.url, item)
    }
  }
  return Array.from(series.values())
}

/** The gallery: the sites Koharu reads, searched together or browsed by catalog, and the series
 * the user opened. A chapter opens as a project, read in the reader. */
export function Gallery() {
  const { t } = useTranslation()
  const [draft, setDraft] = useState('')
  const [query, setQuery] = useState('')
  const [expanded, setExpanded] = useState<Shown | null>(null)
  const [opened, setOpened] = useState<Opened | null>(null)
  const [removing, setRemoving] = useState<SeriesLink | null>(null)
  const setReading = useKoharuStore((state) => state.setReading)
  const sites = useQuery({ queryKey: sitesKey, queryFn: () => commands.listSites() }).data ?? []
  const shelf = useGallery((state) => state.shelf)
  const keep = useGallery((state) => state.keep)
  const forget = useGallery((state) => state.forget)
  const remember = useGallery((state) => state.remember)

  // An opened series goes first on the shelf.
  const show = (opened: Opened) => {
    setOpened(opened)
    keep({ title: opened.series.title, url: opened.url, cover: opened.series.cover })
  }
  const openSeries = useMutation({
    mutationFn: (url: string) => commands.openSeries(url),
    onSuccess: show,
  })
  // A chapter opens to edit; the user runs the pipeline on it when they choose.
  const read = useMutation({
    mutationFn: (url: string) => commands.readChapter(url),
    onMutate: () => setReading(false),
    onSuccess: (reading) => {
      remember(reading)
      return refresh(projectKey, pageKey)
    },
  })
  // A pasted address opens its series, or reads its chapter when it's no series'.
  const openAddress = useMutation({
    mutationFn: (url: string) => commands.openSeries(url).catch(() => null),
    onSuccess: (series, url) => (series ? show(series) : read.mutate(url)),
  })
  const busy = openSeries.isPending || read.isPending || openAddress.isPending

  const submit = (event: FormEvent) => {
    event.preventDefault()
    const text = draft.trim()
    if (!text || busy) return
    setOpened(null)
    setExpanded(null)
    if (/^https?:\/\//i.test(text) && URL.canParse(text)) openAddress.mutate(text)
    else setQuery(text)
  }

  const onShelf = new Set(shelf.map((kept) => kept.url))
  const open = (url: string) => openSeries.mutate(url)

  return (
    <div className='mt-7 flex min-h-[390px] grow basis-0 flex-col overflow-hidden rounded-2xl border border-border/80 bg-[var(--surface-panel)]'>
      <form
        className='flex shrink-0 items-center gap-2 border-b border-border/70 px-5 py-3'
        onSubmit={submit}
      >
        <div className='relative min-w-0 flex-1'>
          <Search className='pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground' />
          <Input
            value={draft}
            onChange={(event) => {
              setDraft(event.target.value)
              if (!event.target.value.trim()) setQuery('')
            }}
            placeholder={t('gallery.searchPlaceholder')}
            aria-label={t('gallery.search')}
            autoComplete='off'
            spellCheck={false}
            className='h-8 bg-background pl-8 text-[11px]'
          />
        </div>
        {(openAddress.isPending || (read.isPending && !opened)) && (
          <LoaderCircle className='size-3.5 animate-spin' />
        )}
      </form>

      {opened ? (
        <SeriesView
          opened={opened}
          busy={busy}
          reading={read.isPending ? read.variables : null}
          onBack={() => setOpened(null)}
          onRead={(url) => read.mutate(url)}
        />
      ) : expanded ? (
        <FullList
          key={firstPageKey(expanded, query).join('/')}
          shown={expanded}
          query={query}
          onShelf={onShelf}
          busy={busy}
          onBack={() => setExpanded(null)}
          onOpen={open}
        />
      ) : (
        <ScrollArea className='min-h-0 flex-1' viewportClassName='pb-3'>
          {query ? (
            <Rows
              key={query}
              shown={sites.map((site) => ({ site, catalog: null }))}
              query={query}
              onShelf={onShelf}
              busy={busy}
              onOpen={open}
              onSeeAll={setExpanded}
            />
          ) : (
            <>
              {shelf.length > 0 && (
                <Row title={t('gallery.yourSeries')}>
                  {shelf.map((kept) => (
                    <Card
                      key={kept.url}
                      series={kept}
                      busy={busy}
                      onOpen={() => open(kept.url)}
                      onRemove={() => setRemoving(kept)}
                    />
                  ))}
                </Row>
              )}
              <Rows
                shown={sites.flatMap((site) => site.catalogs.map((catalog) => ({ site, catalog })))}
                query=''
                onShelf={onShelf}
                busy={busy}
                onOpen={open}
                onSeeAll={setExpanded}
                empty={shelf.length === 0}
              />
            </>
          )}
        </ScrollArea>
      )}

      <AlertDialog open={removing !== null} onOpenChange={(open) => !open && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogMedia className='bg-destructive/10 text-destructive'>
              <X className='size-5' />
            </AlertDialogMedia>
            <AlertDialogTitle>{t('gallery.removeTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('gallery.removeDescription', { title: removing?.title })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('common.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              variant='destructive'
              onClick={() => {
                if (removing) forget(removing.url)
                setRemoving(null)
              }}
            >
              {t('gallery.removeAction')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

/** A row for each list that answered with series; one that failed or found nothing is left
 * out. */
function Rows({
  shown,
  query,
  onShelf,
  busy,
  onOpen,
  onSeeAll,
  empty = true,
}: {
  shown: Shown[]
  query: string
  onShelf: Set<string>
  busy: boolean
  onOpen: (url: string) => void
  onSeeAll: (shown: Shown) => void
  /** Whether nothing else is shown, so finding nothing says so. */
  empty?: boolean
}) {
  const { t } = useTranslation()
  const pages = useQueries({
    queries: shown.map((each) => ({
      queryKey: firstPageKey(each, query),
      queryFn: () => listing(each, query, null),
      ...kept,
    })),
  })
  const searching = pages.some((page) => page.isPending)
  const rows = shown.flatMap((each, index) => {
    const page = pages[index]?.data
    return page && page.series.length > 0 ? [{ each, page }] : []
  })
  return (
    <>
      {rows.map(({ each, page }) => (
        <Row
          key={`${each.site.id}/${each.catalog ?? 'search'}`}
          title={each.site.name}
          detail={each.catalog ? t(`gallery.${each.catalog}`) : undefined}
          onSeeAll={page.next ? () => onSeeAll(each) : undefined}
        >
          {page.series.map((series) => (
            <Card
              key={series.url}
              series={series}
              onShelf={onShelf.has(series.url)}
              busy={busy}
              onOpen={() => onOpen(series.url)}
            />
          ))}
        </Row>
      ))}
      {searching
        ? query && (
            <p className='flex items-center gap-1.5 px-5 py-3 text-[10px] text-muted-foreground'>
              <LoaderCircle className='size-3 animate-spin' /> {t('gallery.searching')}
            </p>
          )
        : rows.length === 0 &&
          empty && (
            <Empty
              title={query ? t('gallery.noResults', { query }) : t('gallery.noSeries')}
              description={query ? undefined : t('gallery.noSeriesHint')}
            />
          )}
    </>
  )
}

/** A titled row of series cards that scrolls sideways. */
function Row({
  title,
  detail,
  onSeeAll,
  children,
}: {
  title: string
  detail?: string
  onSeeAll?: () => void
  children: React.ReactNode
}) {
  const { t } = useTranslation()
  return (
    <section className='pt-4'>
      <header className='flex h-7 items-center gap-2 px-5'>
        <h3 className='truncate text-[12px] font-semibold'>{title}</h3>
        {detail && <span className='shrink-0 text-[10px] text-muted-foreground'>{detail}</span>}
        {onSeeAll && (
          <Button
            size='sm'
            variant='ghost'
            className='ml-auto h-7 shrink-0 gap-0.5 px-2 text-[10px] text-muted-foreground hover:text-foreground'
            onClick={onSeeAll}
          >
            {t('gallery.seeAll')}
            <ChevronRight className='size-3' />
          </Button>
        )}
      </header>
      <ScrollArea orientation='horizontal' viewportClassName='px-5 pt-1.5 pb-3'>
        <ul aria-label={detail ? `${title} ${detail}` : title} className='flex w-max gap-3'>
          {children}
        </ul>
      </ScrollArea>
    </section>
  )
}

/** One page after another of a site's search or catalog, loaded as the end comes into view. */
function FullList({
  shown,
  query,
  onShelf,
  busy,
  onBack,
  onOpen,
}: {
  shown: Shown
  query: string
  onShelf: Set<string>
  busy: boolean
  onBack: () => void
  onOpen: (url: string) => void
}) {
  const { t } = useTranslation()
  const first = queryClient.getQueryData<Listing | null>(firstPageKey(shown, query))
  const pages = useInfiniteQuery({
    queryKey: ['site-all', ...firstPageKey(shown, query)],
    // A page that fails ends the list.
    queryFn: async ({ pageParam }) =>
      (await listing(shown, query, pageParam)) ?? { series: [], next: null },
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.next,
    initialData: first ? { pages: [first], pageParams: [null] } : undefined,
    select: listedSeries,
    ...kept,
  })
  const end = useRef<HTMLDivElement>(null)
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = pages
  useEffect(() => {
    const target = end.current
    if (!target || !hasNextPage || isFetchingNextPage) return
    const observer = new IntersectionObserver(([entry]) => {
      if (entry?.isIntersecting) void fetchNextPage()
    })
    observer.observe(target)
    return () => observer.disconnect()
  }, [hasNextPage, isFetchingNextPage, fetchNextPage])
  const series = pages.data ?? []
  return (
    <>
      <header className='flex shrink-0 items-center gap-2 px-5 pt-4 pb-2'>
        <Button
          size='icon-sm'
          variant='ghost'
          className='size-7 shrink-0 text-muted-foreground hover:text-foreground'
          aria-label={t('gallery.back')}
          onClick={onBack}
        >
          <ArrowLeft className='size-3.5' />
        </Button>
        <h2 className='truncate text-[13px] font-semibold'>{shown.site.name}</h2>
        <span className='truncate text-[10px] text-muted-foreground'>
          {shown.catalog ? t(`gallery.${shown.catalog}`) : query}
        </span>
      </header>
      <ScrollArea className='min-h-0 flex-1' viewportClassName='px-5 pb-4'>
        <ul
          aria-label={shown.site.name}
          className='grid grid-cols-[repeat(auto-fill,minmax(104px,1fr))] gap-x-3 gap-y-4'
        >
          {series.map((each) => (
            <Card
              key={each.url}
              series={each}
              onShelf={onShelf.has(each.url)}
              busy={busy}
              onOpen={() => onOpen(each.url)}
            />
          ))}
        </ul>
        <div ref={end} className='flex h-10 items-center justify-center'>
          {pages.isFetching && (
            <LoaderCircle className='size-3.5 animate-spin text-muted-foreground' />
          )}
        </div>
      </ScrollArea>
    </>
  )
}

/** A series' cover and title; opening it shows its chapters. */
function Card({
  series,
  onShelf = false,
  busy,
  onOpen,
  onRemove,
}: {
  series: SeriesLink
  onShelf?: boolean
  busy: boolean
  onOpen: () => void
  onRemove?: () => void
}) {
  const { t } = useTranslation()
  return (
    <li className='group relative w-[104px] min-w-0 shrink-0 [ul.grid_&]:w-auto'>
      <button
        type='button'
        className='block w-full rounded-md text-left outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60'
        disabled={busy}
        onClick={onOpen}
      >
        <Cover series={series.url} url={series.cover} className='aspect-[3/4] w-full rounded-md' />
        <span className='mt-1.5 line-clamp-2 text-[10px] leading-[13px]'>{series.title}</span>
      </button>
      {onShelf && (
        <span className='pointer-events-none absolute top-1 left-1 rounded bg-background/90 px-1 py-px text-[9px] text-foreground'>
          {t('gallery.onShelf')}
        </span>
      )}
      {onRemove && (
        <Button
          size='icon-sm'
          variant='secondary'
          className='absolute top-1 right-1 size-6 opacity-0 group-hover:opacity-100 focus-visible:opacity-100'
          aria-label={t('gallery.removeLabel', { title: series.title })}
          onClick={onRemove}
        >
          <X className='size-3' />
        </Button>
      )}
    </li>
  )
}

/** A cover, read through its site's extension once it comes into view, as sites refuse it to
 * the main window's page and a row has many. */
function Cover({
  series,
  url,
  className = 'h-12 w-9 rounded-md',
}: {
  series: string
  url: string | null | undefined
  className?: string
}) {
  const element = useRef<HTMLSpanElement>(null)
  const [seen, setSeen] = useState(false)
  useEffect(() => {
    const target = element.current
    if (!target || seen) return
    const observer = new IntersectionObserver(([entry]) => {
      if (entry?.isIntersecting) setSeen(true)
    })
    observer.observe(target)
    return () => observer.disconnect()
  }, [seen])
  const bytes = useQuery({
    queryKey: ['cover', series, url],
    // A cover that doesn't come is left blank, not reported.
    queryFn: () => (url ? commands.getCover(series, url).catch(() => null) : null),
    enabled: seen && Boolean(url),
  }).data
  const [source, setSource] = useState<string | null>(null)
  useEffect(() => {
    if (!bytes) return
    const objectUrl = URL.createObjectURL(new Blob([bytes]))
    setSource(objectUrl)
    return () => URL.revokeObjectURL(objectUrl)
  }, [bytes])
  return (
    <span
      ref={element}
      className={cn('grid shrink-0 place-items-center overflow-hidden bg-muted', className)}
    >
      {source ? (
        // eslint-disable-next-line @next/next/no-img-element
        <img src={source} alt='' className='size-full object-cover' draggable={false} />
      ) : (
        <Library className='size-3.5 text-muted-foreground' />
      )}
    </span>
  )
}

function Empty({ title, description }: { title: string; description?: string }) {
  return (
    <div className='grid place-items-center px-6 py-16 text-center' role='status'>
      <div>
        <span className='mx-auto grid size-11 place-items-center rounded-xl bg-muted text-muted-foreground'>
          <Library className='size-[18px]' />
        </span>
        <p className='mt-3 text-[12px] font-medium'>{title}</p>
        {description && (
          <p className='mx-auto mt-1 max-w-[46ch] text-[10px] leading-4 text-muted-foreground'>
            {description}
          </p>
        )}
      </div>
    </div>
  )
}

/** A series and its chapters; a chapter opens in the reader. */
function SeriesView({
  opened,
  busy,
  reading,
  onBack,
  onRead,
}: {
  opened: Opened
  busy: boolean
  /** The chapter opening, whose row says so while the others wait. */
  reading: string | null
  onBack: () => void
  onRead: (url: string) => void
}) {
  const { t } = useTranslation()
  const { series } = opened
  return (
    <>
      <header className='flex shrink-0 items-center gap-3 px-5 pt-4 pb-2'>
        <Button
          size='icon-sm'
          variant='ghost'
          className='size-7 shrink-0 text-muted-foreground hover:text-foreground'
          aria-label={t('gallery.back')}
          onClick={onBack}
        >
          <ArrowLeft className='size-3.5' />
        </Button>
        <Cover series={opened.url} url={series.cover} />
        <div className='min-w-0 flex-1'>
          <h2 className='truncate text-[13px] font-semibold'>{series.title}</h2>
          <p className='text-[10px] text-muted-foreground'>
            {t('gallery.chapters', { count: series.chapters.length })}
          </p>
        </div>
      </header>
      <ScrollArea className='min-h-0 flex-1' viewportClassName='px-2 pb-2'>
        <ul aria-label={series.title} className='grid gap-0.5'>
          {series.chapters.map((chapter, index) => (
            <li key={`${chapter.url}-${index}`}>
              <button
                type='button'
                className={cn(
                  'flex w-full min-w-0 items-center gap-2 rounded-lg px-3 py-2 text-left hover:bg-foreground/[0.045] disabled:hover:bg-transparent',
                  chapter.url === reading ? 'bg-foreground/[0.045]' : 'disabled:opacity-50',
                )}
                disabled={busy || chapter.locked}
                aria-busy={chapter.url === reading}
                onClick={() => onRead(chapter.url)}
              >
                <span className='min-w-0 flex-1 truncate text-[11px]'>{chapter.title}</span>
                {chapter.url === reading && (
                  <LoaderCircle className='size-3.5 shrink-0 animate-spin text-muted-foreground' />
                )}
                {chapter.locked && (
                  <span className='flex shrink-0 items-center gap-1 text-[9px] text-muted-foreground'>
                    <Lock className='size-3' /> {t('gallery.locked')}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      </ScrollArea>
    </>
  )
}
