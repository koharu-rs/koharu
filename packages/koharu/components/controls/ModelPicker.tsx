'use client'

import { Check, ChevronLeft, LoaderCircle, Search, X } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { modelKey, providerName } from '@/lib/translation'
import type { Model, ModelSelection, ProviderPreference } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from '@koharu/ui/components/input-group'
import { ScrollArea } from '@koharu/ui/components/scroll-area'

export function ModelPicker({
  value,
  models,
  providers,
  loading = false,
  disabled = false,
  busyModel,
  onBack,
  onSelect,
}: {
  value: ModelSelection | null
  models: Model[]
  providers: ProviderPreference[]
  loading?: boolean
  disabled?: boolean
  busyModel?: string | null
  onBack: () => void
  onSelect: (model: Model) => void
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState('')
  const search = useRef<HTMLInputElement>(null)
  const normalizedQuery = query.trim().toLocaleLowerCase()
  const results = useMemo(
    () =>
      models.filter((model) => {
        if (!normalizedQuery) return true
        const provider = providerName(providers, model.provider)
        return [model.name, model.model, provider].some((candidate) =>
          candidate?.toLocaleLowerCase().includes(normalizedQuery),
        )
      }),
    [models, normalizedQuery, providers],
  )

  useEffect(() => {
    search.current?.focus()
  }, [])

  return (
    <div className='min-w-0 overflow-hidden'>
      <div className='mb-1 flex h-7 items-center border-b border-border/60 px-0.5 pb-1'>
        <Button
          type='button'
          variant='ghost'
          size='icon-xs'
          aria-label={t('common.back')}
          className='rounded-md text-muted-foreground hover:bg-primary/10 hover:text-foreground'
          onClick={onBack}
        >
          <ChevronLeft className='size-3.5' />
        </Button>
        <span className='ml-1 text-[11px] font-medium'>{t('modelPicker.title')}</span>
      </div>

      <div className='border-b border-border/60 p-1'>
        <InputGroup className='h-7 border-0 bg-muted/50 shadow-none focus-within:bg-background has-[>[data-align=inline-start]]:[&>input]:pl-0'>
          <InputGroupAddon className='pr-1 pl-1.5'>
            <Search className='size-3' />
          </InputGroupAddon>
          <InputGroupInput
            ref={search}
            value={query}
            aria-label={t('modelPicker.search')}
            placeholder={t('modelPicker.search')}
            className='h-7 px-0 text-[11px]'
            onChange={(event) => setQuery(event.currentTarget.value)}
          />
          {query && (
            <InputGroupAddon align='inline-end' className='pr-0.5'>
              <InputGroupButton aria-label={t('common.clearSearch')} onClick={() => setQuery('')}>
                <X />
              </InputGroupButton>
            </InputGroupAddon>
          )}
        </InputGroup>
      </div>

      <ScrollArea
        className='max-h-64 min-w-0 overflow-hidden'
        viewportClassName='h-auto max-h-64 min-w-0 overscroll-contain'
      >
        <div
          role='listbox'
          aria-label={t('modelPicker.title')}
          aria-expanded={results.length > 0}
          className='grid min-w-0 gap-0.5 py-0.5'
        >
          {results.map((model) => {
            const key = modelKey(model)
            const selected = value ? key === modelKey(value) : false
            return (
              <Button
                key={key}
                type='button'
                variant='ghost'
                aria-label={t('modelPicker.useModel', {
                  model: model.name,
                  provider: providerName(providers, model.provider),
                })}
                aria-pressed={selected}
                disabled={disabled || Boolean(busyModel)}
                className='h-auto min-h-9 w-full max-w-full min-w-0 justify-start gap-2 overflow-hidden rounded-lg px-2 py-1 text-left font-normal hover:bg-primary/10'
                onClick={() => onSelect(model)}
              >
                <span className='min-w-0 flex-1 overflow-hidden'>
                  <span className='block truncate text-[11px] text-foreground'>{model.name}</span>
                  <span className='block truncate text-[9px] text-muted-foreground'>
                    {providerName(providers, model.provider)}
                  </span>
                </span>
                {busyModel === key ? (
                  <LoaderCircle className='size-3.5 shrink-0 animate-spin text-primary' />
                ) : (
                  selected && <Check className='size-3.5 shrink-0 text-primary' />
                )}
              </Button>
            )
          })}
          {loading && models.length === 0 && (
            <div className='flex h-20 items-center justify-center gap-2 text-[11px] text-muted-foreground'>
              <LoaderCircle className='size-3.5 animate-spin' /> {t('modelPicker.loading')}
            </div>
          )}
          {!loading && models.length === 0 && (
            <div className='px-2.5 py-5 text-center text-[11px] text-muted-foreground'>
              {t('modelPicker.empty')}
            </div>
          )}
          {models.length > 0 && results.length === 0 && (
            <div className='px-2.5 py-5 text-center text-[11px] text-muted-foreground'>
              {t('modelPicker.noResults')}
            </div>
          )}
        </div>
      </ScrollArea>
    </div>
  )
}
