'use client'

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PreferenceRow, PreferenceSection } from '@/components/preferences/PreferenceFields'
import { call } from '@/lib/backend'
import { commands, type WorkflowSettings } from '@koharu/bridge/protocol'
import { Switch } from '@koharu/ui/components/switch'

export function WorkflowPreferences() {
  const { t } = useTranslation()
  const [settings, setSettings] = useState<WorkflowSettings | null>(null)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    let active = true
    void call(commands.getWorkflowSettings)
      .then((value) => {
        if (active) setSettings(value)
      })
      .catch(() => undefined)
    return () => {
      active = false
    }
  }, [])

  const save = (enabled: boolean, preset: string) => {
    setSaving(true)
    void call(commands.configureProjectWorkflow, enabled, preset)
      .then(setSettings)
      .catch(() => undefined)
      .finally(() => setSaving(false))
  }

  return (
    <PreferenceSection title={t('workflow.title')}>
      <PreferenceRow title={t('workflow.enable')} description={t('workflow.enableDescription')}>
        <Switch
          aria-label={t('workflow.enable')}
          checked={settings?.enabled ?? false}
          disabled={!settings || saving}
          onCheckedChange={(enabled) => save(enabled, settings!.active_preset ?? '')}
        />
      </PreferenceRow>
      <PreferenceRow title={t('workflow.preset')} description={t('workflow.presetDescription')}>
        <select
          aria-label={t('workflow.preset')}
          className='h-8 max-w-full rounded-md border border-border bg-background px-2 text-[11px]'
          disabled={!settings || saving}
          value={settings?.active_preset ?? ''}
          onChange={(event) => save(settings?.enabled ?? false, event.target.value)}
        >
          {(settings?.presets ?? []).map((preset) => (
            <option key={preset.name} value={preset.name}>
              {preset.name}
            </option>
          ))}
        </select>
      </PreferenceRow>
    </PreferenceSection>
  )
}
