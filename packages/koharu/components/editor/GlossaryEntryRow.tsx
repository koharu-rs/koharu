'use client'

import { Trash2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { glossaryKinds, normalizeGlossaryTranslation } from '@/lib/glossary'
import type { GlossaryEntryPatch, GlossaryEntryView, GlossaryKind } from '@koharu/bridge/protocol'
import { Badge } from '@koharu/ui/components/badge'
import { Button } from '@koharu/ui/components/button'
import { Checkbox } from '@koharu/ui/components/checkbox'
import { Input } from '@koharu/ui/components/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'
import { Switch } from '@koharu/ui/components/switch'

interface GlossaryEntryRowProps {
  entry: GlossaryEntryView
  selected: boolean
  disabled: boolean
  onSelect: (selected: boolean) => void
  onUpdate: (patch: GlossaryEntryPatch) => void
  onDelete: () => void
}

export function GlossaryEntryRow({
  entry,
  selected,
  disabled,
  onSelect,
  onUpdate,
  onDelete,
}: GlossaryEntryRowProps) {
  const { t } = useTranslation()
  const [source, setSource] = useState(entry.source)
  const [translation, setTranslation] = useState(entry.translation ?? '')
  const cancelSourceBlur = useRef(false)
  const cancelTranslationBlur = useRef(false)

  useEffect(() => {
    setSource(entry.source)
    setTranslation(entry.translation ?? '')
  }, [entry.revision, entry.source, entry.translation])

  const patch = (overrides: Partial<GlossaryEntryPatch> = {}): GlossaryEntryPatch => ({
    source,
    translation: normalizeGlossaryTranslation(translation),
    kind: entry.kind,
    enabled: entry.enabled,
    ...overrides,
  })

  const saveSource = () => {
    const value = source.trim()
    if (value && value !== entry.source) onUpdate(patch({ source: value }))
  }
  const saveTranslation = () => {
    const value = normalizeGlossaryTranslation(translation)
    if (value !== entry.translation) onUpdate(patch({ translation: value }))
  }

  return (
    <article className='rounded-lg border border-border/70 bg-background/40 p-2.5'>
      <div className='flex min-w-0 items-center gap-2'>
        <Checkbox
          aria-label={t('glossary.selectEntry', { source: entry.source })}
          checked={selected}
          disabled={disabled}
          onCheckedChange={(checked) => onSelect(checked === true)}
        />
        <span className='min-w-0 flex-1 truncate text-[11px] font-medium'>{entry.source}</span>
        <Badge variant='secondary' className='h-4 px-1.5 text-[9px]'>
          {t(`glossary.kinds.${entry.kind}`)}
        </Badge>
        <Switch
          size='sm'
          aria-label={t('glossary.enableEntry', { source: entry.source })}
          checked={entry.enabled}
          disabled={disabled}
          onCheckedChange={(enabled) => onUpdate(patch({ enabled }))}
        />
        <Button
          size='icon-xs'
          variant='ghost'
          aria-label={t('glossary.deleteEntry', { source: entry.source })}
          disabled={disabled}
          onClick={onDelete}
        >
          <Trash2 className='size-3' />
        </Button>
      </div>
      <div className='mt-2 grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_5.25rem] gap-1.5'>
        <Input
          aria-label={t('glossary.sourceEntry', { source: entry.source })}
          value={source}
          disabled={disabled}
          className='h-7 px-2 text-[11px]'
          onChange={(event) => setSource(event.currentTarget.value)}
          onBlur={() => {
            if (cancelSourceBlur.current) {
              cancelSourceBlur.current = false
              return
            }
            saveSource()
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur()
            if (event.key === 'Escape') {
              event.preventDefault()
              cancelSourceBlur.current = true
              setSource(entry.source)
              event.currentTarget.blur()
            }
          }}
        />
        <Input
          aria-label={t('glossary.translationEntry', { source: entry.source })}
          value={translation}
          disabled={disabled}
          placeholder={t('glossary.untranslated')}
          className='h-7 px-2 text-[11px]'
          onChange={(event) => setTranslation(event.currentTarget.value)}
          onBlur={() => {
            if (cancelTranslationBlur.current) {
              cancelTranslationBlur.current = false
              return
            }
            saveTranslation()
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur()
            if (event.key === 'Escape') {
              event.preventDefault()
              cancelTranslationBlur.current = true
              setTranslation(entry.translation ?? '')
              event.currentTarget.blur()
            }
          }}
        />
        <Select
          value={entry.kind}
          items={Object.fromEntries(
            glossaryKinds.map((kind) => [kind, t(`glossary.kinds.${kind}`)]),
          )}
          disabled={disabled}
          onValueChange={(kind) => kind && onUpdate(patch({ kind: kind as GlossaryKind }))}
        >
          <SelectTrigger
            size='sm'
            className='h-7 w-full'
            aria-label={t('glossary.categoryEntry', { source: entry.source })}
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent align='end'>
            {glossaryKinds.map((kind) => (
              <SelectItem key={kind} value={kind}>
                {t(`glossary.kinds.${kind}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className='mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[9px] text-muted-foreground'>
        {entry.confidence !== null ? (
          <span>{t('glossary.confidence', { value: Math.round(entry.confidence * 100) })}</span>
        ) : null}
        <span>{t('glossary.occurrences', { count: entry.occurrenceCount })}</span>
        <span>{t(`glossary.origins.${entry.sourceOrigin}`)}</span>
        {entry.translationOrigin ? (
          <span>{t(`glossary.origins.${entry.translationOrigin}`)}</span>
        ) : null}
        {!entry.presentInLastScan ? (
          <span className='font-medium text-amber-600 dark:text-amber-400'>
            {t('glossary.notPresent')}
          </span>
        ) : null}
      </div>
      {entry.examples.length > 0 ? (
        <details className='mt-1.5 text-[9px] text-muted-foreground'>
          <summary className='cursor-pointer select-none'>{t('glossary.examples')}</summary>
          <ul className='mt-1 space-y-0.5 border-l pl-2'>
            {entry.examples.map((example, index) => (
              <li key={`${entry.id}-${index}`}>{example}</li>
            ))}
          </ul>
        </details>
      ) : null}
    </article>
  )
}
