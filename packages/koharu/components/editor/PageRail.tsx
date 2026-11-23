'use client'

import {
  DndContext,
  PointerSensor,
  pointerWithin,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
} from '@dnd-kit/core'
import { useMutation, useQuery } from '@tanstack/react-query'
import { observeElementRect, useVirtualizer } from '@tanstack/react-virtual'
import { FilePlus2, FolderOpen, LoaderCircle, MoreHorizontal, Settings } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useDebounce } from 'use-debounce'

import { send, useImportPages, usePage, useProject } from '@/lib/queries'
import { deletePages, movePage } from '@/lib/scene'
import { pipelineStages, useKoharuStore } from '@/lib/store'
import { commands, type ImportSource, type PageSummary, type Stage } from '@koharu/ipc/protocol'
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
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from '@koharu/ui/components/context-menu'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@koharu/ui/components/dropdown-menu'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

const emptyPages: PageSummary[] = []

export function PageRail() {
  const { t } = useTranslation()
  const project = useProject().data
  const pages = project?.pages ?? emptyPages
  const active = usePage().data?.id ?? null
  const selected = useKoharuStore((state) => state.selectedPages)
  const selectPages = useKoharuStore((state) => state.selectPages)
  const selectLayers = useKoharuStore((state) => state.selectLayers)
  const setSettingsOpen = useKoharuStore((state) => state.setSettingsOpen)
  const { importPages, importing } = useImportPages()
  const anchor = useRef<number | null>(null)
  // A page being dragged, and the place of the page it's over.
  const [dragged, setDragged] = useState<{ page: string; target: number | null } | null>(null)
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }))
  // The page Delete asks about, kept while its dialog closes so the dialog still names it.
  const [pageToDelete, setPageToDelete] = useState<{ page: string; number: number } | null>(null)
  const [confirmingDelete, setConfirmingDelete] = useState(false)
  const pageList = useRef<HTMLDivElement>(null)
  const pageVirtualizer = useVirtualizer({
    count: pages.length,
    getScrollElement: () => pageList.current,
    getItemKey: (index) => pages[index]?.id ?? index,
    estimateSize: () => 72,
    gap: 2,
    overscan: 6,
    initialRect: { width: 240, height: 600 },
    observeElementRect: (instance, callback) =>
      observeElementRect(instance, (rect) =>
        callback({ width: rect.width || 240, height: rect.height || 600 }),
      ),
  })

  const select = (index: number, additive: boolean, range: boolean) => {
    const page = pages[index]
    if (!page) return
    let next: string[]
    if (range && anchor.current !== null) {
      const start = Math.min(anchor.current, index)
      const end = Math.max(anchor.current, index)
      const rangeIds = pages.slice(start, end + 1).map((item) => item.id)
      next = additive ? [...new Set([...selected, ...rangeIds])] : rangeIds
    } else if (additive) {
      next = selected.includes(page.id)
        ? selected.filter((id) => id !== page.id)
        : [...selected, page.id]
      anchor.current = index
    } else {
      next = [page.id]
      anchor.current = index
    }
    selectPages(next)
    if (page.id !== active) {
      selectLayers([])
      send(commands.show, page.id, null)
    }
  }

  const deletion = useMutation({
    mutationFn: (page: string) => deletePages([page]),
    onSuccess: (_, page) => {
      selectPages(selected.filter((selectedPage) => selectedPage !== page))
      if (active === page) selectLayers([])
    },
  })

  /** A page dropped on another takes its place; `move_page` counts its index after the page is
   * taken out, which is that place either way. */
  const drop = (target: number | null) => {
    const page = dragged?.page
    setDragged(null)
    if (page !== undefined && target !== null && pages[target]?.id !== page) {
      send(movePage, page, target)
    }
  }

  return (
    <>
      <aside className='flex h-full min-h-0 flex-col bg-[var(--surface-sidebar)]'>
        <header className='flex h-10 shrink-0 items-center px-2.5'>
          <div className='flex min-w-0 items-center gap-2'>
            <h2 className='text-[11px] font-semibold'>{t('navigator.pages')}</h2>
            <span className='rounded-full bg-primary/[0.07] px-1.5 py-0.5 text-[9px] text-muted-foreground tabular-nums'>
              {pages.length}
            </span>
          </div>
          {/* One selected page behaves exactly like acting on the active
              page, so saying so would be noise. */}
          {selected.length > 1 && (
            <span
              role='status'
              aria-live='polite'
              className='ml-auto text-[9px] text-muted-foreground tabular-nums'
            >
              {t('navigator.selected', { count: selected.length })}
            </span>
          )}
        </header>

        {pages.length === 0 ? (
          <div className='flex min-h-0 flex-1 flex-col items-center justify-center px-4 text-center'>
            <p className='text-[11px] font-medium'>{t('navigator.emptyTitle')}</p>
            <p className='mt-1 text-[10px] leading-4 text-muted-foreground'>
              {t('navigator.emptyDescription')}
            </p>
            <div className='mt-3'>
              <PageImportMenu importing={importing} onImport={importPages} />
            </div>
          </div>
        ) : (
          <DndContext
            sensors={sensors}
            collisionDetection={pointerWithin}
            onDragStart={({ active }) => setDragged({ page: String(active.id), target: null })}
            onDragOver={({ over }) =>
              setDragged(
                (current) =>
                  current && {
                    ...current,
                    target: over ? pages.findIndex((page) => page.id === over.id) : null,
                  },
              )
            }
            onDragCancel={() => setDragged(null)}
            onDragEnd={({ over }) =>
              drop(over ? pages.findIndex((page) => page.id === over.id) : null)
            }
          >
            <ScrollArea
              className='min-h-0 flex-1'
              viewportClassName='px-1.5 py-1.5'
              viewportRef={pageList}
            >
              <div
                className='relative w-full touch-none'
                style={{ height: pageVirtualizer.getTotalSize() }}
              >
                {pageVirtualizer.getVirtualItems().map((virtualRow) => {
                  const page = pages[virtualRow.index]
                  if (!page) return null
                  const index = virtualRow.index
                  return (
                    <div
                      key={virtualRow.key}
                      data-index={virtualRow.index}
                      className='absolute top-0 left-0 w-full'
                      style={{ transform: `translateY(${virtualRow.start}px)` }}
                    >
                      <PageItem
                        page={page}
                        index={index}
                        revision={project?.revisions[page.id]}
                        active={active === page.id}
                        selected={selected.includes(page.id)}
                        dragged={dragged?.page === page.id}
                        target={dragged !== null && dragged.target === index}
                        onSelect={(additive, range) => select(index, additive, range)}
                        onRun={(stage) => send(commands.runPipeline, { pages: [page.id] }, [stage])}
                        onDelete={() => {
                          setPageToDelete({ page: page.id, number: index + 1 })
                          setConfirmingDelete(true)
                        }}
                      />
                    </div>
                  )
                })}
              </div>
            </ScrollArea>
          </DndContext>
        )}

        <div className='mt-auto flex shrink-0 items-center gap-2 px-1.5 pt-0.5 pb-1.5'>
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  type='button'
                  variant='ghost'
                  size='icon'
                  className='size-6 text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground'
                  aria-label={t('menu.settings')}
                  onClick={() => setSettingsOpen(true)}
                />
              }
            >
              <Settings className='size-3.5' />
            </TooltipTrigger>
            <TooltipContent side='right'>{t('menu.settings')}</TooltipContent>
          </Tooltip>
        </div>
      </aside>
      <AlertDialog open={confirmingDelete} onOpenChange={setConfirmingDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('navigator.deleteTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('navigator.deleteDescription', {
                page: t('navigator.pageNumber', { number: pageToDelete?.number }),
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('common.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              variant='destructive'
              onClick={() => {
                setConfirmingDelete(false)
                if (pageToDelete !== null) deletion.mutate(pageToDelete.page)
              }}
            >
              {t('navigator.delete')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

function PageImportMenu({
  importing,
  onImport,
}: {
  importing: boolean
  onImport: (source: ImportSource) => void
}) {
  const { t } = useTranslation()
  const icon = importing ? (
    <LoaderCircle className='size-3.5 animate-spin' />
  ) : (
    <FilePlus2 className='size-3.5' />
  )

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            size='sm'
            variant='outline'
            disabled={importing}
            aria-busy={importing}
            className='rounded-lg text-[10px]'
          />
        }
      >
        {icon}
        {importing ? t('navigator.importingAction') : t('navigator.importAction')}
      </DropdownMenuTrigger>
      <DropdownMenuContent align='start' className='w-auto min-w-20'>
        <DropdownMenuItem disabled={importing} onClick={() => onImport('files')}>
          <FilePlus2 />
          {t('navigator.importFiles')}
        </DropdownMenuItem>
        <DropdownMenuItem disabled={importing} onClick={() => onImport('folder')}>
          <FolderOpen />
          {t('navigator.importFolder')}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function PageItem({
  page,
  index,
  revision,
  active,
  selected,
  dragged,
  target,
  onSelect,
  onRun,
  onDelete,
}: {
  page: PageSummary
  index: number
  /** The project's revision when a change last named this page, so its thumbnail follows its
   * edits. */
  revision: string | undefined
  active: boolean
  selected: boolean
  dragged: boolean
  target: boolean
  onSelect: (additive: boolean, range: boolean) => void
  onRun: (stage: Stage) => void
  onDelete: () => void
}) {
  const { t } = useTranslation()
  const name = t('navigator.pageNumber', { number: index + 1 })
  const handle = useDraggable({ id: page.id })
  const { setNodeRef } = useDroppable({ id: page.id })
  // A press on the page's own buttons and menus doesn't drag it.
  const { onPointerDown, ...listeners } = handle.listeners ?? {}

  return (
    // A right-click on the page opens the menu its button does.
    <ContextMenu>
      <ContextMenuTrigger>
        <article
          ref={(element) => {
            handle.setNodeRef(element)
            setNodeRef(element)
          }}
          {...listeners}
          onPointerDown={(event) => {
            if (!(event.target as HTMLElement).closest('button,[role="menuitem"]')) {
              onPointerDown?.(event)
            }
          }}
          data-page={page.id}
          data-page-index={index}
          data-active={active}
          data-selected={selected}
          className={cn(
            'group grid cursor-default grid-cols-[48px_minmax(0,1fr)] gap-2.5 rounded-xl p-1.5 transition-colors select-none',
            active
              ? 'bg-primary/[0.09] hover:bg-primary/[0.09]'
              : selected
                ? 'bg-foreground/[0.06] hover:bg-foreground/[0.08]'
                : 'hover:bg-foreground/[0.045]',
            dragged && 'opacity-50',
            target && !dragged && 'ring-2 ring-primary/60',
          )}
          onClick={(event) => {
            if ((event.target as HTMLElement).closest('button,[role="menuitem"]')) return
            onSelect(event.ctrlKey || event.metaKey, event.shiftKey)
          }}
        >
          <div className='grid h-16 w-12 place-items-center overflow-hidden rounded-lg bg-[var(--surface-well)]'>
            {page.background ? (
              <PageThumbnail
                page={page.id}
                version={revision ?? page.background.file}
                label={name}
              />
            ) : (
              <span className='text-[9px] text-muted-foreground'>{t('navigator.noImage')}</span>
            )}
          </div>
          <div className='min-w-0 py-0.5'>
            <div className='flex items-start gap-1'>
              <span className='min-w-0 flex-1 truncate text-[10px] font-medium'>{name}</span>
              <DropdownMenu>
                <DropdownMenuTrigger
                  render={
                    <Button
                      variant='ghost'
                      size='icon-xs'
                      aria-label={t('navigator.actionsFor', { page: name })}
                      // Over the name's line, not below it, so the size sits close under the name.
                      className='-my-1.5 shrink-0 opacity-0 shadow-none group-hover:opacity-100 aria-expanded:opacity-100'
                    />
                  }
                >
                  <MoreHorizontal />
                </DropdownMenuTrigger>
                <DropdownMenuContent align='end' className={pageMenu}>
                  <PageActions
                    Item={DropdownMenuItem}
                    Separator={DropdownMenuSeparator}
                    onRun={onRun}
                    onDelete={onDelete}
                  />
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
            <p className='mt-1.5 text-[9px] text-muted-foreground tabular-nums'>
              {page.size.width} × {page.size.height}
            </p>
            {/* Nothing on a page with no text found yet. */}
            {page.texts > 0 && <Translated translated={page.translated} total={page.texts} />}
          </div>
        </article>
      </ContextMenuTrigger>
      <ContextMenuContent className={pageMenu}>
        <PageActions
          Item={ContextMenuItem}
          Separator={ContextMenuSeparator}
          onRun={onRun}
          onDelete={onDelete}
        />
      </ContextMenuContent>
    </ContextMenu>
  )
}

/** How many of a page's texts are translated, as a ring and a count, as a CAT tool's file list
 * shows each file's progress; the sentence it stands for is its label and its tooltip. */
function Translated({ translated, total }: { translated: number; total: number }) {
  const { t } = useTranslation()
  const label = t('navigator.translated', { translated, total })
  return (
    <Tooltip>
      <TooltipTrigger
        render={<span role='img' aria-label={label} />}
        className='mt-1 flex w-fit items-center gap-1 text-[9px] text-muted-foreground tabular-nums'
      >
        <svg viewBox='0 0 16 16' aria-hidden='true' className='size-3 -rotate-90'>
          <circle
            cx='8'
            cy='8'
            r='6.5'
            fill='none'
            strokeWidth='2'
            className='stroke-foreground/10'
          />
          {/* A path length of 1, so the dash is the translated share of the ring. */}
          <circle
            cx='8'
            cy='8'
            r='6.5'
            fill='none'
            strokeWidth='2'
            pathLength={1}
            strokeDasharray={`${translated / total} 1`}
            className='stroke-primary'
          />
        </svg>
        {translated} / {total}
      </TooltipTrigger>
      <TooltipContent side='right'>{label}</TooltipContent>
    </Tooltip>
  )
}

/** A compact menu, as the rail's import menu is. */
const pageMenu = 'w-auto min-w-20 border border-border/50 p-0.5 shadow-sm ring-0'
const pageMenuItem =
  'min-h-7 gap-1 px-1.5 py-0.5 text-[11px] [&_svg:not([class*="size-"])]:size-3.5'

/** A page's actions, as the items of the menu `Item` and `Separator` belong to: each pipeline
 * stage on the page alone, in the pipeline's order, then deleting it. */
function PageActions({
  Item,
  Separator,
  onRun,
  onDelete,
}: {
  Item: typeof DropdownMenuItem | typeof ContextMenuItem
  Separator: typeof DropdownMenuSeparator | typeof ContextMenuSeparator
  onRun: (stage: Stage) => void
  onDelete: () => void
}) {
  const { t } = useTranslation()
  return (
    <>
      {pipelineStages.map((stage) => (
        <Item key={stage} className={pageMenuItem} onClick={() => onRun(stage)}>
          {t(`navigator.run.${stage}`)}
        </Item>
      ))}
      <Separator />
      <Item variant='destructive' className={pageMenuItem} onClick={onDelete}>
        {t('navigator.delete')}
      </Item>
    </>
  )
}

/** The page as it renders, fetched again when `version` changes. */
function PageThumbnail({ page, version, label }: { page: string; version: string; label: string }) {
  const { t } = useTranslation()
  const [source, setSource] = useState<string | null>(null)
  // Read once the page settles, not on every change of a stroke or a drag.
  const [settled] = useDebounce(version, 100)

  const thumbnail = useQuery({
    queryKey: ['thumbnail', page, settled],
    queryFn: async () => new Uint8Array(await commands.getPageThumbnail(page)),
    staleTime: Number.POSITIVE_INFINITY,
    notifyOnChangeProps: ['data'],
  }).data

  useEffect(() => {
    if (!thumbnail) return
    const url = URL.createObjectURL(new Blob([thumbnail.buffer], { type: 'image/webp' }))
    setSource(url)
    return () => {
      URL.revokeObjectURL(url)
    }
  }, [thumbnail])

  return (
    <div className='grid size-full place-items-center'>
      {source ? (
        // eslint-disable-next-line @next/next/no-img-element
        <img src={source} alt={label} draggable={false} className='size-full object-contain' />
      ) : (
        <span className='text-[9px] text-muted-foreground'>{t('common.loading')}</span>
      )}
    </div>
  )
}
