'use client'

import { ArrowLeftRight, BookOpen } from 'lucide-react'
import { useEffect, useRef, useState, type PointerEvent, type WheelEvent } from 'react'
import { useHotkeys } from 'react-hotkeys-hook'
import { useTranslation } from 'react-i18next'

import { useCanvasViewport } from '@/components/editor/CanvasWorkspace'
import { useGallery } from '@/lib/gallery'
import { send, useProject } from '@/lib/queries'
import { commands, type FacingPage, type PageId, type PageSide } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { cn } from '@koharu/ui/lib/utils'

/** How near the bottom, in pixels, the pointer brings up the page indicator. */
const indicatorReach = 80

/** The way pages turn when the chapter doesn't say, a viewer's own choice. */
const directionKey = 'koharu.reader.direction'
type Turning = 'rtl' | 'ltr'

/** A spread: the indexes of the pages it shows side by side, in reading order. */
type Spread = number[]

/** The chapter's `count` pages as a book's spreads: the first page alone when it stands alone, as a
 * cover does, then two at a time. */
function spreads(count: number, firstAlone: boolean): Spread[] {
  const paired: Spread[] = firstAlone && count > 0 ? [[0]] : []
  for (let index = paired.length; index < count; index += 2) {
    paired.push(index + 1 < count ? [index, index + 1] : [index])
  }
  return paired
}

/** Whether a chapter's first page stands alone in a spread: the side it takes is the one a spread
 * begins on its other side, so it has no page beside it. A chapter that doesn't say has its
 * cover alone. */
function standsAlone(side: PageSide | null | undefined, direction: Turning): boolean {
  if (!side) return true
  return direction === 'rtl' ? side === 'left' : side === 'right'
}

function storedDirection(): Turning {
  try {
    return window.localStorage.getItem(directionKey) === 'ltr' ? 'ltr' : 'rtl'
  } catch {
    return 'rtl'
  }
}

/** The reader: the open project's pages fit to the window, two at a time as a book's spread while
 * the window is wider than tall, with no editing. Keys, clicks on either half, and the wheel turn
 * pages, in the chapter's direction. */
export function Reader() {
  const { t } = useTranslation()
  const surface = useRef<HTMLDivElement>(null)
  useCanvasViewport(surface)
  const project = useProject().data
  const pages = project?.pages ?? []
  const index = pages.findIndex((page) => page.id === project?.page)
  const reading = useGallery((state) => (project ? state.readings[project.name] : undefined))
  const [preferred, setPreferred] = useState<Turning>(storedDirection)
  // A chapter's own direction wins; a webtoon's strip turns forward downward and to the right.
  const direction: Turning =
    reading?.direction === 'rtl' ? 'rtl' : reading?.direction ? 'ltr' : preferred
  // What a click on the left half and on the right half turns by.
  const [left, right] = direction === 'rtl' ? [1, -1] : [-1, 1]

  // Two pages show side by side while the reader is wider than tall, except a webtoon's strip.
  const [wide, setWide] = useState(false)
  useEffect(() => {
    const element = surface.current
    if (!element) return
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setWide(entry.contentRect.width > entry.contentRect.height)
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  const spreading = wide && reading?.direction !== 'vertical'
  const [shifted, setShifted] = useState(false)
  const firstAlone = standsAlone(reading?.firstPage, direction) !== shifted
  const spreadsShown = spreading
    ? spreads(pages.length, firstAlone)
    : pages.map((_, page) => [page])
  const at = spreadsShown.findIndex((spread) => spread.includes(index))

  // A spread shows its first page, with the next facing it: on its left as Japanese manga reads.
  const facing = (spread: Spread): FacingPage | null => {
    const page = spread[1] === undefined ? undefined : pages[spread[1]]
    return page ? { page: page.id, side: direction === 'rtl' ? 'left' : 'right' } : null
  }
  const sent = useRef<{ page: PageId; facing: string | null } | null>(null)
  const show = (spread: Spread | undefined) => {
    const page = spread && pages[spread[0]]
    if (!page) return
    const beside = facing(spread)
    sent.current = { page: page.id, facing: beside && `${beside.page}:${beside.side}` }
    send(commands.show, page.id, beside)
  }
  const turn = (by: number) => show(spreadsShown[at + by])
  // The spread holding the page shown, which another page shown from elsewhere moves to; the
  // canvas's facing page goes with any other show.
  const spread = spreadsShown[at]
  const wanted = spread && facing(spread)
  const wantedKey = wanted ? `${wanted.page}:${wanted.side}` : null
  const first = spread && pages[spread[0]]?.id
  useEffect(() => {
    if (!project?.page || !first) return
    const shownFacing = sent.current?.page === project.page ? sent.current.facing : null
    if (first === project.page && wantedKey === shownFacing) return
    show(spread)
  })
  // Leaving, the canvas shows the page alone again, as the editor does.
  useEffect(
    () => () => {
      const shown = sent.current
      if (shown?.facing) send(commands.show, shown.page, null)
    },
    [],
  )
  // The page shown fits the window, as a reader shows each page whole.
  useEffect(() => {
    if (project?.page) send(commands.fit)
  }, [project?.page])

  const forwardKey = direction === 'rtl' ? 'arrowleft' : 'arrowright'
  const backwardKey = direction === 'rtl' ? 'arrowright' : 'arrowleft'
  useHotkeys([forwardKey, 'arrowdown', 'pagedown', 'space'], () => turn(1), {
    preventDefault: true,
  })
  useHotkeys([backwardKey, 'arrowup', 'pageup'], () => turn(-1), { preventDefault: true })
  useHotkeys('home', () => show(spreadsShown[0]), { preventDefault: true })
  useHotkeys('end', () => show(spreadsShown.at(-1)), { preventDefault: true })

  // One turn for each wheel gesture, not each of its events.
  const wheeling = useRef(0)
  const wheel = (event: WheelEvent) => {
    const now = performance.now()
    if (now - wheeling.current < 250 || Math.abs(event.deltaY) < 4) return
    wheeling.current = now
    turn(event.deltaY > 0 ? 1 : -1)
  }

  const flip = () => {
    const next: Turning = preferred === 'rtl' ? 'ltr' : 'rtl'
    setPreferred(next)
    try {
      window.localStorage.setItem(directionKey, next)
    } catch {
      // A viewer without storage keeps it for now only.
    }
  }

  // The page indicator stays out of the page's way until the pointer comes near the bottom.
  const [indicating, setIndicating] = useState(false)
  const point = (event: PointerEvent) => {
    const bottom = event.currentTarget.getBoundingClientRect().bottom
    setIndicating(bottom - event.clientY < indicatorReach)
  }

  return (
    <div
      className='relative h-full min-h-0 bg-transparent'
      onPointerMove={point}
      onPointerLeave={() => setIndicating(false)}
    >
      <div ref={surface} className='absolute inset-0' onWheel={wheel}>
        {/* The halves turn pages. Painted all but invisibly, so the pointer is the interface's,
            not the canvas's, and reading never edits. */}
        <button
          type='button'
          aria-label={left > 0 ? t('reader.next') : t('reader.previous')}
          className='absolute inset-y-0 left-0 w-1/2 cursor-default bg-black/[0.004] outline-none'
          onClick={() => turn(left)}
        />
        <button
          type='button'
          aria-label={right > 0 ? t('reader.next') : t('reader.previous')}
          className='absolute inset-y-0 right-0 w-1/2 cursor-default bg-black/[0.004] outline-none'
          onClick={() => turn(right)}
        />
      </div>
      <div
        className={cn(
          'pointer-events-none absolute inset-x-0 bottom-3 flex justify-center transition-opacity duration-150',
          indicating ? 'opacity-100' : 'opacity-0 focus-within:opacity-100',
        )}
      >
        <div
          className={cn(
            'flex items-center gap-2 rounded-full border border-border/60 bg-[var(--surface-panel)]/90 px-3 py-1 text-[10px] text-muted-foreground shadow-sm backdrop-blur',
            indicating && 'pointer-events-auto',
          )}
        >
          <span className='tabular-nums'>
            {pages.length === 0
              ? t('reader.noPages')
              : `${spread && spread.length > 1 ? `${spread[0] + 1}–${spread[1] + 1}` : index + 1} / ${pages.length}`}
          </span>
          {spreading && (
            <Button
              size='sm'
              variant='ghost'
              className='h-6 gap-1 px-2 text-[10px] aria-pressed:bg-foreground/[0.07] aria-pressed:text-foreground'
              aria-pressed={firstAlone}
              onClick={() => setShifted(!shifted)}
            >
              <BookOpen className='size-3' />
              {t('reader.coverAlone')}
            </Button>
          )}
          {!reading?.direction && (
            <Button size='sm' variant='ghost' className='h-6 gap-1 px-2 text-[10px]' onClick={flip}>
              <ArrowLeftRight className='size-3' />
              {preferred === 'rtl' ? t('reader.rtl') : t('reader.ltr')}
            </Button>
          )}
        </div>
      </div>
    </div>
  )
}
