'use client'

import * as Sentry from '@sentry/nextjs'
import { openUrl } from '@tauri-apps/plugin-opener'
import { FilePlus2, FolderOpen, LoaderCircle, Settings } from 'lucide-react'
import Image from 'next/image'
import { useState, type ComponentProps } from 'react'
import { useTranslation } from 'react-i18next'

import { AboutDialog } from '@/components/app/AboutDialog'
import { useMacOS, WindowControls } from '@/components/app/WindowChrome'
import { call, dispatch } from '@/lib/backend'
import { useCommand, useImportPages, usePage, useProject } from '@/lib/queries'
import { deleteLayers } from '@/lib/scene'
import { trace } from '@/lib/sentry'
import { pipelineStages, useKoharuStore } from '@/lib/store'
import { commands, type Stage, type Target } from '@koharu/bridge/protocol'
import {
  Menubar,
  MenubarContent as UiMenubarContent,
  MenubarItem as UiMenubarItem,
  MenubarMenu,
  MenubarSeparator as UiMenubarSeparator,
  MenubarShortcut as UiMenubarShortcut,
  MenubarSub,
  MenubarSubContent,
  MenubarSubTrigger,
  MenubarTrigger as UiMenubarTrigger,
} from '@koharu/ui/components/menubar'
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
  const selectedPages = useKoharuStore((state) => state.selectedPages)
  const selectedLayers = useKoharuStore((state) => state.selectedLayers)
  const selectLayers = useKoharuStore((state) => state.selectLayers)
  const setSettingsOpen = useKoharuStore((state) => state.setSettingsOpen)
  const { importPages, importing } = useImportPages()
  const { run: exportProject, busy: exporting } = useCommand(
    ['export-project'],
    commands.export,
    'menu.export',
  )

  const run = (target: Target, stages: Stage[] = [...pipelineStages]) =>
    void call(commands.process, target, stages, trace()).catch(() => undefined)
  const psdLabels = {
    background: t('psd.background'),
    cleanup: t('psd.cleanup'),
    outline: t('psd.outline'),
  }

  const closeProject = () => void call(commands.closeProject).catch(() => undefined)

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
            <Image
              className='pointer-events-none'
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
                <MenubarSubTrigger
                  disabled={!project || importing}
                  aria-busy={importing}
                  className='min-h-8 gap-1.5 px-2 py-1 text-xs'
                >
                  {importing && <LoaderCircle className='animate-spin' aria-hidden='true' />}
                  {importing ? t('navigator.importing') : t('menu.import')}
                </MenubarSubTrigger>
                <MenubarSubContent className='min-w-40 p-1'>
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
                  className='min-h-8 gap-1.5 px-2 py-1 text-xs'
                >
                  {exporting && <LoaderCircle className='animate-spin' aria-hidden='true' />}
                  {t('menu.export')}
                </MenubarSubTrigger>
                <MenubarSubContent className='min-w-40 p-1'>
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
              <MenubarItem disabled={!project} onClick={closeProject}>
                {t('menu.closeProject')}
              </MenubarItem>
              <MenubarSeparator />
              <MenubarItem onClick={() => setSettingsOpen(true)}>
                <Settings />
                {t('menu.settings')}
              </MenubarItem>
            </MenubarContent>
          </MenubarMenu>

          <MenubarMenu>
            <MenubarTrigger>{t('menu.edit')}</MenubarTrigger>
            <MenubarContent>
              <MenubarItem
                disabled={!history || history.current === 0}
                onClick={() => dispatch(commands.undo)}
              >
                {t('menu.undo')}
                <MenubarShortcut>Ctrl+Z</MenubarShortcut>
              </MenubarItem>
              <MenubarItem
                disabled={!history || history.current >= history.steps.length}
                onClick={() => dispatch(commands.redo)}
              >
                {t('menu.redo')}
                <MenubarShortcut>Ctrl+Shift+Z</MenubarShortcut>
              </MenubarItem>
              <MenubarSeparator />
              <MenubarItem
                disabled={!page}
                onClick={() => selectLayers(page?.layers.map((layer) => layer.id) ?? [])}
              >
                {t('menu.selectAllLayers')}
                <MenubarShortcut>Ctrl+A</MenubarShortcut>
              </MenubarItem>
              <MenubarItem
                disabled={selectedLayers.length === 0}
                variant='destructive'
                onClick={() => dispatch(deleteLayers, selectedLayers)}
              >
                {t('menu.delete')}
                <MenubarShortcut>Del</MenubarShortcut>
              </MenubarItem>
            </MenubarContent>
          </MenubarMenu>

          <MenubarMenu>
            <MenubarTrigger>{t('menu.process')}</MenubarTrigger>
            <MenubarContent>
              <MenubarItem disabled={!project || pages.length === 0} onClick={() => run('project')}>
                {t('menu.processProject')}
              </MenubarItem>
              <MenubarItem
                disabled={selectedPages.length === 0}
                onClick={() => run({ pages: selectedPages })}
              >
                {t('menu.processPages')}
              </MenubarItem>
              <MenubarItem
                disabled={selectedLayers.length === 0}
                onClick={() => run({ blocks: selectedLayers }, ['recognize', 'translate'])}
              >
                {t('menu.processLayers')}
              </MenubarItem>
              <MenubarSeparator />
              {pipelineStages.map((stage, index) => (
                <MenubarItem
                  key={stage}
                  disabled={!project || pages.length === 0}
                  onClick={() => run('project', pipelineStages.slice(0, index + 1))}
                >
                  {t('menu.runPhase', {
                    phase: t(`phase.${stage}`),
                  })}
                </MenubarItem>
              ))}
            </MenubarContent>
          </MenubarMenu>

          <MenubarMenu>
            <MenubarTrigger>{t('menu.view')}</MenubarTrigger>
            <MenubarContent>
              <MenubarItem disabled={!page} onClick={() => dispatch(commands.fitCanvas)}>
                {t('menu.fit')}
              </MenubarItem>
            </MenubarContent>
          </MenubarMenu>

          <MenubarMenu>
            <MenubarTrigger>{t('menu.help')}</MenubarTrigger>
            <MenubarContent>
              <MenubarItem
                onClick={() => void openUrl('https://discord.gg/mHvHkxGnUY').catch(() => undefined)}
              >
                {t('menu.discord')}
              </MenubarItem>
              <MenubarItem
                onClick={() =>
                  void openUrl('https://github.com/koharu-rs/koharu').catch(() => undefined)
                }
              >
                {t('menu.github')}
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

function MenubarContent({ className, ...props }: ComponentProps<typeof UiMenubarContent>) {
  return <UiMenubarContent className={cn('min-w-44 p-1', className)} {...props} />
}

function MenubarItem({ className, ...props }: ComponentProps<typeof UiMenubarItem>) {
  return (
    <UiMenubarItem
      className={cn(
        "min-h-8 gap-1.5 px-2 py-1 text-xs [&_svg:not([class*='size-'])]:size-3.5",
        className,
      )}
      {...props}
    />
  )
}

function MenubarShortcut({ className, ...props }: ComponentProps<typeof UiMenubarShortcut>) {
  return <UiMenubarShortcut className={cn('text-[11px]', className)} {...props} />
}

function MenubarSeparator({ className, ...props }: ComponentProps<typeof UiMenubarSeparator>) {
  return <UiMenubarSeparator className={cn('my-0.5', className)} {...props} />
}
