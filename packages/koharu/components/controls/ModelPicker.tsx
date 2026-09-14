'use client'

import { Check, ChevronLeft, ChevronRight, HardDrive, LoaderCircle, Search, X } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { modelKey, modelSelection, providerName } from '@/lib/translation'
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
  onSelect: (model: ModelSelection) => void
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState('')
  const [pendingKey, setPendingKey] = useState<string | null>(null)
  const pendingModel = models.find((model) => modelKey(model) === pendingKey)
  const selectedKey = value ? modelKey(value) : null
  const saving = Boolean(busyModel)
  const normalizedQuery = query.trim().toLocaleLowerCase()
  const options = pendingModel
    ? pendingModel.quantizations.map((quantization) => ({ model: pendingModel, quantization }))
    : models
        .filter((model) => {
          if (!normalizedQuery) return true
          return [model.name, model.model, providerName(providers, model.provider)].some(
            (candidate) => candidate?.toLocaleLowerCase().includes(normalizedQuery),
          )
        })
        .map((model) => ({ model, quantization: null }))

  return (
    <div className='min-w-0 overflow-hidden'>
      <div className='mb-1 flex h-7 items-center border-b border-border/60 px-0.5 pb-1'>
        <Button
          type='button'
          variant='ghost'
          size='icon-xs'
          aria-label={t('common.back')}
          className='rounded-md text-muted-foreground hover:bg-primary/10 hover:text-foreground'
          disabled={saving}
          onClick={() => (pendingModel ? setPendingKey(null) : onBack())}
        >
          <ChevronLeft className='size-3.5' />
        </Button>
        <span className='ml-1 min-w-0 flex-1 truncate text-[11px] font-medium'>
          {pendingModel?.name ?? t('modelPicker.title')}
        </span>
        {pendingModel && busyModel === pendingKey && (
          <LoaderCircle className='mr-1 size-3.5 shrink-0 animate-spin text-primary' />
        )}
      </div>

      {pendingModel ? (
        <div className='border-b border-border/60 px-2 py-1.5 text-[11px] text-muted-foreground'>
          {t('settings.translation.selectQuantization')}
        </div>
      ) : (
        <div className='border-b border-border/60 p-1'>
          <InputGroup className='h-7 border-0 bg-muted/50 shadow-none focus-within:bg-background has-[>[data-align=inline-start]]:[&>input]:pl-0'>
            <InputGroupAddon className='pr-1 pl-1.5'>
              <Search className='size-3' />
            </InputGroupAddon>
            <InputGroupInput
              autoFocus
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
      )}

      <ScrollArea
        key={pendingKey}
        className='max-h-64 min-w-0 overflow-hidden'
        viewportClassName='h-auto max-h-64 min-w-0 overscroll-contain'
      >
        <div className='grid min-w-0 gap-0.5 py-0.5'>
          {options.map(({ model, quantization }, index) => {
            const key = modelKey(model)
            const expandable = !quantization && model.quantizations.length > 0
            const downloaded =
              quantization?.downloaded ??
              model.quantizations.some((candidate) => candidate.downloaded)
            const selected =
              key === selectedKey &&
              (!quantization ||
                quantization.id === (value?.quantization ?? model.quantizations[0]?.id))
            const provider = providerName(providers, model.provider)
            return (
              <Button
                key={quantization?.id ?? key}
                autoFocus={Boolean(quantization) && index === 0}
                type='button'
                variant='ghost'
                aria-label={
                  quantization
                    ? undefined
                    : t('modelPicker.useModel', { model: model.name, provider })
                }
                aria-pressed={selected}
                disabled={disabled || saving}
                className='h-auto min-h-9 w-full max-w-full min-w-0 justify-start gap-2 overflow-hidden rounded-lg px-2 py-1 text-left font-normal hover:bg-primary/10'
                onClick={() => {
                  if (expandable) setPendingKey(key)
                  else onSelect(modelSelection(model, quantization?.id ?? null))
                }}
              >
                <span className='min-w-0 flex-1 overflow-hidden'>
                  <span className='block truncate text-[11px] text-foreground'>
                    {quantization?.name ?? model.name}
                  </span>
                  {!quantization && (
                    <span className='block truncate text-[9px] text-muted-foreground'>
                      {provider}
                    </span>
                  )}
                </span>
                {downloaded && <HardDrive className='size-3.5 shrink-0 text-muted-foreground' />}
                {!quantization && busyModel === key ? (
                  <LoaderCircle className='size-3.5 shrink-0 animate-spin text-primary' />
                ) : (
                  selected && <Check className='size-3.5 shrink-0 text-primary' />
                )}
                {expandable && <ChevronRight className='size-3.5 shrink-0 text-muted-foreground' />}
              </Button>
            )
          })}
          {options.length === 0 && (
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
        </div>
      </ScrollArea>
    </div>
  )
}
