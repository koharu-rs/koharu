'use client'

import { Italic } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Choice } from '@/components/controls/Choice'
import { FontPicker } from '@/components/controls/FontPicker'
import { GROUPS, usableFontWeights, type Faces } from '@/lib/styles'
import type { Face, FontFamily, Kernel } from '@koharu/ipc/protocol'
import { Toggle } from '@koharu/ui/components/toggle'
import { cn } from '@koharu/ui/lib/utils'

/** Each kernel's face in `script` that a new project starts with, as Firefox's font settings map
 * each kind of text to a font for one writing system: a line each, its name, its font, whose first
 * entry is Default, antique's, and a weight menu and an italic toggle as the type bar's. The kernels alone in
 * their group come first; each group of several follows under its title, which only labels it. As
 * macOS's settings lists, a menu draws only its value until pointed at, a Default one in grey, and
 * the toggle shows only when on. */
export function KernelFaces({
  script,
  faces,
  families,
  disabled,
  onChange,
}: {
  script: string
  faces: Faces | undefined
  families: FontFamily[]
  disabled?: boolean
  onChange: (kernel: Kernel, face: Face | null) => void
}) {
  const { t } = useTranslation()
  const antique = faces?.antique?.[script]
  const toggle =
    'size-8 min-w-8 shrink-0 px-0 text-muted-foreground aria-pressed:bg-transparent aria-pressed:text-primary aria-pressed:hover:bg-muted'
  const row = (kernel: Kernel) => {
    const shown: Face = faces?.[kernel]?.[script] ?? {}
    const name = t(`styles.kernels.${kernel}`)
    // Antique is what the others take by default, so it has no default of its own.
    const base = kernel === 'antique'
    /** `patch` over what the style sets; one that sets nothing goes. */
    const set = (patch: Face) => {
      const next = { ...shown, ...patch }
      const empty = next.family == null && next.weight == null && next.italic == null
      onChange(kernel, empty ? null : next)
    }
    // The weights its font has, as the type bar lists them, and its own besides.
    const font = (shown.family ?? antique?.family)?.toLowerCase()
    const weights = usableFontWeights(families.find((family) => family.name.toLowerCase() === font))
    if (shown.weight != null && !weights.includes(shown.weight)) weights.push(shown.weight)
    // Keyed `w400` and on, since an object lists number-like keys first, and Default goes first.
    const choices: Record<string, string> = {
      ...(base ? {} : { default: t('styles.defaultWeight') }),
      ...Object.fromEntries(
        weights.sort((a, b) => a - b).map((weight) => [`w${weight}`, String(weight)]),
      ),
    }
    return (
      // Its menus' chevrons a size down from a field's, as they stand alone without outlines.
      <div
        key={kernel}
        className='flex h-12 items-center gap-3 [&_[aria-haspopup]>svg:last-child]:size-3'
      >
        <span className='w-36 shrink-0 truncate text-[12px]'>{name}</span>
        <div className='min-w-0 flex-1'>
          <FontPicker
            value={shown.family ?? ''}
            families={families}
            disabled={disabled}
            variant='ghost'
            ariaLabel={t('styles.font', { name })}
            fallback={
              base
                ? undefined
                : {
                    label: t('styles.default', { name: antique?.family ?? '—' }),
                    onSelect: () => set({ family: null }),
                  }
            }
            onChange={(family) => set({ family })}
          />
        </div>
        {/* Set or left to antique's, as the type bar sets a text's. */}
        <Choice
          className={cn(
            'w-28 shrink-0 border-transparent hover:bg-muted aria-expanded:bg-muted dark:bg-transparent dark:hover:bg-muted/50',
            shown.weight == null && !base && 'text-muted-foreground',
          )}
          label={t('styles.weight', { name })}
          value={shown.weight == null ? (base ? 'w400' : 'default') : `w${shown.weight}`}
          choices={choices}
          disabled={disabled}
          onChange={(next) => set({ weight: next === 'default' ? null : Number(next.slice(1)) })}
        />
        <Toggle
          aria-label={t('styles.italic', { name })}
          className={toggle}
          disabled={disabled}
          pressed={shown.italic === true}
          onPressedChange={(pressed) => set({ italic: pressed ? true : null })}
        >
          <Italic />
        </Toggle>
      </div>
    )
  }
  const rows = 'divide-y divide-border/60'
  return (
    <div>
      <div className={rows}>
        {GROUPS.filter((group) => group.length === 1).map(([kernel]) => row(kernel))}
      </div>
      {GROUPS.filter((group) => group.length > 1).map((group) => (
        <section key={group[0]} className='pt-4'>
          <h4 className='pb-1 text-[10px] font-medium tracking-wide text-muted-foreground uppercase'>
            {/* Only the groups of several have titles. */}
            {t(`styles.groups.${group[0] as 'mincho' | 'brush' | 'design'}`)}
          </h4>
          <div className={rows}>{group.map(row)}</div>
        </section>
      ))}
    </div>
  )
}
