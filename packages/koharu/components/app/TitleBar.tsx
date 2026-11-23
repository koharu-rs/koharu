'use client'

import * as Sentry from '@sentry/nextjs'
import { openUrl } from '@tauri-apps/plugin-opener'
import { BookOpen, FilePlus2, FolderOpen, LoaderCircle, PenLine } from 'lucide-react'
import Image from 'next/image'
import { useState, type ComponentProps } from 'react'
import { useTranslation } from 'react-i18next'

import { AboutDialog } from '@/components/app/AboutDialog'
import { useMacOS, WindowControls } from '@/components/app/WindowChrome'
import { send, useCommand, useImportPages, usePage, useProject } from '@/lib/queries'
import { deleteSelection } from '@/lib/scene'
import { shortcutLabel, shortcuts } from '@/lib/shortcuts'
import { useKoharuStore } from '@/lib/store'
import { commands } from '@koharu/ipc/protocol'
import {
  Menubar,
  MenubarCheckboxItem,
  MenubarContent,
  MenubarItem,
  MenubarMenu,
  MenubarSeparator,
  MenubarShortcut,
  MenubarSub,
  MenubarSubContent,
  MenubarSubTrigger,
  MenubarTrigger as UiMenubarTrigger,
} from '@koharu/ui/components/menubar'
import { ToggleGroup, ToggleGroupItem } from '@koharu/ui/components/toggle-group'
import { cn } from '@koharu/ui/lib/utils'

export function TitleBar() {
  const { t } = useTranslation()
  const [aboutOpen, setAboutOpen] = useState(false)
  const macOS = useMacOS()
  const project = useProject().data
  const pageQuery = usePage(Boolean(project))
  const pages = project?.pages ?? []
  const page = project ? (pageQuery.data ?? null) : null
  const pageNumber = page ? pages.findIndex((candidate) => candidate.id === page.id) + 1 : 0
  const history = project?.history
  const historyOpen = useKoharuStore((state) => state.historyOpen)
  const setHistoryOpen = useKoharuStore((state) => state.setHistoryOpen)
  const selectedLayers = useKoharuStore((state) => state.selectedLayers)
  const selectLayers = useKoharuStore((state) => state.selectLayers)
  const setSettingsOpen = useKoharuStore((state) => state.setSettingsOpen)
  const settingsOpen = useKoharuStore((state) => state.settingsOpen)
  const reading = useKoharuStore((state) => state.reading)
  const setReading = useKoharuStore((state) => state.setReading)
  const { importPages, importing } = useImportPages()
  const { run: exportProject, busy: exporting } = useCommand(
    ['export-project'],
    commands.export,
    'menu.export',
  )

  const psdLabels = {
    background: t('psd.background'),
    cleanup: t('psd.cleanup'),
    outline: t('psd.outline'),
  }

  /** Sentry's feedback form, in the interface's language; it leaves the page when it closes. */
  const sendFeedback = async () => {
    const feedback = Sentry.getFeedback()
    if (!feedback) return
    const form = await feedback.createForm({
      formTitle: t('feedback.title'),
      nameLabel: t('feedback.name'),
      namePlaceholder: t('feedback.namePlaceholder'),
      emailLabel: t('feedback.email'),
      emailPlaceholder: t('feedback.emailPlaceholder'),
      messageLabel: t('feedback.message'),
      messagePlaceholder: t('feedback.messagePlaceholder'),
      isRequiredLabel: t('feedback.required'),
      submitButtonLabel: t('feedback.submit'),
      cancelButtonLabel: t('feedback.cancel'),
      confirmButtonLabel: t('feedback.confirm'),
      addScreenshotButtonLabel: t('feedback.addScreenshot'),
      removeScreenshotButtonLabel: t('feedback.removeScreenshot'),
      highlightToolText: t('feedback.highlight'),
      hideToolText: t('feedback.hide'),
      removeHighlightText: t('feedback.removeHighlight'),
      successMessageText: t('feedback.sent'),
      errorEmptyMessageText: t('feedback.emptyMessage'),
      errorNoClientText: t('feedback.noClient'),
      errorTimeoutText: t('feedback.timeout'),
      errorForbiddenText: t('feedback.forbidden'),
      errorGenericText: t('feedback.failed'),
      onFormClose: () => form.removeFromDom(),
    })
    form.appendToDom()
    form.open()
  }

  return (
    <>
      <header
        data-tauri-drag-region='deep'
        className='relative flex h-10 shrink-0 items-center bg-[var(--surface-titlebar)] text-[12px]'
      >
        {macOS ? (
          <div className='w-[84px] shrink-0' />
        ) : (
          <div className='relative z-10 flex h-full w-10 shrink-0 items-center justify-center rounded-br-lg'>
            {/* Its weight is in its top, its feet pale, so centred it reads 2 px above the
                menus' words; lowered by that to sit on their line. */}
            <Image
              className='pointer-events-none translate-y-0.5'
              src='/icon.png'
              alt='Koharu'
              width={17}
              height={17}
              draggable={false}
              priority
            />
          </div>
        )}
        <Menubar className='relative z-10 h-full shrink-0 gap-0 border-0 bg-transparent p-0 shadow-none'>
          <MenubarMenu>
            <MenubarTrigger>{t('menu.file')}</MenubarTrigger>
            <MenubarContent>
              <MenubarSub>
                <MenubarSubTrigger disabled={!project || importing} aria-busy={importing}>
                  {importing && <LoaderCircle className='animate-spin' aria-hidden='true' />}
                  {importing ? t('navigator.importing') : t('menu.import')}
                </MenubarSubTrigger>
                <MenubarSubContent>
                  <MenubarItem disabled={importing} onClick={() => importPages('files')}>
                    <FilePlus2 />
                    {t('navigator.importFiles')}
                  </MenubarItem>
                  <MenubarItem disabled={importing} onClick={() => importPages('folder')}>
                    <FolderOpen />
                    {t('navigator.importFolder')}
                  </MenubarItem>
                </MenubarSubContent>
              </MenubarSub>
              <MenubarSub>
                <MenubarSubTrigger
                  disabled={!project || pages.length === 0 || exporting}
                  aria-busy={exporting}
                >
                  {exporting && <LoaderCircle className='animate-spin' aria-hidden='true' />}
                  {t('menu.export')}
                </MenubarSubTrigger>
                <MenubarSubContent>
                  {(['png', 'psd', 'cbz'] as const).map((format) => (
                    <MenubarItem
                      key={format}
                      disabled={exporting}
                      onClick={() => exportProject(format, psdLabels)}
                    >
                      {format.toUpperCase()}…
                    </MenubarItem>
                  ))}
                </MenubarSubContent>
              </MenubarSub>
              <MenubarSeparator />
              <MenubarItem disabled={!project} onClick={() => send(commands.close)}>
                {t('menu.closeProject')}
              </MenubarItem>
              <MenubarSeparator />
              <MenubarItem onClick={() => setSettingsOpen(true)}>{t('menu.settings')}</MenubarItem>
            </MenubarContent>
          </MenubarMenu>

          <MenubarMenu>
            <MenubarTrigger>{t('menu.edit')}</MenubarTrigger>
            <MenubarContent>
              <MenubarItem
                disabled={!history || history.current === 0}
                onClick={() => send(commands.undo)}
              >
                {t('menu.undo')}
                <MenubarShortcut>{shortcutLabel(shortcuts.undo)}</MenubarShortcut>
              </MenubarItem>
              <MenubarItem
                disabled={!history || history.current >= history.steps.length}
                onClick={() => send(commands.redo)}
              >
                {t('menu.redo')}
                <MenubarShortcut>{shortcutLabel(shortcuts.redo)}</MenubarShortcut>
              </MenubarItem>
              <MenubarSeparator />
              <MenubarItem
                disabled={!page}
                onClick={() => selectLayers(page?.layers.map((layer) => layer.id) ?? [])}
              >
                {t('menu.selectAllLayers')}
                <MenubarShortcut>{shortcutLabel(shortcuts.selectAll)}</MenubarShortcut>
              </MenubarItem>
              <MenubarItem
                disabled={selectedLayers.length === 0}
                variant='destructive'
                onClick={() => send(deleteSelection, selectedLayers, page?.layers ?? [])}
              >
                {t('menu.delete')}
                <MenubarShortcut>{shortcutLabel(shortcuts.delete)}</MenubarShortcut>
              </MenubarItem>
              <MenubarSeparator />
              {/* Frees the GPU, as Photoshop's Edit → Purge frees memory. */}
              <MenubarItem onClick={() => send(commands.unload)}>
                {t('menu.unloadModels')}
              </MenubarItem>
            </MenubarContent>
          </MenubarMenu>

          <MenubarMenu>
            <MenubarTrigger>{t('menu.view')}</MenubarTrigger>
            <MenubarContent>
              <MenubarItem disabled={!page} onClick={() => send(commands.fit)}>
                {t('menu.fit')}
                <MenubarShortcut>{shortcutLabel(shortcuts.fit)}</MenubarShortcut>
              </MenubarItem>
              <MenubarCheckboxItem
                disabled={!page}
                checked={historyOpen}
                onCheckedChange={setHistoryOpen}
                closeOnClick
              >
                {t('menu.history')}
              </MenubarCheckboxItem>
            </MenubarContent>
          </MenubarMenu>

          <MenubarMenu>
            <MenubarTrigger>{t('menu.help')}</MenubarTrigger>
            <MenubarContent>
              <MenubarItem
                onClick={() =>
                  void openUrl('https://github.com/koharu-rs/koharu').catch(() => undefined)
                }
              >
                {t('menu.github')}
              </MenubarItem>
              <MenubarItem
                onClick={() => void openUrl('https://discord.gg/mHvHkxGnUY').catch(() => undefined)}
              >
                {t('menu.discord')}
              </MenubarItem>
              <MenubarSeparator />
              <MenubarItem
                onClick={() =>
                  void openUrl('https://github.com/sponsors/mayocream').catch(() => undefined)
                }
              >
                {t('menu.donate')}
              </MenubarItem>
              <MenubarItem onClick={() => void sendFeedback().catch(() => undefined)}>
                {t('menu.feedback')}
              </MenubarItem>
              <MenubarSeparator />
              <MenubarItem onClick={() => setAboutOpen(true)}>{t('menu.about')}</MenubarItem>
            </MenubarContent>
          </MenubarMenu>
        </Menubar>

        <div className='min-w-0 flex-1' />
        <div className='pointer-events-none absolute inset-y-0 left-1/2 flex max-w-[40vw] min-w-16 -translate-x-1/2 items-center justify-center px-3 text-[11px] text-muted-foreground select-none'>
          {project ? (
            <span className='truncate'>
              <span className='font-medium text-foreground'>{project.name}</span>
              {page && (
                <>
                  <span className='mx-2'>/</span>
                  <span>{t('navigator.pageNumber', { number: pageNumber })}</span>
                </>
              )}
            </span>
          ) : (
            <span>Koharu</span>
          )}
        </div>

        {project && !settingsOpen && (
          <ToggleGroup
            value={[reading ? 'read' : 'edit']}
            onValueChange={([next]) => next && setReading(next === 'read')}
            aria-label={t('reader.mode')}
            className='relative z-10 mr-2 shrink-0 rounded-md border border-input bg-background p-0.5'
          >
            {(
              [
                ['read', BookOpen, t('reader.read')],
                ['edit', PenLine, t('reader.edit')],
              ] as const
            ).map(([value, Icon, label]) => (
              <ToggleGroupItem
                key={value}
                value={value}
                className='h-6 gap-1 px-2 text-[10px] text-muted-foreground aria-pressed:bg-primary aria-pressed:text-primary-foreground hover:aria-pressed:bg-primary/90'
              >
                <Icon className='size-3' />
                {label}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        )}
        {!macOS && <WindowControls />}
      </header>
      <AboutDialog open={aboutOpen} onOpenChange={setAboutOpen} />
    </>
  )
}

function MenubarTrigger({ className, ...props }: ComponentProps<typeof UiMenubarTrigger>) {
  return (
    <UiMenubarTrigger
      className={cn(
        'h-6 px-1.5 text-[11px] text-muted-foreground transition-colors hover:bg-primary/10 hover:text-muted-foreground aria-expanded:bg-primary/10 aria-expanded:text-muted-foreground',
        className,
      )}
      {...props}
    />
  )
}
