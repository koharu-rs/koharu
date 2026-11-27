'use client'

import { observeElementRect, useVirtualizer } from '@tanstack/react-virtual'
import {
  ArrowLeftRight,
  Check,
  ChevronDown,
  ChevronRight,
  HardDrive,
  LoaderCircle,
  TriangleAlert,
} from 'lucide-react'
import {
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type ComponentProps,
  type KeyboardEvent,
  type Ref,
} from 'react'
import { useTranslation } from 'react-i18next'

import { MenuPanel } from '@/components/controls/MenuPanel'
import { PickerSearch } from '@/components/controls/PickerSearch'
import { classifyVramFit, findCatalogSelection, formatDiskSize, formatVramSize } from '@/lib/models'
import { vendorName } from '@/lib/vendors'
import type { ModelVariant, StageModelEntry, TranslateModelEntry } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  Combobox,
  ComboboxEmpty,
  ComboboxItem,
  ComboboxList,
  useComboboxFilteredItems,
} from '@koharu/ui/components/combobox'
import { Popover, PopoverContent, PopoverTrigger } from '@koharu/ui/components/popover'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

/** A row of the list: a vendor's heading, or one of its models with its place among the matches. */
type Row = { vendor: string } | { model: TranslateModelEntry; index: number }

/** A row's height in pixels: local families carry a second metadata line. */
const height = (row: Row | undefined) =>
  row && 'model' in row ? (row.model.disk_bytes !== null ? 42 : 28) : 24

/** The list's height in pixels, which it opens with the model in use in the middle of. */
const LIST_HEIGHT = 288

/** Renders disk size and download status for a local model or variant. */
export function ModelDiskMeta({
  diskBytes,
  downloaded,
}: {
  diskBytes: number
  downloaded: boolean
}) {
  const { t, i18n } = useTranslation()
  return (
    <span className='inline-flex shrink-0 items-center gap-1 text-[10px] text-muted-foreground tabular-nums'>
      <Tooltip>
        <TooltipTrigger
          render={
            <span
              data-preserve-color=''
              className={cn(
                'inline-flex items-center',
                downloaded ? 'text-emerald-600 dark:text-emerald-400' : 'text-muted-foreground/60',
              )}
              aria-label={t(downloaded ? 'modelPicker.downloaded' : 'modelPicker.notDownloaded')}
            />
          }
        >
          <HardDrive className='size-2.5 shrink-0' />
        </TooltipTrigger>
        <TooltipContent>
          {t(downloaded ? 'modelPicker.downloaded' : 'modelPicker.notDownloaded')}
        </TooltipContent>
      </Tooltip>
      <span>{formatDiskSize(diskBytes, t, i18n.language)}</span>
    </span>
  )
}

/** Renders a VRAM fit pill with icon + label + exact-number tooltip when both measured VRAM and
 * available VRAM are known; renders nothing when unmeasured. */
export function VramFitBadge({
  candidateVramBytes,
  otherStagesVramBytes = 0,
  availableVramBytes,
}: {
  candidateVramBytes: number | null | undefined
  otherStagesVramBytes?: number
  availableVramBytes: number | null | undefined
}) {
  const { t, i18n } = useTranslation()
  const fit = classifyVramFit(candidateVramBytes, otherStagesVramBytes, availableVramBytes)
  if (fit === null || candidateVramBytes == null || availableVramBytes == null) return null

  const model = formatVramSize(candidateVramBytes, t, i18n.language)
  const available = formatVramSize(availableVramBytes, t, i18n.language)
  const tooltip =
    otherStagesVramBytes > 0
      ? t('modelPicker.vramTooltip.withOther', {
          model,
          other: formatVramSize(otherStagesVramBytes, t, i18n.language),
          available,
        })
      : t('modelPicker.vramTooltip.alone', { model, available })

  const Icon = fit === 'fits' ? Check : fit === 'swaps' ? ArrowLeftRight : TriangleAlert
  const tone =
    fit === 'fits'
      ? 'bg-emerald-500/12 text-emerald-700 dark:text-emerald-400'
      : fit === 'swaps'
        ? 'bg-amber-500/12 text-amber-700 dark:text-amber-400'
        : 'bg-destructive/12 text-destructive'

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <span
            data-preserve-color=''
            className={cn(
              'inline-flex shrink-0 items-center gap-0.5 rounded px-1 py-px text-[9px] leading-none font-medium',
              tone,
            )}
          />
        }
      >
        <Icon className='size-2.5 shrink-0' />
        <span>{t(`modelPicker.vramFit.${fit}`)}</span>
      </TooltipTrigger>
      <TooltipContent>{tooltip}</TooltipContent>
    </Tooltip>
  )
}

/** Picks a language model by its `vendor/model` or `koharu/<id>:<quant>` reference, grouping
 * local quantizations under one family row with a drill-down `MenuPanel` for families that offer
 * more than one quantization. */
export function ModelPicker({
  value,
  models,
  availableVramBytes = null,
  otherStagesVramBytes = 0,
  loading = false,
  disabled = false,
  onBack,
  onSelect,
}: {
  value: string | null
  models: TranslateModelEntry[]
  availableVramBytes?: number | null
  otherStagesVramBytes?: number
  loading?: boolean
  disabled?: boolean
  onBack?: () => void
  onSelect: (reference: string) => void
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState('')
  const [variantFamilyRef, setVariantFamilyRef] = useState<string | null>(null)
  const list = useRef<{ reveal: (index: number) => void }>(null)
  const highlighted = useRef<TranslateModelEntry | null>(null)

  // Each vendor's models together, the vendors in the order they first come.
  const ordered = useMemo(() => {
    const vendors = [...new Set(models.map((model) => model.vendor))]
    return vendors.flatMap((vendor) => models.filter((model) => model.vendor === vendor))
  }, [models])

  const selected = useMemo(() => findCatalogSelection(ordered, value), [ordered, value])
  const variantFamily = useMemo(
    () =>
      variantFamilyRef
        ? (ordered.find((model) => model.reference === variantFamilyRef) ?? null)
        : null,
    [ordered, variantFamilyRef],
  )

  if (variantFamily) {
    return (
      <VariantSubmenu
        family={variantFamily}
        selectedValue={value}
        availableVramBytes={availableVramBytes}
        otherStagesVramBytes={otherStagesVramBytes}
        disabled={disabled}
        onBack={() => setVariantFamilyRef(null)}
        onSelect={(reference) => {
          setVariantFamilyRef(null)
          onSelect(reference)
        }}
      />
    )
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ArrowRight') return
    const target = highlighted.current
    if (!target || target.variants.length <= 1) return
    const input = event.target as HTMLInputElement | null
    if (input && typeof input.selectionStart === 'number' && input.value.length > 0) {
      if (input.selectionStart !== input.value.length) return
    }
    event.preventDefault()
    setVariantFamilyRef(target.reference)
  }

  const body = (
    <div className='min-w-0' onKeyDown={handleKeyDown}>
      <Combobox<TranslateModelEntry>
        inline
        open
        virtualized
        items={ordered}
        value={selected?.model ?? null}
        inputValue={query}
        readOnly={disabled}
        itemToStringLabel={(model) => model.name}
        isItemEqualToValue={(model, other) => model.reference === other.reference}
        filter={(model, text) => {
          const needle = text.trim().toLocaleLowerCase()
          if (!needle) return true
          return (
            [model.name, model.reference, model.vendor].some((candidate) =>
              candidate.toLocaleLowerCase().includes(needle),
            ) ||
            model.variants.some(
              (variant) =>
                variant.name.toLocaleLowerCase().includes(needle) ||
                variant.reference.toLocaleLowerCase().includes(needle),
            )
          )
        }}
        onInputValueChange={setQuery}
        onValueChange={(model) => {
          if (!model) return
          const active =
            selected?.model.reference === model.reference && value
              ? value
              : (model.variants[0]?.reference ?? model.reference)
          onSelect(
            model.variants.length > 0 && active === model.variants[0]?.reference
              ? model.reference
              : active,
          )
        }}
        onItemHighlighted={(item, { reason, index }) => {
          highlighted.current = item ?? null
          if (reason === 'keyboard') list.current?.reveal(index)
        }}
      >
        <div className='mb-0.5 border-b border-border/60 pb-1'>
          <PickerSearch
            query={query}
            label={t('modelPicker.search')}
            onClear={() => setQuery('')}
          />
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
        <ComboboxList
          aria-label={t('modelPicker.title')}
          className='max-h-none overflow-visible p-0'
        >
          {/* Anew for each query, so a filtered list starts at its top. */}
          <ModelList
            key={query}
            selectedValue={value}
            selectedFamilyRef={selected?.model.reference ?? null}
            availableVramBytes={availableVramBytes}
            otherStagesVramBytes={otherStagesVramBytes}
            onOpenVariants={setVariantFamilyRef}
            ref={list}
          />
        </ComboboxList>
      </Combobox>
    </div>
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
  selectedValue,
  selectedFamilyRef,
  availableVramBytes,
  otherStagesVramBytes,
  onOpenVariants,
  ref,
}: {
  selectedValue: string | null
  selectedFamilyRef: string | null
  availableVramBytes: number | null
  otherStagesVramBytes: number
  onOpenVariants: (reference: string) => void
  ref: Ref<{ reveal: (index: number) => void }>
}) {
  const { t } = useTranslation()
  const matches = useComboboxFilteredItems<TranslateModelEntry>()
  const rows = useMemo(
    () =>
      matches.flatMap((model, index): Row[] =>
        matches[index - 1]?.vendor === model.vendor
          ? [{ model, index }]
          : [{ vendor: model.vendor }, { model, index }],
      ),
    [matches],
  )
  const current = rows.findIndex(
    (row) => 'model' in row && row.model.reference === selectedFamilyRef,
  )
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
    initialOffset: () =>
      current < 0
        ? 0
        : Math.max(
            0,
            rows.slice(0, current).reduce((top, row) => top + height(row), 2) +
              (28 - LIST_HEIGHT) / 2,
          ),
    initialRect: { width: 256, height: LIST_HEIGHT },
    observeElementRect: (instance, callback) =>
      observeElementRect(instance, (rect) =>
        callback({ width: rect.width || 256, height: rect.height || LIST_HEIGHT }),
      ),
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
          const model = row.model
          const isLocal = model.disk_bytes !== null
          const activeVariant =
            (selectedFamilyRef === model.reference
              ? model.variants.find((variant) => variant.reference === selectedValue)
              : undefined) ??
            model.variants[0] ??
            null
          const diskBytes = activeVariant?.disk_bytes ?? model.disk_bytes
          const downloaded = activeVariant?.downloaded ?? model.downloaded
          const measuredVram = activeVariant?.measured_vram_bytes ?? model.measured_vram_bytes
          const hasMultipleVariants = model.variants.length > 1

          if (!isLocal) {
            return (
              <ComboboxItem
                key={item.key}
                index={row.index}
                value={model}
                aria-label={t('modelPicker.useModel', {
                  model: model.name,
                  provider: vendorName(model.vendor, t),
                })}
                aria-setsize={matches.length}
                aria-posinset={row.index + 1}
                className='absolute top-0 left-0 h-7'
                style={place}
              >
                <span className='min-w-0 flex-1 truncate'>{model.name}</span>
              </ComboboxItem>
            )
          }

          return (
            <div
              key={item.key}
              className='absolute top-0 left-0 flex h-[42px] w-full min-w-0 items-center gap-0.5'
              style={place}
            >
              <ComboboxItem
                index={row.index}
                value={model}
                aria-label={t('modelPicker.useModel', {
                  model: model.name,
                  provider: vendorName(model.vendor, t),
                })}
                aria-setsize={matches.length}
                aria-posinset={row.index + 1}
                className='h-full min-w-0 flex-1 items-center py-1'
              >
                <div className='flex min-w-0 flex-1 flex-col gap-0.5'>
                  <div className='flex min-w-0 items-center gap-1'>
                    <span className='min-w-0 flex-1 truncate font-medium'>{model.name}</span>
                    {activeVariant && (
                      <span className='shrink-0 rounded bg-muted px-1 py-px text-[9px] leading-none text-muted-foreground'>
                        {activeVariant.name}
                      </span>
                    )}
                  </div>
                  <div className='flex min-w-0 items-center gap-1.5'>
                    {diskBytes !== null && (
                      <ModelDiskMeta diskBytes={diskBytes} downloaded={downloaded} />
                    )}
                    <VramFitBadge
                      candidateVramBytes={measuredVram}
                      otherStagesVramBytes={otherStagesVramBytes}
                      availableVramBytes={availableVramBytes}
                    />
                  </div>
                </div>
              </ComboboxItem>
              {hasMultipleVariants && (
                <Button
                  type='button'
                  variant='ghost'
                  size='icon-xs'
                  tabIndex={-1}
                  aria-label={t('modelPicker.chooseQuantization', { model: model.name })}
                  className='h-7 w-6 shrink-0 rounded-md text-muted-foreground hover:bg-primary/10 hover:text-foreground'
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => onOpenVariants(model.reference)}
                >
                  <ChevronRight className='size-3.5' />
                </Button>
              )}
            </div>
          )
        })}
      </div>
    </ScrollArea>
  )
}

/** Drill-down `MenuPanel` listing all quantizations of a local model family. */
function VariantSubmenu({
  family,
  selectedValue,
  availableVramBytes,
  otherStagesVramBytes,
  disabled,
  onBack,
  onSelect,
}: {
  family: TranslateModelEntry
  selectedValue: string | null
  availableVramBytes: number | null
  otherStagesVramBytes: number
  disabled: boolean
  onBack: () => void
  onSelect: (reference: string) => void
}) {
  const { t } = useTranslation()
  const listRef = useRef<HTMLDivElement>(null)
  const active =
    family.variants.find((variant) => variant.reference === selectedValue) ??
    (selectedValue === family.reference ? family.variants[0] : undefined) ??
    null

  useEffect(() => {
    listRef.current?.focus()
  }, [])

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowLeft') {
      event.preventDefault()
      onBack()
    }
  }

  return (
    <MenuPanel title={family.name} backDisabled={disabled} onBack={onBack}>
      <div className='min-w-0' onKeyDown={handleKeyDown}>
        <Combobox<ModelVariant>
          inline
          open
          items={family.variants}
          value={active}
          readOnly={disabled}
          itemToStringLabel={(variant) => variant.name}
          isItemEqualToValue={(left, right) => left.reference === right.reference}
          onValueChange={(variant) => variant && onSelect(variant.reference)}
        >
          <ComboboxList
            ref={listRef}
            aria-label={t('modelPicker.chooseQuantization', { model: family.name })}
            className='max-h-none overflow-visible p-0 outline-none'
          >
            <ScrollArea
              className='max-h-72 min-w-0 overflow-hidden'
              viewportClassName='h-auto max-h-72 min-w-0 overscroll-contain'
            >
              <div className='grid min-w-0 gap-0.5 py-0.5'>
                {family.variants.map((variant, index) => (
                  <ComboboxItem
                    key={variant.reference}
                    index={index}
                    value={variant}
                    aria-label={variant.name}
                    className='h-auto min-h-10 min-w-0 py-1'
                  >
                    <div className='flex min-w-0 flex-1 flex-col gap-0.5'>
                      <div className='flex min-w-0 items-center gap-1.5'>
                        <span className='min-w-0 flex-1 truncate font-medium'>{variant.name}</span>
                        {index === 0 && (
                          <span className='shrink-0 rounded bg-muted px-1 py-px text-[9px] leading-none text-muted-foreground'>
                            {t('model.default')}
                          </span>
                        )}
                      </div>
                      <div className='flex min-w-0 items-center gap-1.5'>
                        <ModelDiskMeta
                          diskBytes={variant.disk_bytes}
                          downloaded={variant.downloaded}
                        />
                        <VramFitBadge
                          candidateVramBytes={variant.measured_vram_bytes}
                          otherStagesVramBytes={otherStagesVramBytes}
                          availableVramBytes={availableVramBytes}
                        />
                      </div>
                    </div>
                  </ComboboxItem>
                ))}
              </div>
            </ScrollArea>
          </ComboboxList>
        </Combobox>
      </div>
    </MenuPanel>
  )
}

/** Popover selector for non-translation pipeline stages (`detect`, `recognize`, `inpaint`),
 * displaying disk size, download status, and combined VRAM fit when measured. */
export function StageModelSelect({
  value,
  entries,
  fallbackNames,
  availableVramBytes = null,
  otherStagesVramBytes = 0,
  label,
  size = 'default',
  side = 'bottom',
  align = 'start',
  finalFocus,
  className,
  onOpen,
  onChange,
}: {
  value: string
  entries: StageModelEntry[]
  fallbackNames?: Record<string, string>
  availableVramBytes?: number | null
  otherStagesVramBytes?: number
  label: string
  size?: 'default' | 'sm'
  side?: 'top' | 'bottom'
  align?: 'start' | 'center'
  finalFocus?: ComponentProps<typeof PopoverContent>['finalFocus']
  className?: string
  onOpen?: () => void
  onChange: (id: string) => void
}) {
  const [open, setOpen] = useState(false)
  const selected = entries.find((entry) => entry.id === value)
  const displayName = selected?.name ?? fallbackNames?.[value] ?? value
  const compact = size === 'sm'

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (next) onOpen?.()
      }}
    >
      <PopoverTrigger
        type='button'
        aria-label={label}
        className={cn(
          'flex min-w-0 items-center justify-between gap-2 rounded-lg border border-input bg-transparent px-2.5 text-[11px] transition-colors outline-none hover:bg-foreground/[0.03] focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50',
          compact ? 'h-7 w-auto' : 'h-8 w-full',
          className,
        )}
      >
        <span className='flex min-w-0 flex-1 items-center gap-2 text-left'>
          <span className='min-w-0 truncate'>{displayName}</span>
          {!compact && selected && (
            <>
              <ModelDiskMeta diskBytes={selected.disk_bytes} downloaded={selected.downloaded} />
              <VramFitBadge
                candidateVramBytes={selected.measured_vram_bytes}
                otherStagesVramBytes={otherStagesVramBytes}
                availableVramBytes={availableVramBytes}
              />
            </>
          )}
        </span>
        <ChevronDown className='size-3.5 shrink-0 text-muted-foreground' />
      </PopoverTrigger>
      <PopoverContent
        side={side}
        align={align}
        sideOffset={4}
        finalFocus={finalFocus}
        className={cn(
          'gap-0.5 overflow-hidden p-1',
          compact ? 'w-72' : 'w-(--anchor-width) min-w-64',
        )}
      >
        {entries.map((entry) => {
          const isSelected = entry.id === value
          return (
            <button
              key={entry.id}
              type='button'
              aria-pressed={isSelected}
              className='flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] transition-colors outline-none hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent focus-visible:text-accent-foreground'
              onClick={() => {
                setOpen(false)
                if (entry.id !== value) onChange(entry.id)
              }}
            >
              <div className='flex min-w-0 flex-1 flex-col gap-0.5'>
                <span className='min-w-0 truncate font-medium'>{entry.name}</span>
                <div className='flex min-w-0 items-center gap-1.5'>
                  <ModelDiskMeta diskBytes={entry.disk_bytes} downloaded={entry.downloaded} />
                  <VramFitBadge
                    candidateVramBytes={entry.measured_vram_bytes}
                    otherStagesVramBytes={otherStagesVramBytes}
                    availableVramBytes={availableVramBytes}
                  />
                </div>
              </div>
              <Check
                className={cn('size-3.5 shrink-0', isSelected ? 'opacity-100' : 'opacity-0')}
              />
            </button>
          )
        })}
      </PopoverContent>
    </Popover>
  )
}
