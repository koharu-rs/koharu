'use client'

import { observeElementRect, useVirtualizer } from '@tanstack/react-virtual'
import { ChevronDown, Search, X } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useFontPreview } from '@/lib/queries'
import type { FontFamily } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Empty, EmptyTitle } from '@koharu/ui/components/empty'
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from '@koharu/ui/components/input-group'
import { Popover, PopoverContent, PopoverTrigger } from '@koharu/ui/components/popover'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Separator } from '@koharu/ui/components/separator'
import { cn } from '@koharu/ui/lib/utils'

const rowHeight = 38
const listHeight = 248

export function FontPicker({
  value,
  families,
  disabled,
  size = 'default',
  ariaLabel,
  placeholder,
  onChange,
}: {
  value: string
  families: FontFamily[]
  disabled?: boolean
  size?: 'default' | 'sm'
  ariaLabel?: string
  placeholder?: string
  onChange: (family: string) => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const input = useRef<HTMLInputElement>(null)
  const orderedFamilies = useMemo(
    () =>
      [...families].sort((left, right) =>
        left.name.localeCompare(right.name, undefined, { numeric: true, sensitivity: 'base' }),
      ),
    [families],
  )
  const results = useMemo(() => {
    const normalized = normalizeFontName(query)
    return orderedFamilies.filter(
      (family) =>
        !normalized ||
        [family.name, ...family.faces.map((face) => face.post_script_name)].some((name) =>
          normalizeFontName(name).includes(normalized),
        ),
    )
  }, [orderedFamilies, query])
  const selectedFamily = useMemo(
    () =>
      orderedFamilies.find((family) => normalizeFontName(family.name) === normalizeFontName(value)),
    [orderedFamilies, value],
  )

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) setQuery('')
      }}
    >
      <PopoverTrigger
        render={
          <Button
            variant='outline'
            size={size === 'sm' ? 'xs' : 'default'}
            className='w-full min-w-0 justify-between font-normal'
          />
        }
        disabled={disabled}
        aria-label={ariaLabel}
        data-testid='type-font-picker'
        className={cn(
          'w-full min-w-0 justify-between font-normal',
          size === 'sm' ? 'h-6 gap-1 px-1.5 text-[11px]' : 'h-8 gap-2 px-2.5 text-[12px]',
        )}
      >
        {selectedFamily ? (
          <FontPreviewLabel
            family={selectedFamily}
            className='min-w-0 flex-1 py-1 [&>img]:max-h-3.5'
          />
        ) : (
          <span className='truncate'>{value || placeholder || t('fontPicker.choose')}</span>
        )}
        <ChevronDown className='size-3.5 shrink-0 text-muted-foreground' />
      </PopoverTrigger>
      <PopoverContent
        align='start'
        className='w-(--anchor-width) min-w-44 gap-0 overflow-hidden rounded-lg p-0'
        initialFocus={input}
      >
        <div className='border-b p-1'>
          <InputGroup className='h-6 border-0 bg-muted/50 shadow-none focus-within:bg-background has-[>[data-align=inline-start]]:[&>input]:pl-0'>
            <InputGroupAddon className='pr-1 pl-1.5'>
              <Search className='size-3' />
            </InputGroupAddon>
            <InputGroupInput
              ref={input}
              value={query}
              aria-label={t('fontPicker.search')}
              placeholder={t('fontPicker.search')}
              className='h-6 px-0 text-[11px]'
              onChange={(event) => setQuery(event.currentTarget.value)}
            />
            {query && (
              <InputGroupAddon align='inline-end' className='gap-0.5 pr-0.5'>
                <InputGroupButton aria-label={t('common.clearSearch')} onClick={() => setQuery('')}>
                  <X />
                </InputGroupButton>
              </InputGroupAddon>
            )}
          </InputGroup>
        </div>
        <div className='flex h-6 items-center px-2 text-[9px] text-muted-foreground'>
          {t('fontPicker.fontCount', { count: results.length })}
        </div>
        <Separator />
        {open && (
          <FontList
            key={query}
            families={results}
            value={value}
            onSelect={(family) => {
              onChange(family)
              setOpen(false)
            }}
          />
        )}
      </PopoverContent>
    </Popover>
  )
}

function FontList({
  families,
  value,
  onSelect,
}: {
  families: FontFamily[]
  value: string
  onSelect: (family: string) => void
}) {
  const { t } = useTranslation()
  const list = useRef<HTMLDivElement>(null)
  const selectedIndex = families.findIndex(
    (family) => normalizeFontName(family.name) === normalizeFontName(value),
  )
  const virtualizer = useVirtualizer({
    count: families.length,
    getScrollElement: () => list.current,
    getItemKey: (index) => normalizeFontName(families[index]?.name ?? String(index)),
    estimateSize: () => rowHeight,
    overscan: 6,
    initialOffset: Math.max(
      0,
      selectedIndex * rowHeight - Math.floor((listHeight / rowHeight - 1) / 2) * rowHeight,
    ),
    initialRect: { width: 240, height: listHeight },
    observeElementRect: (instance, callback) =>
      observeElementRect(instance, (rect) =>
        callback({ width: rect.width || 240, height: rect.height || listHeight }),
      ),
  })

  return (
    <ScrollArea
      viewportRef={list}
      role='listbox'
      aria-label={t('fontPicker.fonts')}
      className='relative'
      viewportClassName='overflow-x-hidden'
      style={{
        height:
          families.length === 0
            ? rowHeight * 2
            : Math.min(listHeight, Math.max(rowHeight, families.length * rowHeight)),
      }}
    >
      <div className='relative' style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const family = families[virtualRow.index]
          if (!family) return null
          const selected = normalizeFontName(family.name) === normalizeFontName(value)
          return (
            <button
              key={virtualRow.key}
              type='button'
              role='option'
              aria-label={family.name}
              aria-selected={selected}
              className={cn(
                'absolute inset-x-0 top-0 flex h-9.5 w-full items-center px-2 text-left hover:bg-accent focus-visible:bg-accent focus-visible:outline-none',
                selected && 'bg-accent text-accent-foreground',
              )}
              style={{ transform: `translateY(${virtualRow.start}px)` }}
              onClick={() => onSelect(family.name)}
            >
              <div className='flex min-w-0 flex-1 flex-col justify-center gap-px'>
                <FontPreviewLabel family={family} className='h-4.5 w-full min-w-0' />
                <span className='min-w-0 truncate text-[8px] leading-none text-muted-foreground'>
                  {family.name}
                </span>
              </div>
            </button>
          )
        })}
      </div>
      {families.length === 0 && (
        <Empty role='status' className='absolute inset-1 rounded-md p-2'>
          <EmptyTitle className='text-[11px] text-muted-foreground'>
            {t('fontPicker.noResults')}
          </EmptyTitle>
        </Empty>
      )}
    </ScrollArea>
  )
}

function FontPreviewLabel({ family, className }: { family: FontFamily; className?: string }) {
  const previewQuery = useFontPreview(family)
  const preview = previewQuery.data
  const [url, setUrl] = useState<string | null>(null)

  useEffect(() => {
    setUrl(null)
    if (!preview) return
    const next = URL.createObjectURL(new Blob([preview.buffer], { type: 'image/webp' }))
    setUrl(next)
    return () => URL.revokeObjectURL(next)
  }, [preview])

  return url ? (
    <span className={cn('flex h-full min-w-0 items-center', className)}>
      <img
        src={url}
        alt={family.name}
        className='max-h-[18px] max-w-full object-contain object-left dark:invert'
      />
    </span>
  ) : (
    <span className={cn('truncate font-sans', className)}>{family.name}</span>
  )
}

function normalizeFontName(value: string): string {
  return value.normalize('NFKC').trim().replace(/\s+/g, ' ').toLowerCase()
}
