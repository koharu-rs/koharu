'use client'

import {
  closestCenter,
  DndContext,
  PointerSensor,
  useDroppable,
  useSensor,
  useSensors,
} from '@dnd-kit/core'
import {
  arrayMove,
  arraySwap,
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable'
import { CSS } from '@dnd-kit/utilities'
import { GripVertical, X } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { FontPicker } from '@/components/controls/FontPicker'
import {
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { useFonts } from '@/lib/queries'
import type { FontFamily, RendererConfig } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { cn } from '@koharu/ui/lib/utils'

/** The preferred fonts as the renderer follows them, in the settings' rows: the primary, then the
 * fallbacks it tries in order for characters the primary lacks, each dragged by a grip in the
 * gutter to move it, or onto the primary to trade places with it. */
export function TypesettingPreferences({
  value,
  onChange,
}: {
  value: RendererConfig
  onChange: (value: RendererConfig) => void
}) {
  const { t } = useTranslation()
  const fonts = useFonts()
  const families = fonts.data ?? []
  const fontFamilies = useMemo(() => value.font_families ?? [], [value.font_families])
  const ids = useMemo(() => fontFamilies.map(normalizeFontName), [fontFamilies])
  const [primary, ...fallbacks] = fontFamilies
  /** The families a font of the stack can become: its own, or any the stack doesn't hold. */
  const choicesFor = (family: string) =>
    families.filter(
      (candidate) =>
        normalizeFontName(candidate.name) === normalizeFontName(family) ||
        !ids.includes(normalizeFontName(candidate.name)),
    )
  // Only a grip that moves drags a fallback.
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }))

  const save = (next: string[]) => onChange({ ...value, font_families: next })
  const remove = (index: number) => save(fontFamilies.filter((_, other) => other !== index))
  const loading = fonts.isPending ? t('settings.typesetting.loadingFonts') : undefined

  return (
    <PreferencePage
      title={t('settings.typesetting.title')}
      description={t('settings.typesetting.description')}
    >
      {/* One drag across both rows: a fallback dropped among the fallbacks moves there, and one
          dropped on the primary trades places with it. Around the section, not in it, as its
          hidden announcements would join the section's divided rows. */}
      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        onDragEnd={({ active, over }) => {
          if (!over || active.id === over.id) return
          const from = ids.indexOf(String(active.id))
          const to = ids.indexOf(String(over.id))
          save(to === 0 ? arraySwap(fontFamilies, from, 0) : arrayMove(fontFamilies, from, to))
        }}
      >
        <PreferenceSection
          title={t('settings.typesetting.fontFallback')}
          description={t('settings.typesetting.fontFallbackDescription')}
        >
          <PreferenceRow
            title={t('settings.typesetting.primary')}
            description={primary === undefined ? t('settings.typesetting.empty') : undefined}
          >
            <Primary id={ids[0]}>
              <FontSlot
                value={primary ?? ''}
                families={choicesFor(primary ?? '')}
                disabled={fonts.isPending}
                label={t('settings.typesetting.primary')}
                placeholder={loading ?? t('settings.typesetting.addFamily')}
                onChange={(family) =>
                  save(primary === undefined ? [family] : fontFamilies.with(0, family))
                }
                onRemove={primary === undefined ? undefined : () => remove(0)}
              />
            </Primary>
          </PreferenceRow>
          {primary !== undefined && (
            <PreferenceRow
              title={t('settings.typesetting.fallbacks')}
              description={t('settings.typesetting.fallbacksDescription')}
              align='start'
            >
              <div className='grid gap-1.5'>
                <SortableContext items={ids.slice(1)} strategy={verticalListSortingStrategy}>
                  {fallbacks.map((family, index) => (
                    <Fallback
                      key={ids[index + 1]}
                      id={ids[index + 1] ?? family}
                      family={family}
                      families={choicesFor(family)}
                      label={t('settings.typesetting.familyLabel', { index: index + 2 })}
                      onChange={(next) => save(fontFamilies.with(index + 1, next))}
                      onRemove={() => remove(index + 1)}
                    />
                  ))}
                </SortableContext>
                <FontSlot
                  value=''
                  families={choicesFor('')}
                  disabled={fonts.isPending || choicesFor('').length === 0}
                  label={t('settings.typesetting.addFallback')}
                  placeholder={loading ?? t('settings.typesetting.addFallback')}
                  onChange={(family) => save([...fontFamilies, family])}
                />
              </div>
            </PreferenceRow>
          )}
        </PreferenceSection>
      </DndContext>
      {fonts.isError && (
        <p role='status' className='-mt-6 text-[10px] leading-4 text-destructive'>
          {t('settings.typesetting.loadError')}
        </p>
      )}
    </PreferencePage>
  )
}

/** The primary's select, which a fallback is dropped on to trade places with it, outlined while
 * one is over it. */
function Primary({ id, children }: { id: string | undefined; children: React.ReactNode }) {
  const { setNodeRef, isOver } = useDroppable({ id: id ?? 'primary', disabled: id === undefined })
  return (
    <div
      ref={setNodeRef}
      className={cn(
        'rounded-lg',
        isOver && 'ring-2 ring-ring/50 ring-offset-2 ring-offset-[var(--surface-panel)]',
      )}
    >
      {children}
    </div>
  )
}

/** A fallback, dragged by a grip that shows in the gutter beside it on hover. */
function Fallback({
  id,
  family,
  families,
  label,
  onChange,
  onRemove,
}: {
  id: string
  family: string
  families: FontFamily[]
  label: string
  onChange: (family: string) => void
  onRemove: () => void
}) {
  const { setNodeRef, setActivatorNodeRef, listeners, transform, transition, isDragging } =
    useSortable({ id })
  return (
    <div
      ref={setNodeRef}
      // Along the list only.
      style={{ transform: CSS.Translate.toString(transform && { ...transform, x: 0 }), transition }}
      className={isDragging ? 'group relative z-10' : 'group relative'}
    >
      <span
        ref={setActivatorNodeRef}
        {...listeners}
        aria-hidden='true'
        className='absolute top-0 -left-6 grid h-8 w-5 cursor-grab place-items-center text-muted-foreground opacity-0 group-hover:opacity-100 active:cursor-grabbing'
      >
        <GripVertical className='size-3.5' />
      </span>
      <FontSlot
        value={family}
        families={families}
        label={label}
        onChange={onChange}
        onRemove={onRemove}
      />
    </div>
  )
}

/** A font select as the settings' other selects are, with a button that removes its font, or the
 * button's room kept, so the selects of the section line up. */
function FontSlot({
  value,
  families,
  disabled,
  label,
  placeholder,
  onChange,
  onRemove,
}: {
  value: string
  families: FontFamily[]
  disabled?: boolean
  label: string
  placeholder?: string
  onChange: (family: string) => void
  onRemove?: () => void
}) {
  const { t } = useTranslation()
  return (
    <div className='flex items-center gap-1.5'>
      <div className='min-w-0 flex-1'>
        <FontPicker
          value={value}
          families={families}
          disabled={disabled}
          ariaLabel={label}
          placeholder={placeholder}
          onChange={onChange}
        />
      </div>
      {onRemove ? (
        <Button
          type='button'
          variant='ghost'
          size='icon-sm'
          aria-label={t('settings.typesetting.removeFamily', { family: value })}
          className='shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive'
          onClick={onRemove}
        >
          <X className='size-3.5' />
        </Button>
      ) : (
        <span aria-hidden='true' className='size-7 shrink-0' />
      )}
    </div>
  )
}

function normalizeFontName(value: string): string {
  return value.normalize('NFKC').trim().replace(/\s+/g, ' ').toLowerCase()
}
