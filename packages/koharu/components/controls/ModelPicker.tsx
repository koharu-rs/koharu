'use client'

import { useVirtualizer } from '@tanstack/react-virtual'
import { LoaderCircle } from 'lucide-react'
import { useImperativeHandle, useMemo, useRef, useState, type Ref } from 'react'
import { useTranslation } from 'react-i18next'

import { MenuPanel } from '@/components/controls/MenuPanel'
import { PickerSearch } from '@/components/controls/PickerSearch'
import { vendorName } from '@/lib/vendors'
import type { Model } from '@koharu/ipc/protocol'
import {
  Combobox,
  ComboboxEmpty,
  ComboboxItem,
  ComboboxList,
  useComboboxFilteredItems,
} from '@koharu/ui/components/combobox'
import { ScrollArea } from '@koharu/ui/components/scroll-area'

/** A row of the list: a vendor's heading, or one of its models with its place among the matches. */
type Row = { vendor: string } | { model: Model; index: number }

/** A row's height in pixels: a model's line, or a vendor's heading. */
const height = (row: Row | undefined) => (row && 'model' in row ? 28 : 24)

/** The list's height in pixels, which it opens with the model in use in the middle of. */
const LIST_HEIGHT = 288

/** Picks a model by its `vendor/model` reference, from Koharu's own and each vendor's that has a
 * key or a sign-in, as Claude's and Codex's model menus do: a search line, the models under their
 * vendors one line each, the one in use checked, and the keyboard's arrows and Enter. `onBack`
 * gives it a heading that goes back, for a menu it opens inside of. */
export function ModelPicker({
  value,
  models,
  loading = false,
  disabled = false,
  onBack,
  onSelect,
}: {
  value: string | null
  models: Model[]
  loading?: boolean
  disabled?: boolean
  onBack?: () => void
  onSelect: (reference: string) => void
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState('')
  const list = useRef<{ reveal: (index: number) => void }>(null)
  // Each vendor's models together, the vendors in the order they first come.
  const ordered = useMemo(() => {
    const vendors = [...new Set(models.map((model) => model.vendor))]
    return vendors.flatMap((vendor) => models.filter((model) => model.vendor === vendor))
  }, [models])

  const body = (
    <Combobox<Model>
      inline
      open
      virtualized
      items={ordered}
      value={ordered.find((model) => model.reference === value) ?? null}
      inputValue={query}
      readOnly={disabled}
      itemToStringLabel={(model) => model.name}
      isItemEqualToValue={(model, other) => model.reference === other.reference}
      filter={(model, text) => {
        const needle = text.trim().toLocaleLowerCase()
        return [model.name, model.reference, model.vendor].some((candidate) =>
          candidate.toLocaleLowerCase().includes(needle),
        )
      }}
      onInputValueChange={setQuery}
      onValueChange={(model) => model && onSelect(model.reference)}
      onItemHighlighted={(_, { reason, index }) => {
        if (reason === 'keyboard') list.current?.reveal(index)
      }}
    >
      <div className='mb-0.5 border-b border-border/60 pb-1'>
        <PickerSearch query={query} label={t('modelPicker.search')} onClear={() => setQuery('')} />
      </div>
      {/* Its children come only while nothing matches, so it's empty, and hidden, otherwise. */}
      <ComboboxEmpty className='flex items-center gap-2 px-2.5 py-5 empty:hidden'>
        {loading && models.length === 0 && <LoaderCircle className='size-3.5 animate-spin' />}
        {t(
          models.length > 0
            ? 'modelPicker.noResults'
            : loading
              ? 'modelPicker.loading'
              : 'modelPicker.empty',
        )}
      </ComboboxEmpty>
      <ComboboxList aria-label={t('modelPicker.title')} className='max-h-none overflow-visible p-0'>
        {/* Anew for each query, so a filtered list starts at its top. */}
        <ModelList key={query} selected={value} ref={list} />
      </ComboboxList>
    </Combobox>
  )
  return onBack ? (
    <MenuPanel title={t('modelPicker.title')} backDisabled={disabled} onBack={onBack}>
      {body}
    </MenuPanel>
  ) : (
    <div className='min-w-0 overflow-hidden'>{body}</div>
  )
}

/** The matches under their vendors' headings. Only the rows in view exist, as a vendor such as
 * OpenRouter lists hundreds of models; `reveal` scrolls to a match the keyboard reaches. */
function ModelList({
  selected,
  ref,
}: {
  selected: string | null
  ref: Ref<{ reveal: (index: number) => void }>
}) {
  const { t } = useTranslation()
  const matches = useComboboxFilteredItems<Model>()
  const rows = useMemo(
    () =>
      matches.flatMap((model, index): Row[] =>
        matches[index - 1]?.vendor === model.vendor
          ? [{ model, index }]
          : [{ vendor: model.vendor }, { model, index }],
      ),
    [matches],
  )
  const current = rows.findIndex((row) => 'model' in row && row.model.reference === selected)
  const viewport = useRef<HTMLDivElement>(null)
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => viewport.current,
    getItemKey: (index) => {
      const row = rows[index]
      return row ? ('model' in row ? row.model.reference : `vendor:${row.vendor}`) : index
    },
    estimateSize: (index) => height(rows[index]),
    paddingStart: 2,
    paddingEnd: 2,
    overscan: 8,
    // The list opens on the model in use, wherever it is, in its middle. The virtualizer scrolls
    // here each time it takes the list, as a second mount in Strict Mode does, where a scroll of
    // the list's own would be undone.
    initialOffset: () =>
      current < 0
        ? 0
        : Math.max(
            0,
            rows.slice(0, current).reduce((top, row) => top + height(row), 2) +
              (28 - LIST_HEIGHT) / 2,
          ),
    initialRect: { width: 256, height: LIST_HEIGHT },
  })

  useImperativeHandle(ref, () => ({
    reveal: (index) =>
      virtualizer.scrollToIndex(
        rows.findIndex((row) => 'model' in row && row.index === index),
        { align: 'auto' },
      ),
  }))

  if (rows.length === 0) return null
  return (
    <ScrollArea
      className='max-h-72 min-w-0 overflow-hidden'
      viewportClassName='h-auto max-h-72 min-w-0 overscroll-contain'
      viewportRef={viewport}
    >
      <div className='relative w-full min-w-0' style={{ height: virtualizer.getTotalSize() }}>
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
          return (
            <ComboboxItem
              key={item.key}
              index={row.index}
              value={row.model}
              aria-label={t('modelPicker.useModel', {
                model: row.model.name,
                provider: vendorName(row.model.vendor, t),
              })}
              aria-setsize={matches.length}
              aria-posinset={row.index + 1}
              className='absolute top-0 left-0 h-7'
              style={place}
            >
              <span className='min-w-0 flex-1 truncate'>{row.model.name}</span>
            </ComboboxItem>
          )
        })}
      </div>
    </ScrollArea>
  )
}
