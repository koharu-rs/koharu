'use client'

import {
  ArrowLeft,
  Cpu,
  KeyRound,
  Keyboard,
  Languages,
  Monitor,
  Moon,
  Palette,
  Sun,
  Type,
} from 'lucide-react'
import { useTheme } from 'next-themes'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PipelinePreferences } from '@/components/preferences/PipelinePreferences'
import {
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { ProviderPreferences } from '@/components/preferences/ProviderPreferences'
import {
  promptNamesTarget,
  TranslationPreferences,
} from '@/components/preferences/TranslationPreferences'
import { TypesettingPreferences } from '@/components/preferences/TypesettingPreferences'
import { refreshModels, refreshPreferences, savePreferences } from '@/lib/backend'
import { supportedLanguages } from '@/lib/i18n'
import { useKoharuStore, type ShortcutAction } from '@/lib/store'
import type { Preferences } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'

const tabs = [
  ['appearance', Palette],
  ['pipeline', Cpu],
  ['providers', KeyRound],
  ['translation', Languages],
  ['typesetting', Type],
  ['shortcuts', Keyboard],
] as const
type Tab = (typeof tabs)[number][0]

export function SettingsPage() {
  const { t } = useTranslation()
  const open = useKoharuStore((state) => state.settingsOpen)
  const setOpen = useKoharuStore((state) => state.setSettingsOpen)
  const preferences = useKoharuStore((state) => state.preferences)
  const models = useKoharuStore((state) => state.models)
  const [tab, setTab] = useState<Tab>('appearance')
  const [draft, setDraft] = useState<Preferences | null>(preferences)
  const lastSaved = useRef<string | null>(null)
  const pending = useRef<Promise<unknown>>(Promise.resolve())

  // A draft the backend would refuse waits for its fix instead of failing a save per keystroke.
  const saveDraft = useCallback((next: Preferences) => {
    const serialized = JSON.stringify(next)
    if (serialized === lastSaved.current || !promptNamesTarget(next.pipeline.translator ?? {})) {
      return pending.current
    }
    lastSaved.current = serialized
    pending.current = savePreferences(next).catch((error: unknown) => {
      if (lastSaved.current === serialized) lastSaved.current = null
      throw error
    })
    return pending.current
  }, [])

  useEffect(() => {
    if (!open) return
    void refreshPreferences().catch(() => undefined)
    void refreshModels().catch(() => undefined)
  }, [open])

  useEffect(() => {
    if (!open || !preferences) return
    const serialized = JSON.stringify(preferences)
    // The file's state replaces the draft only when it isn't this draft's own save coming back.
    if (serialized === lastSaved.current) return
    lastSaved.current = serialized
    setDraft(preferences)
  }, [open, preferences])

  useEffect(() => {
    if (!open || !draft) return
    if (JSON.stringify(draft) === lastSaved.current) return
    const timeout = window.setTimeout(() => void saveDraft(draft).catch(() => undefined), 260)
    return () => window.clearTimeout(timeout)
  }, [draft, open, saveDraft])

  if (!open) return null

  return (
    <section className='settings-page flex min-h-0 flex-1 bg-[var(--surface-sidebar)] text-foreground'>
      <nav className='flex w-64 shrink-0 flex-col bg-[var(--surface-sidebar)] px-3 py-4'>
        <Button
          type='button'
          variant='ghost'
          className='mb-5 h-9 justify-start gap-2 rounded-lg px-2 text-[12px] text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground'
          onClick={() => {
            if (!draft) {
              setOpen(false)
              return
            }
            void saveDraft(draft)
              .then(() => setOpen(false))
              .catch(() => undefined)
          }}
        >
          <ArrowLeft className='size-4' /> {t('settings.backToEditor')}
        </Button>
        <p className='mb-2 px-2 text-[10px] font-semibold tracking-[0.14em] text-muted-foreground uppercase'>
          {t('settings.title')}
        </p>
        <div className='grid gap-1'>
          {tabs.map(([id, Icon]) => (
            <Button
              key={id}
              type='button'
              variant='ghost'
              size='sm'
              data-active={tab === id}
              className='h-10 w-full justify-start gap-3 rounded-lg px-3 text-left text-[12px] text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground data-[active=true]:bg-accent data-[active=true]:text-accent-foreground'
              onClick={() => setTab(id)}
            >
              <Icon className='size-4' /> {t(`settings.tabs.${id}`)}
            </Button>
          ))}
        </div>
      </nav>
      <main className='relative z-10 flex min-w-0 flex-1 flex-col overflow-hidden rounded-tl-2xl bg-[var(--surface-canvas)] shadow-[var(--shadow-content)]'>
        <header className='flex h-14 shrink-0 items-center border-b border-border/80 px-8'>
          <h1 className='text-[13px] font-semibold tracking-[-0.02em]'>{t('settings.title')}</h1>
        </header>
        <ScrollArea className='min-h-0 flex-1'>
          <div className='mx-auto w-full max-w-4xl px-10 py-10'>
            {tab === 'appearance' && <AppearancePreferences />}
            {tab !== 'appearance' && tab !== 'shortcuts' && !draft && <LoadingPreferences />}
            {tab === 'pipeline' && draft && (
              <PipelinePreferences
                value={draft.pipeline}
                onChange={(pipeline) => setDraft({ ...draft, pipeline })}
              />
            )}
            {tab === 'providers' && draft && (
              <ProviderPreferences
                value={draft.providers}
                onChange={(providers) => setDraft({ ...draft, providers })}
              />
            )}
            {tab === 'translation' && draft && (
              <TranslationPreferences
                value={draft.pipeline.translator ?? {}}
                models={models}
                onChange={(translator) =>
                  setDraft({ ...draft, pipeline: { ...draft.pipeline, translator } })
                }
              />
            )}
            {tab === 'typesetting' && draft && (
              <TypesettingPreferences
                value={draft.renderer}
                onChange={(renderer) => setDraft({ ...draft, renderer })}
              />
            )}
            {tab === 'shortcuts' && <ShortcutPreferences />}
          </div>
        </ScrollArea>
      </main>
    </section>
  )
}

function AppearancePreferences() {
  const { theme, setTheme } = useTheme()
  const { t, i18n } = useTranslation()
  const themes = [
    ['light', Sun],
    ['dark', Moon],
    ['system', Monitor],
  ] as const

  return (
    <PreferencePage
      title={t('settings.appearance.title')}
      description={t('settings.appearance.description')}
    >
      <PreferenceSection title={t('settings.appearance.interface')}>
        <PreferenceRow
          title={t('settings.appearance.theme')}
          description={t('settings.appearance.themeDescription')}
        >
          <div className='grid grid-cols-3 rounded-xl border border-input bg-background p-0.5'>
            {themes.map(([value, Icon]) => (
              <Button
                key={value}
                type='button'
                variant='ghost'
                size='sm'
                data-active={theme === value}
                className='h-9 gap-1.5 text-[10px] text-muted-foreground hover:text-foreground data-[active=true]:bg-primary data-[active=true]:text-primary-foreground data-[active=true]:hover:bg-primary/90 data-[active=true]:hover:text-primary-foreground'
                onClick={() => setTheme(value)}
              >
                <Icon className='size-3.5' /> {t(`settings.appearance.themes.${value}`)}
              </Button>
            ))}
          </div>
        </PreferenceRow>
        <PreferenceRow title={t('settings.appearance.language')}>
          <Select
            value={i18n.language}
            items={Object.fromEntries(
              supportedLanguages.map((language) => [language, t(`languages.${language}`)]),
            )}
            onValueChange={(language) => language && i18n.changeLanguage(language)}
          >
            <SelectTrigger className='h-8 w-full text-[11px]'>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {supportedLanguages.map((language) => (
                <SelectItem key={language} value={language}>
                  {t(`languages.${language}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </PreferenceRow>
      </PreferenceSection>
    </PreferencePage>
  )
}

function ShortcutPreferences() {
  const { t } = useTranslation()
  const shortcuts = useKoharuStore((state) => state.shortcuts)
  const setShortcut = useKoharuStore((state) => state.setShortcut)
  const actions: ShortcutAction[] = [
    'select',
    'text',
    'draw',
    'eraser',
    'color_picker',
    'remove',
    'pan',
    'fit',
  ]
  return (
    <PreferencePage
      title={t('settings.shortcuts.title')}
      description={t('settings.shortcuts.description')}
    >
      <PreferenceSection title={t('settings.shortcuts.tools')}>
        {actions.map((action) => (
          <PreferenceRow key={action} title={t(shortcutKeys[action])}>
            <Input
              aria-label={t('settings.shortcuts.inputLabel', { action: t(shortcutKeys[action]) })}
              maxLength={1}
              value={shortcuts[action]}
              className='ml-auto h-8 w-14 text-center text-[12px] uppercase'
              onChange={(event) => setShortcut(action, event.currentTarget.value)}
            />
          </PreferenceRow>
        ))}
      </PreferenceSection>
    </PreferencePage>
  )
}

function LoadingPreferences() {
  const { t } = useTranslation()
  return <p className='py-10 text-[12px] text-muted-foreground'>{t('settings.loading')}</p>
}

const shortcutKeys: Record<ShortcutAction, string> = {
  select: 'tools.select',
  text: 'tools.text',
  draw: 'tools.draw',
  eraser: 'tools.eraser',
  color_picker: 'tools.color_picker',
  remove: 'tools.remove',
  pan: 'tools.pan',
  fit: 'canvas.fit',
}
