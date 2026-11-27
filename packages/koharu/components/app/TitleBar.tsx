'use client'

import * as Sentry from '@sentry/nextjs'
import { useIsMutating, useMutation } from '@tanstack/react-query'
import { openUrl } from '@tauri-apps/plugin-opener'
import { BookOpen, BookText, FilePlus2, FolderOpen, LoaderCircle, PenLine } from 'lucide-react'
import Image from 'next/image'
import { useState, type ComponentProps } from 'react'
import { useTranslation } from 'react-i18next'

import { AboutDialog } from '@/components/app/AboutDialog'
import { checkForUpdates } from '@/components/app/Updater'
import { useMacOS, WindowControls } from '@/components/app/WindowChrome'
import { Choice } from '@/components/controls/Choice'
import { MetadataDialog } from '@/components/editor/MetadataPanel'
import {
  send,
  useCommand,
  useImportPages,
  usePage,
  usePreferences,
  useProject,
  useSavePreferences,
} from '@/lib/queries'
import { applyDefaultStyles, deleteSelection } from '@/lib/scene'
import { shortcutLabel, shortcuts, useKeyboard } from '@/lib/shortcuts'
import { useKoharuStore } from '@/lib/store'
import {
  commands,
  type ExportConfig,
  type ImageCodec,
  type ImageFormat,
  type PageId,
} from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@koharu/ui/components/dialog'
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
import { Slider } from '@koharu/ui/components/slider'
import { Switch } from '@koharu/ui/components/switch'
import { ToggleGroup, ToggleGroupItem } from '@koharu/ui/components/toggle-group'
import { cn } from '@koharu/ui/lib/utils'

export function TitleBar() {
  const { t } = useTranslation()
  const [aboutOpen, setAboutOpen] = useState(false)
  // Kept once asked for, so the dialog closes in place.
  const [exportOptions, setExportOptions] = useState<ExportOptions>()
  const [exportOpen, setExportOpen] = useState(false)
  const askExportOptions = (options: ExportOptions) => {
    setExportOptions(options)
    setExportOpen(true)
  }
  const macOS = useMacOS()
  const project = useProject().data
  const preferences = usePreferences().data
  const savePreferences = useSavePreferences()
  const pageQuery = usePage(Boolean(project))
  const pages = project?.pages ?? []
  const page = project ? (pageQuery.data ?? null) : null
  const pageNumber = page ? pages.findIndex((candidate) => candidate.id === page.id) + 1 : 0
  const history = project?.history
  const original = useKoharuStore((state) => state.original)
  const setOriginal = useKoharuStore((state) => state.setOriginal)
  const historyOpen = useKoharuStore((state) => state.historyOpen)
  const setHistoryOpen = useKoharuStore((state) => state.setHistoryOpen)
  const selectedLayers = useKoharuStore((state) => state.selectedLayers)
  const selectLayers = useKoharuStore((state) => state.selectLayers)
  const selectedPages = useKoharuStore((state) => state.selectedPages)
  const setSettingsOpen = useKoharuStore((state) => state.setSettingsOpen)
  const settingsOpen = useKoharuStore((state) => state.settingsOpen)
  const metadataOpen = useKoharuStore((state) => state.metadataOpen)
  const setMetadataOpen = useKoharuStore((state) => state.setMetadataOpen)
  const reading = useKoharuStore((state) => state.reading)
  const setReading = useKoharuStore((state) => state.setReading)
  const { importPages, importing } = useImportPages()
  const updateCheck = useMutation({ mutationFn: () => checkForUpdates(true) })
  // The selection keeps a page an undo or an agent took away.
  const exportable = pages.map(({ id }) => id).filter((id) => selectedPages.includes(id))

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
              width={16}
              height={16}
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
              <ExportMenu
                label='menu.export'
                pages={null}
                disabled={pages.length === 0}
                onOptions={askExportOptions}
              />
              <ExportMenu
                label='menu.exportSelected'
                pages={exportable}
                disabled={exportable.length === 0}
                onOptions={askExportOptions}
              />
              <MenubarSeparator />
              <MenubarItem disabled={!project} onClick={() => setMetadataOpen(true)}>
                <BookText />
                {t('menu.metadata')}
              </MenubarItem>
              <MenubarSeparator />
              {/* Google Docs' Use my default styles and Save as my default styles: a project keeps
                its own styles, which Preferences' only start. */}
              <MenubarItem
                disabled={!project || !preferences}
                onClick={() =>
                  preferences && send(applyDefaultStyles, preferences.renderer.styles ?? {})
                }
              >
                {t('menu.useDefaultStyles')}
              </MenubarItem>
              <MenubarItem
                disabled={!project || !preferences}
                onClick={() =>
                  project &&
                  preferences &&
                  savePreferences.mutate({
                    ...preferences,
                    renderer: { ...preferences.renderer, styles: project.meta.styles ?? {} },
                  })
                }
              >
                {t('menu.saveDefaultStyles')}
              </MenubarItem>
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
                checked={original}
                onCheckedChange={setOriginal}
                closeOnClick
              >
                {t('menu.original')}
                <KeyShortcut code={shortcuts.original} />
              </MenubarCheckboxItem>
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
              <MenubarItem disabled={updateCheck.isPending} onClick={() => updateCheck.mutate()}>
                {t('menu.checkForUpdates')}
              </MenubarItem>
              <MenubarItem onClick={() => setAboutOpen(true)}>{t('menu.about')}</MenubarItem>
            </MenubarContent>
          </MenubarMenu>
        </Menubar>

        <div
          data-tauri-drag-region='deep'
          className='flex min-w-0 flex-1 items-center justify-center px-3 text-[11px] text-muted-foreground select-none'
        >
          {project ? (
            <span className='flex max-w-full min-w-0 items-center'>
              <span className='min-w-0 truncate font-medium text-foreground'>{project.name}</span>
              {page && (
                <>
                  <span className='mx-2 shrink-0'>/</span>
                  <span className='shrink-0'>
                    {t('navigator.pageNumber', { number: pageNumber })}
                  </span>
                </>
              )}
            </span>
          ) : (
            <span>Koharu</span>
          )}
        </div>

        {project && !settingsOpen && (
          <>
            <Button
              type='button'
              variant='ghost'
              size='xs'
              onClick={() => setMetadataOpen(true)}
              className='relative z-10 mr-1.5 h-6 shrink-0 gap-1 px-2 text-[10px] text-muted-foreground hover:bg-primary/10 hover:text-foreground'
            >
              <BookText className='size-3' aria-hidden='true' />
              {t('metadata.button')}
            </Button>
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
          </>
        )}
        {!macOS && <WindowControls />}
      </header>
      <AboutDialog open={aboutOpen} onOpenChange={setAboutOpen} />
      <MetadataDialog open={metadataOpen} onOpenChange={setMetadataOpen} />
      {exportOptions && (
        <ExportDialog options={exportOptions} open={exportOpen} onOpenChange={setExportOpen} />
      )}
    </>
  )
}

type ExportLabel = 'menu.export' | 'menu.exportSelected'

/** An export whose options its dialog asks for. */
type ExportOptions = {
  label: ExportLabel
  pages: PageId[] | null
  format: 'jpeg' | 'webp' | 'jxl' | 'cbz'
}

const imageNames: Record<ImageCodec, string> = {
  png: 'PNG',
  jpeg: 'JPEG',
  webp: 'WebP',
  jxl: 'JPEG XL',
}

/** The formats the pages go out in, all of them when `pages` is null. JPEG, WebP, JPEG XL, and CBZ
 * ask for their options first. One export runs at a time. */
function ExportMenu({
  label,
  pages,
  disabled,
  onOptions,
}: {
  label: ExportLabel
  pages: PageId[] | null
  disabled: boolean
  onOptions: (options: ExportOptions) => void
}) {
  const { t } = useTranslation()
  const { run, busy } = useCommand(['export', label], commands.export, label)
  const exporting = useIsMutating({ mutationKey: ['export'] }) > 0
  const formats = [
    ['PNG', () => run(pages, { images: 'png' })],
    ['JPEG', () => onOptions({ label, pages, format: 'jpeg' })],
    ['WebP', () => onOptions({ label, pages, format: 'webp' })],
    ['JPEG XL', () => onOptions({ label, pages, format: 'jxl' })],
    [
      'PSD',
      () =>
        run(pages, {
          psd: {
            background: t('psd.background'),
            cleanup: t('psd.cleanup'),
            outline: t('psd.outline'),
          },
        }),
    ],
    ['CBZ', () => onOptions({ label, pages, format: 'cbz' })],
    ['PDF', () => run(pages, 'pdf')],
  ] as const

  return (
    <MenubarSub>
      <MenubarSubTrigger disabled={disabled || exporting} aria-busy={busy}>
        {busy && <LoaderCircle className='animate-spin' aria-hidden='true' />}
        {t(label)}
      </MenubarSubTrigger>
      <MenubarSubContent>
        {formats.map(([name, choose]) => (
          <MenubarItem key={name} disabled={exporting} onClick={choose}>
            {name}…
          </MenubarItem>
        ))}
      </MenubarSubContent>
    </MenubarSub>
  )
}

/** An export's options before its destination, as Photoshop's Export As asks for them: a CBZ's
 * image format, lossless mode for WebP/JPEG XL, and lossy quality. Saved to preferences on export
 * so defaults carry across sessions and CLI runs. */
function ExportDialog({
  options,
  open,
  onOpenChange,
}: {
  options: ExportOptions
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const { data: preferences } = usePreferences(open)
  const savePreferences = useSavePreferences()
  const { run } = useCommand(['export', options.label], commands.export, options.label)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className='sm:max-w-xs'>
        <DialogHeader>
          <DialogTitle>
            {t('export.title', {
              format: options.format === 'cbz' ? 'CBZ' : imageNames[options.format],
            })}
          </DialogTitle>
        </DialogHeader>
        {open &&
          (preferences ? (
            <ExportForm
              key={options.format}
              options={options}
              initial={preferences.export}
              saving={savePreferences.isPending}
              onCancel={() => onOpenChange(false)}
              onConfirm={async (nextExport, imageFormat) => {
                try {
                  await savePreferences.mutateAsync({ ...preferences, export: nextExport })
                  onOpenChange(false)
                  run(
                    options.pages,
                    options.format === 'cbz' ? { cbz: imageFormat } : { images: imageFormat },
                  )
                } catch {
                  // MutationCache.onError reports the failure toast; keep dialog open.
                }
              }}
            />
          ) : (
            <p className='py-4 text-[11px] text-muted-foreground'>{t('settings.loading')}</p>
          ))}
      </DialogContent>
    </Dialog>
  )
}

function ExportForm({
  options,
  initial,
  saving,
  onCancel,
  onConfirm,
}: {
  options: ExportOptions
  initial: ExportConfig
  saving: boolean
  onCancel: () => void
  onConfirm: (nextExport: ExportConfig, imageFormat: ImageFormat) => Promise<void>
}) {
  const { t } = useTranslation()
  const [cbzCodec, setCbzCodec] = useState<ImageCodec>(initial.cbz_codec ?? 'png')
  const [quality, setQuality] = useState(initial.quality ?? 90)
  const [lossless, setLossless] = useState(initial.lossless ?? false)
  const codec: ImageCodec = options.format === 'cbz' ? cbzCodec : options.format
  const supportsLossless = codec === 'webp' || codec === 'jxl'
  const showsQuality = codec === 'jpeg' || (supportsLossless && !lossless)
  const imageFormat: ImageFormat =
    codec === 'png'
      ? 'png'
      : codec === 'jpeg'
        ? { jpeg: { quality } }
        : codec === 'webp'
          ? { webp: { quality, lossless } }
          : { jxl: { quality, lossless } }

  return (
    <>
      <div className='grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-4 gap-y-3 text-[12px]'>
        {options.format === 'cbz' && (
          <>
            <span className='text-muted-foreground'>{t('export.imageFormat')}</span>
            <Choice
              value={cbzCodec}
              choices={imageNames}
              label={t('export.imageFormat')}
              onChange={setCbzCodec}
            />
          </>
        )}
        {supportsLossless && (
          <>
            <div className='min-w-0'>
              <span className='text-muted-foreground'>{t('export.lossless')}</span>
            </div>
            <div className='flex items-center justify-end'>
              <Switch
                aria-label={t('export.lossless')}
                checked={lossless}
                onCheckedChange={setLossless}
              />
            </div>
          </>
        )}
        {showsQuality && (
          <>
            <span className='text-muted-foreground'>{t('export.quality')}</span>
            <div className='flex min-w-0 items-center gap-3'>
              <Slider
                aria-label={t('export.quality')}
                min={1}
                max={100}
                step={1}
                value={quality}
                onValueChange={setQuality}
              />
              <span className='w-6 shrink-0 text-right tabular-nums'>{quality}</span>
            </div>
          </>
        )}
      </div>
      <DialogFooter>
        <Button variant='outline' onClick={onCancel}>
          {t('common.cancel')}
        </Button>
        <Button
          disabled={saving}
          aria-busy={saving}
          onClick={() =>
            void onConfirm(
              {
                cbz_codec: options.format === 'cbz' ? cbzCodec : (initial.cbz_codec ?? 'png'),
                quality,
                lossless: supportsLossless ? lossless : (initial.lossless ?? false),
              },
              imageFormat,
            )
          }
        >
          {saving && <LoaderCircle className='size-3.5 animate-spin' aria-hidden='true' />}
          {t('export.export')}
        </Button>
      </DialogFooter>
    </>
  )
}

/** A shortcut on a punctuation key, named by what the user's layout types on the key standing for
 * it; the menu mounts it as it opens, which reads the layout again. */
function KeyShortcut({ code }: { code: string }) {
  return <MenubarShortcut>{useKeyboard().label(code)}</MenubarShortcut>
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
