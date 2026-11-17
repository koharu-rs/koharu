'use client'

import { useVirtualizer } from '@tanstack/react-virtual'
import { Check, ChevronLeft, LoaderCircle, Search, X } from 'lucide-react'
import { useId, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { vendorName } from '@/components/preferences/models'
import type { Model } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { cn } from '@koharu/ui/lib/utils'

/** A row of the menu: a vendor's heading, or one of its models. */
type Row = { vendor: string } | { model: Model; option: number }

/** Picks a model by its `vendor/model` reference, from Koharu's own and each vendor's that has a
 * key or a sign-in, as Claude's and Codex's model menus do: a search line, the models under their
 * vendors one line each, the one in use checked, and the keyboard's arrows, Enter, and Escape. Only
 * the rows in view exist, as a vendor such as OpenRouter lists hundreds of models. `onBack` gives
 * it a heading that goes back, for a menu it opens inside of. */
export function ModelPicker({
  value,
  models,
  loading = false,
  disabled = false,
  busyModel,
  onBack,
  onSelect,
}: {
  value: string | null
  models: Model[]
  loading?: boolean
  disabled?: boolean
  busyModel?: string | null
  onBack?: () => void
  onSelect: (reference: string) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const [query, setQuery] = useState('')
  const saving = Boolean(busyModel)
  const normalizedQuery = query.trim().toLocaleLowerCase()
  // The matches under their vendors, in the order the vendors first come.
  const { rows, options } = useMemo(() => {
    const groups = new Map<string, Model[]>()
    for (const model of models) {
      const match =
        !normalizedQuery ||
        [model.name, model.reference, model.vendor].some((candidate) =>
          candidate.toLocaleLowerCase().includes(normalizedQuery),
        )
      if (match) groups.set(model.vendor, [...(groups.get(model.vendor) ?? []), model])
    }
    const rows: Row[] = []
    const options: number[] = []
    for (const [vendor, members] of groups) {
      rows.push({ vendor })
      for (const model of members) {
        options.push(rows.length)
        rows.push({ model, option: options.length - 1 })
      }
    }
    return { rows, options }
  }, [models, normalizedQuery])
  const current = rows.findIndex((row) => 'model' in row && row.model.reference === value)
  // The option the keyboard moves, which the pointer moves too.
  const [active, setActive] = useState(() => {
    const row = rows[current]
    return row && 'model' in row ? row.option : 0
  })
  const shown = Math.min(active, Math.max(options.length - 1, 0))

  const list = useRef<HTMLDivElement>(null)
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => list.current,
    getItemKey: (index) => {
      const row = rows[index]
      return row ? ('model' in row ? row.model.reference : `vendor:${row.vendor}`) : index
    },
    estimateSize: (index) => {
      const row = rows[index]
      return row && 'model' in row ? 28 : 24
    },
    paddingStart: 2,
    paddingEnd: 2,
    overscan: 8,
    initialRect: { width: 256, height: 288 },
  })
  // The menu opens on the model in use, wherever it is in the list, scrolled before it paints. The
  // list is scrolled itself: the virtualizer's own scroll can be dropped when Strict Mode mounts
  // it twice, and this runs once.
  const opened = useRef(false)
  useLayoutEffect(() => {
    const viewport = list.current
    if (opened.current || current < 0 || !viewport) return
    opened.current = true
    viewport.scrollTop = virtualizer.getOffsetForIndex(current, 'center')?.[0] ?? 0
  }, [current, virtualizer])

  const move = (by: number) => {
    if (!options.length) return
    const next = (shown + by + options.length) % options.length
    setActive(next)
    virtualizer.scrollToIndex(options[next] ?? 0, { align: 'auto' })
  }
  const pick = (model: Model) => {
    if (!disabled && !saving) onSelect(model.reference)
  }

  return (
    <div className='min-w-0 overflow-hidden'>
      {onBack && (
        <div className='mb-1 flex h-7 items-center border-b border-border/60 px-0.5 pb-1'>
          <Button
            type='button'
            variant='ghost'
            size='icon-xs'
            aria-label={t('common.back')}
            className='rounded-md text-muted-foreground hover:bg-primary/10 hover:text-foreground'
            disabled={saving}
            onClick={onBack}
          >
            <ChevronLeft className='size-3.5' />
          </Button>
          <span className='ml-1 min-w-0 flex-1 truncate text-[11px] font-medium'>
            {t('modelPicker.title')}
          </span>
        </div>
      )}

      <label className='mb-0.5 flex h-8 items-center gap-1.5 border-b border-border/60 px-2'>
        <Search className='size-3.5 shrink-0 text-muted-foreground' />
        <input
          autoFocus
          value={query}
          role='combobox'
          aria-expanded
          aria-controls={`${id}-models`}
          aria-activedescendant={options.length ? `${id}-${options[shown]}` : undefined}
          aria-label={t('modelPicker.search')}
          placeholder={t('modelPicker.search')}
          className='h-full min-w-0 flex-1 bg-transparent text-[11px] outline-none placeholder:text-muted-foreground'
          onChange={(event) => {
            setQuery(event.currentTarget.value)
            setActive(0)
            virtualizer.scrollToOffset(0)
          }}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
              event.preventDefault()
              move(event.key === 'ArrowDown' ? 1 : -1)
            } else if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
              const row = rows[options[shown] ?? -1]
              if (row && 'model' in row) {
                event.preventDefault()
                pick(row.model)
              }
            }
          }}
        />
        {query && (
          <button
            type='button'
            aria-label={t('common.clearSearch')}
            className='grid size-5 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground'
            onClick={() => setQuery('')}
          >
            <X className='size-3' />
          </button>
        )}
      </label>

      <ScrollArea
        className='max-h-72 min-w-0 overflow-hidden'
        viewportClassName='h-auto max-h-72 min-w-0 overscroll-contain'
        viewportRef={list}
      >
        {rows.length > 0 ? (
          <div
            id={`${id}-models`}
            role='listbox'
            aria-label={t('modelPicker.title')}
            className='relative w-full min-w-0'
            style={{ height: virtualizer.getTotalSize() }}
          >
            {virtualizer.getVirtualItems().map((item) => {
              const row = rows[item.index]
              if (!row) return null
              const place = { transform: `translateY(${item.start}px)` }
              if (!('model' in row)) {
                return (
                  <div
                    key={item.key}
                    role='presentation'
                    className='absolute top-0 left-0 flex h-6 w-full items-end truncate px-2 pb-0.5 text-[10px] text-muted-foreground'
                    style={place}
                  >
                    {vendorName(row.vendor, t)}
                  </div>
                )
              }
              const { model, option } = row
              const selected = model.reference === value
              return (
                <div
                  key={item.key}
                  id={`${id}-${item.index}`}
                  role='option'
                  aria-selected={selected}
                  aria-disabled={disabled || saving}
                  aria-label={t('modelPicker.useModel', {
                    model: model.name,
                    provider: vendorName(model.vendor, t),
                  })}
                  data-active={option === shown}
                  className={cn(
                    'absolute top-0 left-0 flex h-7 w-full min-w-0 cursor-default items-center gap-2 rounded-md px-2 text-[11px] select-none data-[active=true]:bg-accent data-[active=true]:text-accent-foreground',
                    (disabled || saving) && 'opacity-50',
                  )}
                  style={place}
                  // Only the pointer's own moves: one the browser sends as rows pass under a
                  // still pointer, as the menu opens or scrolls, moves nothing.
                  onPointerMove={(event) => {
                    if ((event.movementX || event.movementY) && option !== shown) setActive(option)
                  }}
                  onPointerDown={(event) => event.preventDefault()}
                  onClick={() => pick(model)}
                >
                  <span className='min-w-0 flex-1 truncate'>{model.name}</span>
                  {busyModel === model.reference ? (
                    <LoaderCircle className='size-3.5 shrink-0 animate-spin text-primary' />
                  ) : (
                    selected && <Check className='size-3.5 shrink-0' />
                  )}
                </div>
              )
            })}
          </div>
        ) : (
          <div className='flex items-center justify-center gap-2 px-2.5 py-5 text-center text-[11px] text-muted-foreground'>
            {loading && models.length === 0 && <LoaderCircle className='size-3.5 animate-spin' />}
            {t(
              models.length > 0
                ? 'modelPicker.noResults'
                : loading
                  ? 'modelPicker.loading'
                  : 'modelPicker.empty',
            )}
          </div>
        )}
      </ScrollArea>
    </div>
  )
}
