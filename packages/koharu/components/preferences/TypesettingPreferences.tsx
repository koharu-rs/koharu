'use client'

import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Choice } from '@/components/controls/Choice'
import { KernelFaces } from '@/components/controls/KernelFaces'
import { PreferencePage, PreferenceSection } from '@/components/preferences/PreferenceFields'
import { scriptNames } from '@/lib/i18n'
import { useFonts } from '@/lib/queries'
import { SCRIPTS, type Faces } from '@/lib/styles'
import type { RendererConfig } from '@koharu/ipc/protocol'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@koharu/ui/components/alert-dialog'
import { Button } from '@koharu/ui/components/button'

/** A letter or two of each script, which its menu shows before its name, as Font Book's languages. */
const SAMPLES: Partial<Record<string, string>> = {
  Latn: 'Aa',
  Arab: 'ع',
  Cyrl: 'Бб',
  Beng: 'অ',
  Tibt: 'ཀ',
  Gujr: 'અ',
  Hebr: 'א',
  Deva: 'अ',
  Jpan: 'あ',
  Khmr: 'ក',
  Kore: '가',
  Mymr: 'က',
  Taml: 'அ',
  Telu: 'అ',
  Thai: 'ก',
  Hant: '漢',
  Hans: '汉',
}

/** The styles each new project starts with, a script at a time, which a project keeps as its own
 * from then on. */
export function TypesettingPreferences({
  value,
  onChange,
}: {
  value: RendererConfig
  onChange: (value: RendererConfig) => void
}) {
  const { t, i18n } = useTranslation()
  const fonts = useFonts()
  const [script, setScript] = useState(SCRIPTS[0])
  const [restoring, setRestoring] = useState(false)
  const names = scriptNames(i18n.language)
  const faces: Faces = value.styles ?? {}
  return (
    <PreferencePage
      title={t('settings.typesetting.title')}
      description={t('settings.typesetting.description')}
    >
      <PreferenceSection
        title={t('settings.typesetting.styles')}
        description={t('settings.typesetting.stylesDescription')}
      >
        {/* The script, as Firefox's Fonts for, and a way back to Koharu's own styles, as a
          browser's Reset settings. */}
        <div className='flex items-center gap-3 py-3.5'>
          <span className='text-[12px] font-medium'>{t('settings.typesetting.script')}</span>
          <Choice
            className='w-48'
            value={script}
            choices={Object.fromEntries(
              SCRIPTS.map((code) => [
                code,
                <span key={code} className='flex min-w-0 items-center gap-2'>
                  {/* In the script's own lang, so a Han sample takes its own region's face. */}
                  <span
                    aria-hidden
                    lang={`und-${code}`}
                    className='grid size-5 shrink-0 place-items-center rounded-xs bg-primary/10 text-[11px] font-medium text-primary'
                  >
                    {SAMPLES[code]}
                  </span>
                  <span className='truncate'>{names.of(code) ?? code}</span>
                </span>,
              ]),
            )}
            label={t('settings.typesetting.script')}
            onChange={setScript}
          />
          <Button
            type='button'
            variant='ghost'
            size='sm'
            className='ml-auto text-muted-foreground'
            onClick={() => setRestoring(true)}
          >
            {t('settings.typesetting.restore')}
          </Button>
        </div>
        <KernelFaces
          script={script}
          faces={faces}
          families={fonts.data ?? []}
          disabled={fonts.isPending}
          onChange={(kernel, face) => {
            const { [script]: _, ...others } = faces[kernel] ?? {}
            const cells = face ? { ...others, [script]: face } : others
            onChange({ ...value, styles: { ...faces, [kernel]: cells } })
          }}
        />
      </PreferenceSection>
      <AlertDialog open={restoring} onOpenChange={setRestoring}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('settings.typesetting.restoreTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('settings.typesetting.restoreDescription')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('common.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setRestoring(false)
                // With no styles, the section saves Koharu's own, which the save answers with.
                onChange({ ...value, styles: undefined })
              }}
            >
              {t('settings.typesetting.restore')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      {fonts.isError && (
        <p role='status' className='-mt-6 text-[10px] leading-4 text-destructive'>
          {t('settings.typesetting.loadError')}
        </p>
      )}
    </PreferencePage>
  )
}
