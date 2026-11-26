'use client'

import { observeElementRect, useVirtualizer } from '@tanstack/react-virtual'
import { Star } from 'lucide-react'
import { useEffect, useImperativeHandle, useMemo, useRef, useState, type Ref } from 'react'
import { useTranslation } from 'react-i18next'
import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import { PickerSearch } from '@/components/controls/PickerSearch'
import { useFontPreview } from '@/lib/queries'
import type { FontFamily } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxItem,
  ComboboxList,
  ComboboxTrigger,
  useComboboxFilteredItems,
} from '@koharu/ui/components/combobox'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Separator } from '@koharu/ui/components/separator'
import { Toggle } from '@koharu/ui/components/toggle'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

const rowHeight = 38
const listHeight = 248

/** The families starred as favorites, by normalized name, and whether the picker lists only them,
 * as Photoshop's font menu keeps favorites and its Show Favorite Fonts filter. Every picker shares
 * them, and the interface keeps them in its browser profile, as it does the editor's panel widths. */
const useFavoriteFonts = create<{
  favorites: string[]
  only: boolean
  toggle: (family: string) => void
  showOnly: (only: boolean) => void
}>()(
  persist(
    (set) => ({
      favorites: [],
      only: false,
      toggle: (family) =>
        set(({ favorites }) => {
          const name = normalizeFontName(family)
          return {
            favorites: favorites.includes(name)
              ? favorites.filter((favorite) => favorite !== name)
              : [...favorites, name],
          }
        }),
      showOnly: (only) => set({ only }),
    }),
    { name: 'favorite-fonts' },
  ),
)

/** Picks a font family, each shown in its own face: a search line that matches a family's name or
 * any of its faces' PostScript names, and a star beside it that lists only the favorites, then the
 * matches in alphabetical order, the keyboard's arrows and Enter moving through them. With
 * `fallback`, an entry above them picks none, as Firefox's font menus list Default first; with no
 * `value`, it shows as the picker's. A `ghost` one draws only its value until pointed at, for a list
 * of them. */
export function FontPicker({
  value,
  families,
  disabled,
  size = 'default',
  variant = 'outline',
  ariaLabel,
  placeholder,
  fallback,
  onChange,
}: {
  value: string
  families: FontFamily[]
  disabled?: boolean
  size?: 'default' | 'sm'
  variant?: 'outline' | 'ghost'
  ariaLabel?: string
  placeholder?: string
  fallback?: { label: string; onSelect: () => void }
  onChange: (family: string) => void
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState('')
  const [open, setOpen] = useState(false)
  const list = useRef<{ reveal: (index: number) => void }>(null)
  const orderedFamilies = useMemo(
    () =>
      [...families].sort((left, right) =>
        left.name.localeCompare(right.name, undefined, { numeric: true, sensitivity: 'base' }),
      ),
    [families],
  )
  const selectedFamily = useMemo(
    () =>
      orderedFamilies.find((family) => normalizeFontName(family.name) === normalizeFontName(value)),
    [orderedFamilies, value],
  )
  const { favorites, only, showOnly } = useFavoriteFonts()
  const listed = useMemo(
    () =>
      only
        ? orderedFamilies.filter((family) => favorites.includes(normalizeFontName(family.name)))
        : orderedFamilies,
    [only, favorites, orderedFamilies],
  )

  return (
    <Combobox<FontFamily>
      virtualized
      items={listed}
      value={selectedFamily ?? null}
      inputValue={query}
      open={open}
      disabled={disabled}
      itemToStringLabel={(family) => family.name}
      isItemEqualToValue={(family, other) =>
        normalizeFontName(family.name) === normalizeFontName(other.name)
      }
      filter={(family, text) => {
        const normalized = normalizeFontName(text)
        return [family.name, ...family.faces.map((face) => face.post_script_name)].some((name) =>
          normalizeFontName(name).includes(normalized),
        )
      }}
      onInputValueChange={setQuery}
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) setQuery('')
      }}
      onValueChange={(family) => family && onChange(family.name)}
      onItemHighlighted={(_, { reason, index }) => {
        if (reason === 'keyboard') list.current?.reveal(index)
      }}
    >
      <ComboboxTrigger
        render={
          <Button
            variant={variant}
            size={size === 'sm' ? 'xs' : 'default'}
            className='w-full min-w-0 justify-between font-normal'
          />
        }
        aria-label={ariaLabel}
        data-testid='type-font-picker'
        className={cn(
          'w-full min-w-0 justify-between font-normal [&>svg:last-child]:size-3.5',
          size === 'sm' ? 'h-6 gap-1 px-1.5 text-[11px]' : 'h-8 gap-2 px-2.5 text-[12px]',
        )}
      >
        {selectedFamily ? (
          <FontPreviewLabel
            family={selectedFamily}
            className='min-w-0 flex-1 py-1 [&>img]:max-h-3.5'
          />
        ) : (
          <span className={cn('truncate', !value && 'text-muted-foreground')}>
            {value || fallback?.label || placeholder || t('fontPicker.choose')}
          </span>
        )}
      </ComboboxTrigger>
      <ComboboxContent className='w-(--anchor-width) min-w-44'>
        <div className='flex items-center gap-1 border-b p-1'>
          <div className='min-w-0 flex-1'>
            <PickerSearch
              query={query}
              label={t('fontPicker.search')}
              onClear={() => setQuery('')}
            />
          </div>
          <Tooltip>
            <TooltipTrigger
              render={
                <Toggle
                  size='sm'
                  aria-label={t('fontPicker.favoritesOnly')}
                  pressed={only}
                  onPressedChange={showOnly}
                  // The search line keeps the keys, so the arrows still move through the list.
                  onMouseDown={(event) => event.preventDefault()}
                  className='size-7 shrink-0 px-0 text-muted-foreground aria-pressed:bg-transparent aria-pressed:text-primary aria-pressed:hover:bg-muted'
                />
              }
            >
              <Star className={cn('size-3.5', only && 'fill-current')} />
            </TooltipTrigger>
            <TooltipContent>{t('fontPicker.favoritesOnly')}</TooltipContent>
          </Tooltip>
        </div>
        {fallback && (
          <>
            <button
              type='button'
              aria-pressed={!value}
              className={cn(
                'flex h-8 w-full items-center px-2 text-left text-[11px] outline-none hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent',
                !value && 'bg-accent text-accent-foreground',
              )}
              onClick={() => {
                fallback.onSelect()
                setOpen(false)
              }}
            >
              <span className='truncate'>{fallback.label}</span>
            </button>
            <Separator />
          </>
        )}
        <ComboboxEmpty className='py-4'>
          {only && favorites.length === 0 ? t('fontPicker.noFavorites') : t('fontPicker.noResults')}
        </ComboboxEmpty>
        <ComboboxList
          aria-label={t('fontPicker.fonts')}
          className='max-h-none overflow-visible p-0'
        >
          {/* Anew for each query, so a filtered list starts at its top. */}
          <FontList key={query} selected={value} ref={list} />
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  )
}

/** The matches, how many, then each in its own face behind its favorite star, opening on
 * `selected`. Only the rows in view exist, as a system can hold thousands of families; `reveal`
 * scrolls to a match the keyboard reaches. */
function FontList({
  selected,
  ref,
}: {
  selected: string
  ref: Ref<{ reveal: (index: number) => void }>
}) {
  const { t } = useTranslation()
  const families = useComboboxFilteredItems<FontFamily>()
  const { favorites, toggle } = useFavoriteFonts()
  const selectedIndex = families.findIndex(
    (family) => normalizeFontName(family.name) === normalizeFontName(selected),
  )
  const viewport = useRef<HTMLDivElement>(null)
  const virtualizer = useVirtualizer({
    count: families.length,
    getScrollElement: () => viewport.current,
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

  useImperativeHandle(ref, () => ({
    reveal: (index) => virtualizer.scrollToIndex(index, { align: 'auto' }),
  }))

  if (families.length === 0) return null
  return (
    <>
      <div className='flex h-6 items-center px-2 text-[9px] text-muted-foreground'>
        {t('fontPicker.fontCount', { count: families.length })}
      </div>
      <Separator />
      <ScrollArea
        viewportRef={viewport}
        className='relative'
        viewportClassName='overflow-x-hidden'
        style={{ height: Math.min(listHeight, families.length * rowHeight) }}
      >
        <div className='relative' style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((row) => {
            const family = families[row.index]
            if (!family) return null
            const favorite = favorites.includes(normalizeFontName(family.name))
            return (
              <ComboboxItem
                key={row.key}
                index={row.index}
                value={family}
                aria-label={family.name}
                aria-setsize={families.length}
                aria-posinset={row.index + 1}
                // The chosen family shows in the accent, as a bar's lists show their chosen item,
                // with no tick. Base UI marks it with a bare `data-selected`, which shadcn's
                // `data-selected:` variant, written for `="true"`, doesn't match. Its right is as
                // wide as the star's strip and gaps on its left, so the face sits between even
                // margins.
                className='group/font absolute inset-x-0 top-0 h-9.5 gap-1 rounded-none pr-5.5 pl-1 data-[selected]:bg-accent data-[selected]:text-accent-foreground data-[selected]:**:text-accent-foreground [&>span]:hidden!'
                style={{ transform: `translateY(${row.start}px)` }}
              >
                {/* Starred with the pointer, as in Photoshop's font menu; its click stops here, so
                    it doesn't pick the family, and the item keeps focus on the search line. */}
                <button
                  type='button'
                  tabIndex={-1}
                  aria-pressed={favorite}
                  aria-label={t(favorite ? 'fontPicker.removeFavorite' : 'fontPicker.addFavorite', {
                    name: family.name,
                  })}
                  className={cn(
                    'grid w-3.5 shrink-0 place-items-center self-stretch text-muted-foreground hover:text-foreground',
                    favorite
                      ? 'text-primary'
                      : 'opacity-0 group-hover/font:opacity-100 group-data-highlighted/font:opacity-100',
                  )}
                  onClick={(event) => {
                    event.stopPropagation()
                    toggle(family.name)
                  }}
                >
                  <Star className={cn('size-3', favorite && 'fill-current')} />
                </button>
                <div className='flex min-w-0 flex-1 flex-col justify-center gap-px'>
                  <FontPreviewLabel family={family} className='h-4.5 w-full min-w-0' />
                  <span className='min-w-0 truncate text-[8px] leading-none text-muted-foreground'>
                    {family.name}
                  </span>
                </div>
              </ComboboxItem>
            )
          })}
        </div>
      </ScrollArea>
    </>
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
