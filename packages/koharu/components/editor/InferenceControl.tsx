'use client'

import { Bot, Check, ChevronDown, ChevronRight, Play, Settings } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { MenuPanel } from '@/components/controls/MenuPanel'
import { ModelPicker } from '@/components/controls/ModelPicker'
import { OutputPicker } from '@/components/controls/OutputPicker'
import {
  computeOtherStagesVram,
  findCatalogSelection,
  formatModelName,
  vendorOf,
} from '@/lib/models'
import { useModelCatalog, usePreferences, useSavePreferences } from '@/lib/queries'
import { isActiveJob, pipelineStages, useKoharuStore, type PipelineScope } from '@/lib/store'
import { languageName } from '@/lib/translation'
import type {
  Preferences,
  RunPriority,
  Stage,
  TranslateModelEntry,
  TranslatorConfig,
} from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { ButtonGroup } from '@koharu/ui/components/button-group'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@koharu/ui/components/dropdown-menu'
import { Popover, PopoverContent, PopoverTrigger } from '@koharu/ui/components/popover'

type SelectorView = 'root' | 'model' | 'scope' | 'stages' | 'output'

export function InferenceControl({
  onRun,
  disabled,
}: {
  onRun: (scope: PipelineScope, stages: Stage[], priority: RunPriority) => void
  disabled: boolean
}) {
  const { t } = useTranslation()
  const scope = useKoharuStore((state) => state.processingScope)
  const stages = useKoharuStore((state) => state.processingStages)
  const setScope = useKoharuStore((state) => state.setProcessingScope)
  const setStages = useKoharuStore((state) => state.setProcessingStages)
  const jobs = useKoharuStore((state) => state.jobs)
  const selectedPages = useKoharuStore((state) => state.selectedPages)
  const hasActiveWork = Object.values(jobs).some(isActiveJob)
  const unavailable = scope === 'selected-pages' && selectedPages.length === 0

  return (
    <div className='flex items-center gap-1'>
      <RuntimeSelector
        scope={scope}
        stages={stages}
        selectionCount={selectedPages.length}
        onScopeChange={setScope}
        onStagesChange={setStages}
      />

      {hasActiveWork ? (
        <ButtonGroup>
          <Button
            type='button'
            size='sm'
            className='bg-primary/80 px-2.5 text-[11px] hover:bg-primary/90'
            disabled={disabled || unavailable}
            aria-label={t('inference.queueProcessing')}
            onClick={() => onRun(scope, stages, 'normal')}
          >
            <Play className='size-3' />
            <span>{t('inference.queue')}</span>
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button
                  type='button'
                  size='sm'
                  className='bg-primary/80 px-1.5 hover:bg-primary/90'
                  disabled={disabled || unavailable}
                  aria-label={t('inference.runOptions')}
                />
              }
            >
              <ChevronDown className='size-3' />
            </DropdownMenuTrigger>
            <DropdownMenuContent align='end' className='w-56'>
              <DropdownMenuItem onClick={() => onRun(scope, stages, 'normal')}>
                <div className='flex flex-col gap-0.5'>
                  <span className='text-[11px] font-medium'>{t('inference.queue')}</span>
                  <span className='text-[10px] text-muted-foreground'>
                    {t('inference.queueDescription')}
                  </span>
                </div>
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => onRun(scope, stages, 'rerun_now')}>
                <div className='flex flex-col gap-0.5'>
                  <span className='text-[11px] font-medium'>{t('inference.rerunNow')}</span>
                  <span className='text-[10px] text-muted-foreground'>
                    {t('inference.rerunNowDescription')}
                  </span>
                </div>
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </ButtonGroup>
      ) : (
        <Button
          type='button'
          size='sm'
          className='rounded-lg bg-primary/80 px-2.5 text-[11px] hover:bg-primary/90'
          disabled={disabled || unavailable}
          aria-label={t('inference.runProcessing')}
          onClick={() => onRun(scope, stages, 'normal')}
        >
          <Play className='size-3' />
          <span>{t('inference.run')}</span>
        </Button>
      )}
    </div>
  )
}

function RuntimeSelector({
  scope,
  stages,
  selectionCount,
  onScopeChange,
  onStagesChange,
}: {
  scope: PipelineScope
  stages: Stage[]
  selectionCount: number
  onScopeChange: (scope: PipelineScope) => void
  onStagesChange: (stages: Stage[]) => void
}) {
  const { t, i18n } = useTranslation()
  const [open, setOpen] = useState(false)
  const [view, setView] = useState<SelectorView>('root')
  const preferences = usePreferences().data
  const catalog = useModelCatalog()
  const saveModel = useSavePreferences()
  const saveOutput = useSavePreferences()
  const setSettingsOpen = useKoharuStore((state) => state.setSettingsOpen)
  const translator = preferences?.pipeline.translator ?? null
  const model = translator?.model || null
  const choices = availableModels(model, catalog.data?.translate ?? [])
  const modelLabel = model ? formatModelName(choices, model, t) : t('inference.noModel')
  const otherStagesVramBytes = computeOtherStagesVram(
    catalog.data,
    preferences?.pipeline,
    'translate',
  )
  const target = translator?.target
  const outputLabel = target ? languageName(target, i18n.language) : t('inference.notSet')

  /** The preferences with the translator's section changed. */
  const withTranslator = (preferences: Preferences, change: Partial<TranslatorConfig>) => ({
    ...preferences,
    pipeline: {
      ...preferences.pipeline,
      translator: { ...preferences.pipeline.translator, ...change },
    },
  })

  const handleOpenChange = (next: boolean) => {
    setOpen(next)
    setView('root')
    if (next) void catalog.refetch()
  }

  const chooseModel = (next: string) => {
    if (!preferences || saveModel.isPending) return
    if (next === model) {
      setView('root')
      return
    }
    saveModel.mutate(withTranslator(preferences, { model: next }))
  }

  const chooseScope = (next: PipelineScope) => {
    onScopeChange(next)
    setView('root')
  }

  const chooseOutput = (next: string) => {
    if (!preferences || saveOutput.isPending) return
    saveOutput.mutate(withTranslator(preferences, { target: next }))
  }

  const toggleStage = (stage: Stage) => {
    if (stages.includes(stage)) {
      if (stages.length > 1) onStagesChange(stages.filter((candidate) => candidate !== stage))
      return
    }
    onStagesChange(
      pipelineStages.filter((candidate) => candidate === stage || stages.includes(candidate)),
    )
  }

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger
        type='button'
        aria-label={t('inference.selector')}
        className='flex h-7 max-w-52 items-center gap-1.5 rounded-lg bg-foreground/[0.05] px-2 text-[10px] text-muted-foreground transition-colors outline-none hover:bg-primary/10 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/25 data-open:bg-primary/10 data-open:text-foreground'
      >
        <Bot className='size-3 shrink-0 text-primary' />
        <span className='min-w-0 truncate'>{modelLabel}</span>
        <ChevronDown className='size-3 shrink-0' />
      </PopoverTrigger>

      <PopoverContent align='start' sideOffset={4} className='w-72 gap-0 overflow-hidden p-1'>
        {view === 'root' && (
          <div className='grid gap-0.5' aria-label={t('inference.shortcuts')}>
            <SelectorRow
              label={t('inference.model')}
              value={modelLabel}
              onClick={() => setView('model')}
            />
            <SelectorRow
              label={t('inference.scope')}
              value={t(`inference.scopeShort.${scope}`)}
              onClick={() => setView('scope')}
            />
            <SelectorRow
              label={t('inference.stages')}
              value={
                stages.length === 1
                  ? t(`phase.${stages[0]}`)
                  : t('inference.stageCount', { count: stages.length })
              }
              onClick={() => setView('stages')}
            />
            <SelectorRow
              label={t('inference.output')}
              value={outputLabel}
              onClick={() => setView('output')}
            />
            <div className='my-1 border-t border-border/70' />
            <Button
              type='button'
              variant='ghost'
              size='sm'
              className='h-7 justify-start gap-2 rounded-md px-2 text-[11px] font-normal text-muted-foreground hover:bg-primary/10 hover:text-foreground'
              onClick={() => {
                setOpen(false)
                setSettingsOpen(true)
              }}
            >
              <Settings className='size-3.5' /> {t('menu.settings')}
            </Button>
          </div>
        )}

        {view === 'model' && (
          <ModelPicker
            value={model}
            models={choices}
            availableVramBytes={catalog.data?.available_vram_bytes ?? null}
            otherStagesVramBytes={otherStagesVramBytes}
            loading={catalog.isFetching}
            disabled={saveModel.isPending}
            onBack={() => setView('root')}
            onSelect={chooseModel}
          />
        )}

        {view === 'scope' && (
          <MenuPanel title={t('inference.scope')} onBack={() => setView('root')}>
            <div className='grid min-w-0 gap-0.5'>
              <SelectorOption
                value='selected-pages'
                label={t('inference.selectedPages')}
                detail={
                  selectionCount
                    ? t('inference.selectedCount', { count: selectionCount })
                    : t('inference.selectPagesFirst')
                }
                selected={scope === 'selected-pages'}
                disabled={selectionCount === 0}
                onSelect={chooseScope}
              />
              <SelectorOption
                value='project'
                label={t('inference.entireProject')}
                detail={t('inference.entireProjectDescription')}
                selected={scope === 'project'}
                onSelect={chooseScope}
              />
            </div>
          </MenuPanel>
        )}

        {view === 'stages' && (
          <MenuPanel title={t('inference.pipelineStages')} onBack={() => setView('root')}>
            <div className='grid min-w-0 gap-0.5'>
              {pipelineStages.map((stage) => (
                <SelectorOption
                  key={stage}
                  value={stage}
                  label={t(`phase.${stage}`)}
                  detail={t(`phaseDescription.${stage}`)}
                  selected={stages.includes(stage)}
                  onSelect={toggleStage}
                />
              ))}
            </div>
          </MenuPanel>
        )}

        {view === 'output' && translator && (
          <OutputPicker
            targetLanguage={translator.target ?? 'en'}
            saving={saveOutput.isPending}
            onBack={() => setView('root')}
            onChange={chooseOutput}
          />
        )}
      </PopoverContent>
    </Popover>
  )
}

function SelectorRow({
  label,
  value,
  onClick,
}: {
  label: string
  value: string
  onClick: () => void
}) {
  return (
    <Button
      type='button'
      variant='ghost'
      size='sm'
      aria-label={`${label} ${value}`}
      className='h-7 min-w-0 justify-start gap-3 overflow-hidden rounded-md px-2 text-[11px] font-normal hover:bg-primary/10'
      onClick={onClick}
    >
      <span className='shrink-0'>{label}</span>
      <span className='ml-auto min-w-0 flex-1 truncate text-right text-muted-foreground'>
        {value}
      </span>
      <ChevronRight className='size-3.5 shrink-0 text-muted-foreground' />
    </Button>
  )
}

function SelectorOption<Value extends string>({
  value,
  label,
  detail,
  selected,
  disabled = false,
  onSelect,
}: {
  value: Value
  label: string
  detail: string
  selected: boolean
  disabled?: boolean
  onSelect: (value: Value) => void
}) {
  return (
    <Button
      type='button'
      variant='ghost'
      aria-pressed={selected}
      disabled={disabled}
      className='h-auto min-h-9 justify-start gap-2 rounded-lg px-2 py-1 text-left font-normal hover:bg-primary/10'
      onClick={() => onSelect(value)}
    >
      <span className='min-w-0 flex-1'>
        <span className='block text-[11px]'>{label}</span>
        <span className='block text-[9px] text-muted-foreground'>{detail}</span>
      </span>
      {selected && <Check className='size-3.5 shrink-0 text-primary' />}
    </Button>
  )
}

/** The models to pick from, with the one the preferences hold first when the list lacks it. */
function availableModels(
  selected: string | null,
  models: TranslateModelEntry[],
): TranslateModelEntry[] {
  if (!selected || findCatalogSelection(models, selected)) return models
  return [
    {
      reference: selected,
      name: selected,
      vendor: vendorOf(selected),
      images: false,
      tools: false,
      disk_bytes: null,
      downloaded: false,
      measured_vram_bytes: null,
      variants: [],
    },
    ...models,
  ]
}
