'use client'

import { Languages } from 'lucide-react'
import { useEffect, useMemo, useRef } from 'react'
import { useTranslation } from 'react-i18next'

import { CommitTextarea } from '@/components/controls/CommitTextarea'
import { call, dispatch } from '@/lib/backend'
import {
  findLayer,
  isBlockLayer,
  isLockedLayer,
  isTextLayer,
  plainText,
  type BlockLayer,
  type TextLayer,
} from '@/lib/document'
import { usePage, useStory } from '@/lib/queries'
import { setSourceText, setText } from '@/lib/scene'
import { useKoharuStore } from '@/lib/store'
import { commands, type LayerId } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

/** The shown page's texts in the scene's reading order, as Clip Studio Paint's Story Editor shows
 * a page's text: a block's source and translation, and a text layer's text, the hidden ones left
 * out, each numbered as the canvas numbers it while Story shows. It follows the canvas to another page, as
 * Layers beside it does. Editing a field edits its layer, and focusing it selects the layer. */
export function StoryPanel() {
  const { t } = useTranslation()
  const page = usePage().data
  const order = useStory().data
  // The scene's order, each text found in the page as it shows; one the page doesn't have yet,
  // between the two reads of a change, waits for the next.
  const texts = useMemo(
    () =>
      page && order
        ? order.flatMap((id) => {
            const placed = findLayer(page.layers, id)
            return placed && (isBlockLayer(placed.layer) || isTextLayer(placed.layer))
              ? [{ layer: placed.layer, locked: isLockedLayer(placed) }]
              : []
          })
        : [],
    [page, order],
  )

  return (
    <ScrollArea className='min-h-0 flex-1'>
      <div className='grid gap-1 p-1'>
        {texts.map(({ layer, locked }, index) => (
          <StoryText key={layer.id} number={index + 1} layer={layer} locked={locked} />
        ))}
        {(!page || texts.length === 0) && (
          <p className='px-4 py-8 text-center text-[10px] leading-4 text-muted-foreground'>
            {t(page ? 'story.noText' : 'inspector.selectPage')}
          </p>
        )}
      </div>
    </ScrollArea>
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
 * selected, and scrolled into view when the canvas or Layers selects it. A block shows a button
 * under its number, while the pointer is on it or it's selected, that translates it alone. */
function StoryText({
  number,
  layer,
  locked,
}: {
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

  useEffect(() => {
    const element = row.current
    if (selected && element && !element.contains(document.activeElement)) {
      element.scrollIntoView({ block: 'nearest' })
    }
  }, [selected])

  const focus = () => {
    const { selectedLayers, selectLayers } = useKoharuStore.getState()
    if (!selectedLayers.includes(layer.id)) selectLayers([layer.id])
  }

  const name = plainText(isBlockLayer(layer) ? (layer.translation ?? layer.source) : layer)
  const field =
    'max-h-24 min-h-8 w-full max-w-full min-w-0 resize-y overflow-y-auto rounded-md bg-background px-1.5 py-1 text-[12px] leading-4 md:text-[12px]'
  return (
    <div
      ref={row}
      data-selected={selected}
      className='group grid grid-cols-[auto_minmax(0,1fr)] gap-1 rounded-md p-1 data-[selected=true]:bg-accent'
      onFocus={focus}
      onPointerEnter={() => setStoryHover(layer.id)}
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
                  onClick={() =>
                    void call(commands.runPipeline, { blocks: [layer.id] }, ['translate']).catch(
                      () => undefined,
                    )
                  }
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
              onCommit={(text) => dispatch(setSourceText, layer.id, text)}
            />
            <CommitTextarea
              aria-label={t('layers.translationLabel', { name })}
              placeholder={t('inspector.translation')}
              wrap='soft'
              disabled={locked}
              className={`${field} border-primary/25`}
              value={plainText(layer.translation)}
              onCommit={(text) => dispatch(setText, layer.id, text)}
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
            onCommit={(text) => dispatch(setText, layer.id, text)}
          />
        )}
      </div>
    </div>
  )
}
