'use client'

import {
  Bot,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Play,
  Settings,
  Sparkles,
  Square,
} from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { ModelPicker } from '@/components/controls/ModelPicker'
import { OutputPicker } from '@/components/controls/OutputPicker'
import { call, refreshModels, savePreferences } from '@/lib/backend'
import { pipelineStages, useKoharuStore, type PipelineScope } from '@/lib/store'
import { modelName, vendorOf } from '@/lib/translation'
import { commands, type ModelInfo, type Stage, type TranslatorConfig } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import { Popover, PopoverContent, PopoverTrigger } from '@koharu/ui/components/popover'
import { cn } from '@koharu/ui/lib/utils'

type SelectorView = 'root' | 'model' | 'scope' | 'stages' | 'output'

export function InferenceControl({
  onRun,
  disabled,
}: {
  onRun: (scope: PipelineScope, stages: Stage[]) => void
  disabled: boolean
}) {
  const { t } = useTranslation()
  const scope = useKoharuStore((state) => state.processingScope)
  const stages = useKoharuStore((state) => state.processingStages)
  const setScope = useKoharuStore((state) => state.setProcessingScope)
  const setStages = useKoharuStore((state) => state.setProcessingStages)
  const jobs = useKoharuStore((state) => state.jobs)
  const selectedPages = useKoharuStore((state) => state.selectedPages)
  const running = Object.values(jobs).find((job) => job.state === 'running') ?? null
  const unavailable = scope === 'selected-pages' && selectedPages.length === 0

  const stop = () => void call(commands.stop).catch(() => undefined)

  return (
    <div className='flex items-center gap-1'>
      <RuntimeSelector
        scope={scope}
        stages={stages}
        selectionCount={selectedPages.length}
        running={Boolean(running)}
        onScopeChange={setScope}
        onStagesChange={setStages}
      />

      <Button
        type='button'
        size='sm'
        className={cn(
          'rounded-lg px-2.5 text-[11px]',
          !running && 'bg-primary/80 hover:bg-primary/90',
        )}
        disabled={(disabled || unavailable) && !running}
        aria-label={running ? t('inference.stopProcessing') : t('inference.runProcessing')}
        onClick={running ? stop : () => onRun(scope, stages)}
      >
        {running ? <Square className='size-3 fill-current' /> : <Play className='size-3' />}
        <span>{running ? t('inference.stop') : t('inference.run')}</span>
      </Button>
    </div>
  )
}

function RuntimeSelector({
  scope,
  stages,
  selectionCount,
  running,
  onScopeChange,
  onStagesChange,
}: {
  scope: PipelineScope
  stages: Stage[]
  selectionCount: number
  running: boolean
  onScopeChange: (scope: PipelineScope) => void
  onStagesChange: (stages: Stage[]) => void
}) {
  const { t, i18n } = useTranslation()
  const [open, setOpen] = useState(false)
  const [view, setView] = useState<SelectorView>('root')
  const [loadingModels, setLoadingModels] = useState(false)
  const [savingModel, setSavingModel] = useState<string | null>(null)
  const [savingOutput, setSavingOutput] = useState(false)
  const preferences = useKoharuStore((state) => state.preferences)
  const models = useKoharuStore((state) => state.models)
  const setSettingsOpen = useKoharuStore((state) => state.setSettingsOpen)
  const translator = preferences?.pipeline.translator ?? null
  const model = translator?.model || null
  const choices = availableModels(model, models)
  const modelLabel = model ? modelName(choices, model) : t('inference.noModel')
  const target = translator?.target
  const outputLabel = target
    ? (new Intl.DisplayNames([i18n.language], { type: 'language' }).of(target) ?? target)
    : t('inference.notSet')

  /** Saves the translator's section with `change` applied. */
  const saveTranslator = (change: Partial<TranslatorConfig>) => {
    if (!preferences) return Promise.resolve()
    return savePreferences({
      ...preferences,
      pipeline: {
        ...preferences.pipeline,
        translator: { ...preferences.pipeline.translator, ...change },
      },
    })
      .then(() => undefined)
      .catch(() => undefined)
  }

  const handleOpenChange = (next: boolean) => {
    setOpen(next)
    setView('root')
    if (!next) return
    setLoadingModels(true)
    void refreshModels(true)
      .catch(() => undefined)
      .finally(() => setLoadingModels(false))
  }

  const chooseModel = (next: string) => {
    if (!preferences || savingModel) return
    if (next === model) {
      setView('root')
      return
    }
    setSavingModel(next)
    void saveTranslator({ model: next }).finally(() => setSavingModel(null))
  }

  const chooseScope = (next: PipelineScope) => {
    onScopeChange(next)
    setView('root')
  }

  const saveOutput = (next: string) => {
    if (!preferences || savingOutput) return
    setSavingOutput(true)
    void saveTranslator({ target: next }).finally(() => setSavingOutput(false))
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
        className={cn(
          'flex h-7 max-w-52 items-center gap-1.5 rounded-lg bg-foreground/[0.05] px-2 text-[10px] text-muted-foreground transition-colors outline-none hover:bg-primary/10 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/25 data-open:bg-primary/10 data-open:text-foreground',
          running && 'text-foreground',
        )}
      >
        {running ? (
          <Sparkles className='size-3 shrink-0 text-primary' />
        ) : (
          <Bot className='size-3 shrink-0 text-primary' />
        )}
        <span className='min-w-0 truncate'>{modelLabel}</span>
        <ChevronDown className='size-3 shrink-0' />
      </PopoverTrigger>

      <PopoverContent
        align='start'
        sideOffset={4}
        className='w-64 min-w-0 gap-0 overflow-hidden rounded-xl border border-border/50 p-1 shadow-sm ring-0'
      >
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
              className='h-8 justify-start gap-2 rounded-lg px-2 text-[11px] font-normal text-muted-foreground hover:bg-primary/10 hover:text-foreground'
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
            loading={loadingModels}
            disabled={running}
            busyModel={savingModel}
            onBack={() => setView('root')}
            onSelect={chooseModel}
          />
        )}

        {view === 'scope' && (
          <SelectorPanel title={t('inference.scope')} onBack={() => setView('root')}>
            <SelectorOption
              value='page'
              label={t('inference.currentPage')}
              detail={t('inference.currentPageDescription')}
              selected={scope === 'page'}
              onSelect={chooseScope}
            />
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
          </SelectorPanel>
        )}

        {view === 'stages' && (
          <SelectorPanel title={t('inference.pipelineStages')} onBack={() => setView('root')}>
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
          </SelectorPanel>
        )}

        {view === 'output' && translator && (
          <OutputPicker
            targetLanguage={translator.target ?? 'en'}
            disabled={running}
            saving={savingOutput}
            onBack={() => setView('root')}
            onChange={saveOutput}
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
      className='h-8 min-w-0 justify-start gap-3 overflow-hidden rounded-lg px-2 text-[11px] font-normal hover:bg-primary/10'
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

function SelectorPanel({
  title,
  onBack,
  children,
}: {
  title: string
  onBack: () => void
  children: ReactNode
}) {
  const { t } = useTranslation()
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
        <span className='ml-1 text-[11px] font-medium'>{title}</span>
      </div>
      <div className='grid min-w-0 gap-0.5 overflow-hidden'>{children}</div>
    </div>
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
function availableModels(selected: string | null, models: ModelInfo[]): ModelInfo[] {
  if (!selected || models.some((model) => model.reference === selected)) return models
  return [
    { reference: selected, name: selected, vendor: vendorOf(selected), images: false, tools: false },
    ...models,
  ]
}
