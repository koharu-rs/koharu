'use client'

import { useMutation } from '@tanstack/react-query'
import { defaultRangeExtractor, observeElementRect, useVirtualizer } from '@tanstack/react-virtual'
import { Languages, MoreHorizontal } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { CommitTextarea } from '@/components/controls/CommitTextarea'
import {
  findLayer,
  isBlockLayer,
  isLockedLayer,
  isTextLayer,
  plainText,
  type BlockLayer,
  type TextLayer,
} from '@/lib/document'
import { send, usePageStory, useProject } from '@/lib/queries'
import { deleteLayers, setSourceText, setText } from '@/lib/scene'
import { useKoharuStore } from '@/lib/store'
import { commands, type LayerId, type PageId } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@koharu/ui/components/dropdown-menu'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

/** Every page's texts in page order, each page's in the scene's reading order, as Clip Studio
 * Paint EX's Story Editor shows a work's text: a block's source and translation, and a text
 * layer's text, the hidden ones left out, each numbered as the canvas numbers it while Story shows.
 * A page's texts load as the list reaches them. Editing a field edits its layer; focusing it
 * selects the layer, showing its page first when the canvas shows another, as the page rail does.
 * Scrolling only browses: the list scrolls to the page the canvas shows when that changes, unless
 * the change came from the list itself. */
export function StoryPanel() {
  const { t } = useTranslation()
  const project = useProject().data
  const pages = project?.pages ?? []
  const shown = project?.page ?? null
  const list = useRef<HTMLDivElement>(null)
  // The page holding the focused field, kept mounted while it scrolls away, so an edit in
  // progress, or a composition, isn't lost with its field.
  const [focused, setFocused] = useState<number | null>(null)
  const virtualizer = useVirtualizer({
    count: pages.length,
    getScrollElement: () => list.current,
    getItemKey: (index) => pages[index]?.id ?? index,
    estimateSize: () => 160,
    overscan: 2,
    rangeExtractor: (range) => {
      const indexes = defaultRangeExtractor(range)
      return focused === null || indexes.includes(focused)
        ? indexes
        : [...indexes, focused].sort((a, b) => a - b)
    },
    initialRect: { width: 280, height: 600 },
    observeElementRect: (instance, callback) =>
      observeElementRect(instance, (rect) =>
        callback({ width: rect.width || 280, height: rect.height || 600 }),
      ),
  })

  const shownIndex = pages.findIndex((page) => page.id === shown)
  useEffect(() => {
    if (shownIndex < 0 || list.current?.contains(document.activeElement)) return
    virtualizer.scrollToIndex(shownIndex, { align: 'start' })
  }, [shownIndex, virtualizer])

  if (pages.length === 0) {
    return (
      <p className='px-4 py-8 text-center text-[10px] leading-4 text-muted-foreground'>
        {t('inspector.selectPage')}
      </p>
    )
  }
  return (
    <ScrollArea className='min-h-0 flex-1' viewportClassName='p-1' viewportRef={list}>
      <div className='relative w-full' style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((item) => {
          const page = pages[item.index]
          if (!page) return null
          return (
            <div
              key={item.key}
              ref={virtualizer.measureElement}
              data-index={item.index}
              className='absolute top-0 left-0 w-full'
              style={{ transform: `translateY(${item.start}px)` }}
              onFocus={() => setFocused(item.index)}
              onBlur={(event) => {
                if (!event.currentTarget.contains(event.relatedTarget)) setFocused(null)
              }}
            >
              <StoryPage page={page.id} number={item.index + 1} shown={page.id === shown} />
            </div>
          )
        })}
      </div>
    </ScrollArea>
  )
}

/** Shows a page on the canvas as clicking it in the page rail does: it alone selected among the
 * pages, and no layer. */
function useShowPage() {
  return useMutation({
    mutationFn: (page: PageId) => {
      const { selectPages, selectLayers } = useKoharuStore.getState()
      selectPages([page])
      selectLayers([])
      return commands.show(page, null)
    },
  })
}

/** A page's heading, which shows the page, and its texts, read when the list first reaches it. */
function StoryPage({ page, number, shown }: { page: PageId; number: number; shown: boolean }) {
  const { t } = useTranslation()
  const data = usePageStory(page).data
  const show = useShowPage()
  // The scene's order, each text found in the page as it shows; one the page doesn't have yet,
  // between the two reads of a change, waits for the next.
  const texts = useMemo(
    () =>
      data
        ? data.story.flatMap((id) => {
            const placed = findLayer(data.page.layers, id)
            return placed && (isBlockLayer(placed.layer) || isTextLayer(placed.layer))
              ? [{ layer: placed.layer, locked: isLockedLayer(placed) }]
              : []
          })
        : [],
    [data],
  )

  return (
    <section className='grid gap-1 pb-3'>
      <h3 data-shown={shown} className='pt-1'>
        <Button
          type='button'
          variant='ghost'
          size='xs'
          className='h-5 px-1 text-[10px] font-semibold text-muted-foreground hover:bg-primary/10 hover:text-foreground in-data-[shown=true]:text-primary'
          onClick={() => {
            if (!shown) show.mutate(page)
          }}
        >
          {t('navigator.pageNumber', { number })}
        </Button>
      </h3>
      {texts.map(({ layer, locked }, index) => (
        <StoryText
          key={layer.id}
          page={page}
          shown={shown}
          number={index + 1}
          layer={layer}
          locked={locked}
        />
      ))}
      {data && texts.length === 0 && (
        <p className='px-1 text-[10px] leading-4 text-muted-foreground'>{t('story.noText')}</p>
      )}
    </section>
  )
}

/** A text's place in reading order, as a badge: filled for dialogue, outlined for a sound
 * effect, as the canvas draws it while Story shows. */
function ReadingNumber({ number, sfx }: { number: number; sfx: boolean }) {
  return (
    <span
      className={cn(
        'inline-grid h-4 min-w-4 place-items-center rounded-full border border-primary px-1 text-[9px] leading-none font-semibold tabular-nums',
        sfx ? 'bg-background text-primary' : 'bg-primary text-primary-foreground',
      )}
    >
      {number}
    </span>
  )
}

/** A text's number and fields, highlighted while its layer, or one of a block's parts, is
 * selected, and scrolled into view when the canvas or Layers selects it. Under its number, while
 * the pointer is on it or it's selected, a block shows a button that translates it alone, and
 * every text a menu that deletes it. */
function StoryText({
  page,
  shown,
  number,
  layer,
  locked,
}: {
  page: PageId
  /** Its page is the one the canvas shows. */
  shown: boolean
  number: number
  layer: BlockLayer | TextLayer
  locked: boolean
}) {
  const { t } = useTranslation()
  const row = useRef<HTMLDivElement>(null)
  const ids = isBlockLayer(layer)
    ? [layer.id, layer.cleanup?.id, layer.mask?.id].filter((id): id is LayerId => id !== undefined)
    : [layer.id]
  const selected = useKoharuStore((state) =>
    state.selectedLayers.some((selected) => ids.includes(selected)),
  )
  const setStoryHover = useKoharuStore((state) => state.setStoryHover)
  const show = useShowPage()

  useEffect(() => {
    const element = row.current
    if (selected && element && !element.contains(document.activeElement)) {
      element.scrollIntoView({ block: 'nearest' })
    }
  }, [selected])

  const focus = () => {
    const { selectedLayers, selectLayers } = useKoharuStore.getState()
    if (shown) {
      if (!selectedLayers.includes(layer.id)) selectLayers([layer.id])
      return
    }
    // Then the text, once the canvas shows its page.
    show.mutate(page, { onSuccess: () => selectLayers([layer.id]) })
  }

  // As Layers deletes a layer: Undo brings it back, so nothing asks first.
  const removal = useMutation({
    mutationFn: () => deleteLayers([layer.id]),
    onSuccess: () => {
      const { selectedLayers, selectLayers } = useKoharuStore.getState()
      selectLayers(selectedLayers.filter((selected) => !ids.includes(selected)))
    },
  })

  const name = plainText(isBlockLayer(layer) ? (layer.translation ?? layer.source) : layer)
  const field =
    'min-h-8 w-full max-w-full min-w-0 resize-none overflow-hidden rounded-md bg-background px-1.5 py-1 text-[12px] leading-4 md:text-[12px]'
  return (
    <div
      ref={row}
      data-selected={selected}
      className='group grid grid-cols-[auto_minmax(0,1fr)] gap-1 rounded-md p-1 data-[selected=true]:bg-accent'
      onFocus={focus}
      // The canvas outlines only the texts of the page it shows.
      onPointerEnter={() => shown && setStoryHover(layer.id)}
      onPointerLeave={() => setStoryHover(null)}
    >
      <div className='flex flex-col items-center gap-1'>
        <ReadingNumber number={number} sfx={isBlockLayer(layer) && layer.role === 'sfx'} />
        {isBlockLayer(layer) && (
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  type='button'
                  variant='outline'
                  size='icon-xs'
                  aria-label={t('story.translate', { name: plainText(layer.source) })}
                  disabled={locked}
                  className='size-5 rounded-md border-primary/20 bg-primary/15 text-accent-foreground opacity-0 group-hover:opacity-100 group-data-[selected=true]:opacity-100 hover:bg-primary/25 hover:text-accent-foreground focus-visible:opacity-100 dark:border-primary/30 dark:bg-primary/25 dark:hover:bg-primary/35'
                  onClick={() => send(commands.runPipeline, { blocks: [layer.id] }, ['translate'])}
                />
              }
            >
              <Languages className='size-3' />
            </TooltipTrigger>
            <TooltipContent side='left'>
              {t('story.translate', { name: plainText(layer.source) })}
            </TooltipContent>
          </Tooltip>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                type='button'
                variant='outline'
                size='icon-xs'
                aria-label={t('navigator.actionsFor', { page: name })}
                className='size-5 rounded-md text-muted-foreground opacity-0 group-hover:opacity-100 group-data-[selected=true]:opacity-100 focus-visible:opacity-100 aria-expanded:opacity-100'
              />
            }
          >
            <MoreHorizontal className='size-3' />
          </DropdownMenuTrigger>
          <DropdownMenuContent align='start' className='w-auto min-w-20'>
            <DropdownMenuItem
              variant='destructive'
              disabled={locked}
              onClick={() => removal.mutate()}
            >
              {t('navigator.delete')}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <div className='grid min-w-0 gap-1'>
        {isBlockLayer(layer) ? (
          <>
            {layer.role === 'sfx' && (
              <span className='px-0.5 text-[8px] font-medium tracking-[0.06em] text-muted-foreground uppercase'>
                {t('story.sfx')}
              </span>
            )}
            <CommitTextarea
              aria-label={t('layers.sourceLabel', { name })}
              placeholder={t('inspector.source')}
              wrap='soft'
              disabled={locked}
              className={`${field} text-muted-foreground`}
              value={plainText(layer.source)}
              onCommit={(text) => send(setSourceText, layer.id, text)}
            />
            <CommitTextarea
              aria-label={t('layers.translationLabel', { name })}
              placeholder={t('inspector.translation')}
              wrap='soft'
              disabled={locked}
              className={`${field} border-primary/25`}
              value={plainText(layer.translation)}
              onCommit={(text) => send(setText, layer.id, text)}
            />
          </>
        ) : (
          <CommitTextarea
            aria-label={t('layers.textLabel', { name })}
            placeholder={t('inspector.text')}
            wrap='soft'
            disabled={locked}
            className={field}
            value={plainText(layer)}
            onCommit={(text) => send(setText, layer.id, text)}
          />
        )}
      </div>
    </div>
  )
}
