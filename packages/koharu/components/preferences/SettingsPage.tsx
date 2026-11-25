'use client'

import { ArrowLeft, Cpu, KeyRound, Library, Monitor, Moon, Palette, Sun, Type } from 'lucide-react'
import { useTheme } from 'next-themes'
import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'

import { Choice } from '@/components/controls/Choice'
import { GalleryPreferences } from '@/components/preferences/GalleryPreferences'
import { PipelinePreferences } from '@/components/preferences/PipelinePreferences'
import {
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { ProviderPreferences } from '@/components/preferences/ProviderPreferences'
import { TypesettingPreferences } from '@/components/preferences/TypesettingPreferences'
import { nativeNames } from '@/lib/i18n'
import {
  modelsKey,
  preferencesKey,
  queryClient,
  refresh,
  useEditPreferences,
  useModels,
  usePreferences,
} from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { Button } from '@koharu/ui/components/button'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@koharu/ui/components/tabs'
import { ToggleGroup, ToggleGroupItem } from '@koharu/ui/components/toggle-group'

const tabs = [
  ['appearance', Palette],
  ['pipeline', Cpu],
  ['providers', KeyRound],
  ['typesetting', Type],
  ['gallery', Library],
] as const

export function SettingsPage() {
  const { t } = useTranslation()
  const setOpen = useKoharuStore((state) => state.setSettingsOpen)
  const preferences = usePreferences().data
  const models = useModels().data ?? []
  const { edit, flush } = useEditPreferences()

  // What the file holds as the page opens, which a save under way would answer itself.
  useEffect(() => {
    void refresh(modelsKey)
    if (!queryClient.isMutating({ mutationKey: preferencesKey })) void refresh(preferencesKey)
  }, [])

  return (
    <Tabs
      defaultValue='appearance'
      orientation='vertical'
      className='settings-page min-h-0 flex-1 gap-0 bg-[var(--surface-sidebar)] text-foreground'
    >
      <nav className='flex w-64 shrink-0 flex-col px-3 py-4'>
        <Button
          type='button'
          variant='ghost'
          className='mb-5 h-9 justify-start gap-2 rounded-lg px-2 text-[12px] text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground'
          onClick={() => {
            flush()
            setOpen(false)
          }}
        >
          <ArrowLeft className='size-4' /> {t('settings.backToEditor')}
        </Button>
        <p className='mb-2 px-2 text-[10px] font-semibold tracking-[0.14em] text-muted-foreground uppercase'>
          {t('settings.title')}
        </p>
        <TabsList className='w-full gap-1 bg-transparent p-0'>
          {tabs.map(([id, Icon]) => (
            <TabsTrigger
              key={id}
              value={id}
              className='h-10 flex-none gap-3 rounded-lg px-3 text-[12px] font-normal text-muted-foreground hover:bg-foreground/[0.06] data-active:bg-accent data-active:text-accent-foreground group-data-[variant=default]/tabs-list:data-active:shadow-none data-active:hover:bg-accent data-active:hover:text-accent-foreground dark:data-active:border-transparent dark:data-active:bg-accent dark:data-active:text-accent-foreground'
            >
              <Icon /> {t(`settings.tabs.${id}`)}
            </TabsTrigger>
          ))}
        </TabsList>
      </nav>
      <main className='relative z-10 flex min-w-0 flex-1 flex-col overflow-hidden rounded-tl-2xl bg-[var(--surface-canvas)] shadow-[var(--shadow-content)]'>
        <header className='flex h-14 shrink-0 items-center border-b border-border/80 px-8'>
          <h1 className='text-[13px] font-semibold tracking-[-0.02em]'>{t('settings.title')}</h1>
        </header>
        <ScrollArea className='min-h-0 flex-1'>
          <div className='mx-auto w-full max-w-4xl px-10 py-10'>
            <TabsContent value='appearance'>
              <AppearancePreferences />
            </TabsContent>
            <TabsContent value='pipeline'>
              {preferences ? (
                <PipelinePreferences
                  value={preferences.pipeline}
                  models={models}
                  providers={preferences.providers}
                  onChange={(pipeline) => edit({ ...preferences, pipeline })}
                  onProvidersChange={(providers) => edit({ ...preferences, providers })}
                />
              ) : (
                <LoadingPreferences />
              )}
            </TabsContent>
            <TabsContent value='providers'>
              {preferences ? (
                <ProviderPreferences
                  value={preferences.providers}
                  onChange={(providers) => edit({ ...preferences, providers })}
                />
              ) : (
                <LoadingPreferences />
              )}
            </TabsContent>
            <TabsContent value='gallery'>
              <GalleryPreferences />
            </TabsContent>
            <TabsContent value='typesetting'>
              {preferences ? (
                <TypesettingPreferences
                  value={preferences.renderer}
                  onChange={(renderer) => edit({ ...preferences, renderer })}
                />
              ) : (
                <LoadingPreferences />
              )}
            </TabsContent>
          </div>
        </ScrollArea>
      </main>
    </Tabs>
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
          <ToggleGroup
            value={theme ? [theme] : []}
            onValueChange={([next]) => next && setTheme(next)}
            aria-label={t('settings.appearance.theme')}
            className='grid w-full grid-cols-3 gap-0 rounded-xl border border-input bg-background p-0.5'
          >
            {themes.map(([value, Icon]) => (
              <ToggleGroupItem
                key={value}
                value={value}
                className='h-9 gap-1.5 text-[10px] text-muted-foreground aria-pressed:bg-primary aria-pressed:text-primary-foreground hover:aria-pressed:bg-primary/90'
              >
                <Icon className='size-3.5' /> {t(`settings.appearance.themes.${value}`)}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        </PreferenceRow>
        <PreferenceRow title={t('settings.appearance.language')}>
          <Choice
            value={i18n.language}
            choices={nativeNames}
            label={t('settings.appearance.language')}
            onChange={(language) => void i18n.changeLanguage(language)}
          />
        </PreferenceRow>
      </PreferenceSection>
    </PreferencePage>
  )
}

function LoadingPreferences() {
  const { t } = useTranslation()
  return <p className='py-10 text-[12px] text-muted-foreground'>{t('settings.loading')}</p>
}
