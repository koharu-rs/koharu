'use client'

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { useKoharuStore } from '@/lib/store'
import { commands, type WorkflowPreset, type WorkflowStep } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@koharu/ui/components/dialog'
import { Input } from '@koharu/ui/components/input'

const steps: WorkflowStep[] = ['detection', 'ocr', 'terminology', 'translation', 'inpainting']
const normalizePreset = (preset: WorkflowPreset): Required<WorkflowPreset> => ({
  name: 'Standard',
  scheduling: 'page_major',
  scope: 'project',
  stages: ['detection', 'ocr', 'translation', 'inpainting'],
  review_glossary: true,
  ...preset,
})

export function WorkflowDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const selectedPages = useKoharuStore((state) => state.selectedPages)
  const [presets, setPresets] = useState<Required<WorkflowPreset>[]>([])
  const [draft, setDraft] = useState<Required<WorkflowPreset> | null>(null)

  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (!open) return
    let active = true
    void call(commands.getWorkflowPresets)
      .then((presets) => {
        if (!active) return
        setPresets(presets.map(normalizePreset))
        setDraft(normalizePreset(presets[Math.min(1, presets.length - 1)] ?? {}))
      })
      .catch(() => undefined)
    return () => {
      active = false
    }
  }, [open])
  const perform = (action: () => Promise<unknown>) => {
    setBusy(true)
    void action()
      .catch(() => undefined)
      .finally(() => setBusy(false))
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className='max-w-md'>
        <DialogHeader>
          <DialogTitle>{t('workflow.title')}</DialogTitle>
          <DialogDescription>{t('workflow.description')}</DialogDescription>
        </DialogHeader>
        {draft && (
          <fieldset disabled={busy} className='grid gap-4 text-xs'>
            <label className='grid gap-1'>
              {t('workflow.preset')}
              <select
                aria-label={t('workflow.preset')}
                className='rounded border bg-background p-2'
                value={presets.some((preset) => preset.name === draft.name) ? draft.name : ''}
                onChange={(event) =>
                  setDraft(presets.find((preset) => preset.name === event.target.value) ?? draft)
                }
              >
                <option value='' disabled>
                  {t('workflow.custom')}
                </option>
                {presets.map((preset) => (
                  <option key={preset.name} value={preset.name}>
                    {preset.name}
                  </option>
                ))}
              </select>
            </label>
            <label className='grid gap-1'>
              {t('workflow.name')}
              <Input
                aria-label={t('workflow.name')}
                value={draft.name}
                onChange={(event) => setDraft({ ...draft, name: event.target.value })}
              />
            </label>
            <label className='grid gap-1'>
              {t('workflow.scheduling')}
              <select
                aria-label={t('workflow.scheduling')}
                className='rounded border bg-background p-2'
                value={draft.scheduling}
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    scheduling: event.target.value as Required<WorkflowPreset>['scheduling'],
                    stages:
                      event.target.value === 'page_major'
                        ? draft.stages.filter((stage) => stage !== 'terminology')
                        : draft.stages,
                  })
                }
              >
                <option value='page_major'>{t('workflow.pageMajor')}</option>
                <option value='stage_major'>{t('workflow.stageMajor')}</option>
              </select>
            </label>
            <label className='grid gap-1'>
              {t('workflow.scope')}
              <select
                aria-label={t('workflow.scope')}
                className='rounded border bg-background p-2'
                value={draft.scope}
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    scope: event.target.value as Required<WorkflowPreset>['scope'],
                  })
                }
              >
                <option value='project'>{t('workflow.project')}</option>
                <option value='selected_pages'>
                  {t('workflow.selected', { count: selectedPages.length })}
                </option>
              </select>
            </label>
            <div className='flex flex-wrap gap-3'>
              {steps.map((stage) => (
                <label key={stage} className='flex items-center gap-1'>
                  <input
                    type='checkbox'
                    checked={draft.stages.includes(stage)}
                    disabled={stage === 'terminology' && draft.scheduling !== 'stage_major'}
                    onChange={(event) =>
                      setDraft({
                        ...draft,
                        stages: steps.filter((value) =>
                          value === stage ? event.target.checked : draft.stages.includes(value),
                        ),
                      })
                    }
                  />
                  {t(`phase.${stage}`)}
                </label>
              ))}
            </div>
            {draft.stages.includes('terminology') && (
              <label className='flex items-start gap-2'>
                <input
                  type='checkbox'
                  checked={draft.review_glossary}
                  onChange={(event) =>
                    setDraft({ ...draft, review_glossary: event.target.checked })
                  }
                />
                <span>
                  {t('workflow.review')}
                  <span className='mt-1 block text-muted-foreground'>
                    {t('workflow.skipReview')}
                  </span>
                </span>
              </label>
            )}
            <div className='flex flex-wrap gap-2'>
              <Button
                variant='outline'
                disabled={!draft.name.trim()}
                onClick={() =>
                  perform(async () => {
                    const next = presets.some((preset) => preset.name === draft.name)
                      ? presets.map((preset) => (preset.name === draft.name ? draft : preset))
                      : [...presets, draft]
                    setPresets(
                      (await call(commands.saveWorkflowPresets, next)).map(normalizePreset),
                    )
                  })
                }
              >
                {t('workflow.savePreset')}
              </Button>
              <Button
                variant='ghost'
                disabled={
                  presets.length <= 1 || !presets.some((preset) => preset.name === draft.name)
                }
                onClick={() =>
                  perform(async () => {
                    const saved = await call(
                      commands.saveWorkflowPresets,
                      presets.filter((preset) => preset.name !== draft.name),
                    )
                    setPresets(saved.map(normalizePreset))
                    setDraft(normalizePreset(saved[0]))
                  })
                }
              >
                {t('workflow.deletePreset')}
              </Button>
              <Button
                className='ml-auto'
                disabled={
                  !draft.stages.length ||
                  (draft.scope === 'selected_pages' && !selectedPages.length)
                }
                onClick={() =>
                  perform(async () => {
                    await call(
                      commands.startWorkflow,
                      draft.scope === 'project'
                        ? { scope: 'project' }
                        : { scope: 'pages', value: selectedPages },
                      draft,
                    )
                    onOpenChange(false)
                  })
                }
              >
                {t('workflow.start')}
              </Button>
            </div>
          </fieldset>
        )}
      </DialogContent>
    </Dialog>
  )
}
