'use client'

import { useMutation } from '@tanstack/react-query'
import { useVirtualizer } from '@tanstack/react-virtual'
import { Folder, FolderPlus, Plus, Settings, Trash2 } from 'lucide-react'
import { useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { ListSearch } from '@/components/controls/PickerSearch'
import { pageKey, projectKey, projectsKey, refresh, useProjects } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { commands } from '@koharu/ipc/protocol'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
} from '@koharu/ui/components/alert-dialog'
import { Button } from '@koharu/ui/components/button'
import { useComboboxFilter } from '@koharu/ui/components/combobox'
import { Input } from '@koharu/ui/components/input'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

export function StartView() {
  const { t } = useTranslation()
  const setSettingsOpen = useKoharuStore((state) => state.setSettingsOpen)
  const list = useProjects()
  const projects = list.data ?? []
  const [query, setQuery] = useState('')
  const filter = useComboboxFilter()
  const matches = projects.filter((project) => filter.contains(project, query))
  const [name, setName] = useState('')
  const [projectToDelete, setProjectToDelete] = useState<string | null>(null)
  const creation = useMutation({
    mutationFn: (projectName: string) => commands.create(projectName),
    onSuccess: async () => {
      await refresh(projectKey, pageKey)
      setName('')
    },
  })
  const opening = useMutation({
    mutationFn: (projectName: string) => commands.open(projectName),
    onSuccess: () => refresh(projectKey, pageKey),
  })
  const deletion = useMutation({
    mutationFn: (projectName: string) => commands.deleteProject(projectName),
    onSuccess: async () => {
      await refresh(projectsKey)
      setProjectToDelete(null)
    },
  })
  const busy = list.isFetching || creation.isPending || opening.isPending || deletion.isPending

  const createProject = (event: FormEvent) => {
    event.preventDefault()
    const projectName = name.trim()
    if (projectName && !busy) creation.mutate(projectName)
  }

  return (
    <AlertDialog
      open={projectToDelete !== null}
      onOpenChange={(open) => {
        if (!open && !busy) setProjectToDelete(null)
      }}
    >
      {/* The page scrolls only in a window too short for the panel, which otherwise fills the
          window, its list scrolling inside it. */}
      <main className='min-h-0 flex-1 overflow-y-auto bg-[var(--surface-canvas)]'>
        <section
          className='mx-auto flex min-h-full w-full max-w-[960px] flex-col px-6 py-10 sm:px-10 sm:py-14'
          aria-labelledby='start-title'
        >
          <header className='flex items-start justify-between gap-6'>
            <div>
              <h1 id='start-title' className='text-[24px] font-semibold tracking-[-0.03em]'>
                {t('start.title')}
              </h1>
              <p className='mt-1 text-[12px] leading-5 text-muted-foreground'>
                {t('start.description')}
              </p>
            </div>
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    type='button'
                    variant='ghost'
                    size='icon-sm'
                    className='size-8 shrink-0 text-muted-foreground hover:bg-foreground/[0.05] hover:text-foreground'
                    aria-label={t('menu.settings')}
                    onClick={() => setSettingsOpen(true)}
                  />
                }
              >
                <Settings className='size-4' />
              </TooltipTrigger>
              <TooltipContent side='bottom'>{t('menu.settings')}</TooltipContent>
            </Tooltip>
          </header>

          {/* A 0 px basis, not flex-1's 0 %, which a column of no set height reads as the
              panel's content, so the panel takes the window's height and its list scrolls. */}
          <div className='mt-7 grid min-h-[390px] grow basis-0 grid-rows-[auto_minmax(0,1fr)] overflow-hidden rounded-2xl border border-border/80 bg-[var(--surface-panel)] md:grid-cols-[300px_minmax(0,1fr)] md:grid-rows-[minmax(0,1fr)]'>
            <aside className='flex flex-col border-b border-border/80 bg-[var(--surface-titlebar)]/55 p-6 md:border-r md:border-b-0'>
              <span className='grid size-10 place-items-center rounded-xl bg-accent text-accent-foreground'>
                <FolderPlus className='size-[18px]' />
              </span>
              <h2 className='mt-5 text-[15px] font-semibold tracking-[-0.02em]'>
                {t('start.newProject')}
              </h2>
              <p className='mt-1 max-w-[30ch] text-[11px] leading-[1.6] text-muted-foreground'>
                {t('start.newDescription')}
              </p>

              <form className='mt-6 grid gap-2.5' onSubmit={createProject}>
                <label htmlFor='project-name' className='text-[10px] font-medium text-foreground'>
                  {t('start.projectName')}
                </label>
                <Input
                  id='project-name'
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder={t('start.projectNamePlaceholder')}
                  aria-label={t('start.projectName')}
                  autoComplete='off'
                  disabled={busy}
                  className='h-9 bg-background text-[11px]'
                />
                <Button
                  type='submit'
                  size='sm'
                  className='h-9 justify-center gap-1.5 text-[11px]'
                  disabled={!name.trim() || busy}
                >
                  <Plus className='size-3.5' />
                  {t('start.create')}
                </Button>
              </form>

              <p className='mt-auto pt-8 text-[10px] leading-4 text-muted-foreground'>
                {t('start.storageHint')}
              </p>
            </aside>

            <section
              className='flex min-h-[320px] min-w-0 flex-col'
              aria-labelledby='project-list-title'
            >
              <header className='flex h-14 shrink-0 items-center border-b border-border/70 px-5'>
                <h2 id='project-list-title' className='text-[12px] font-semibold'>
                  {t('start.yourProjects')}
                </h2>
                <span className='ml-2 rounded-full bg-muted px-2 py-0.5 text-[9px] text-muted-foreground tabular-nums'>
                  {projects.length}
                </span>
                {projects.length > 0 && (
                  <ListSearch
                    query={query}
                    label={t('start.search')}
                    className='ml-auto w-48'
                    onChange={setQuery}
                  />
                )}
              </header>

              {/* Outside the scroll area, whose content takes only its children's
                  height, so these fill the space under the header; a bottom padding
                  as tall as the header centres them on the whole panel. */}
              {list.isFetching && projects.length === 0 ? (
                <p
                  className='grid flex-1 place-items-center pb-14 text-[11px] text-muted-foreground'
                  aria-busy
                >
                  {t('start.loadingProjects')}
                </p>
              ) : projects.length === 0 ? (
                <div
                  className='grid flex-1 place-items-center px-6 pb-14 text-center'
                  role='status'
                >
                  <div>
                    <span className='mx-auto grid size-11 place-items-center rounded-xl bg-muted text-muted-foreground'>
                      <Folder className='size-[18px]' />
                    </span>
                    <p className='mt-3 text-[12px] font-medium'>{t('start.emptyTitle')}</p>
                    <p className='mt-1 text-[10px] leading-4 text-muted-foreground'>
                      {t('start.emptyDescription')}
                    </p>
                  </div>
                </div>
              ) : matches.length === 0 ? (
                <p
                  className='grid flex-1 place-items-center px-6 pb-14 text-center text-[11px] text-muted-foreground'
                  role='status'
                >
                  {t('start.noResults')}
                </p>
              ) : (
                // Anew for each query, so a filtered list starts at its top.
                <ProjectList
                  key={query}
                  projects={matches}
                  fetching={list.isFetching}
                  busy={busy}
                  onOpen={(project) => !busy && opening.mutate(project)}
                  onDelete={setProjectToDelete}
                />
              )}
            </section>
          </div>
        </section>
      </main>

      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogMedia className='bg-destructive/10 text-destructive'>
            <Trash2 className='size-5' />
          </AlertDialogMedia>
          <AlertDialogTitle>{t('start.deleteTitle')}</AlertDialogTitle>
          <AlertDialogDescription>
            {t('start.deleteDescription', { name: projectToDelete })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>{t('common.cancel')}</AlertDialogCancel>
          <AlertDialogAction
            variant='destructive'
            disabled={busy}
            aria-busy={busy}
            onClick={() => projectToDelete && !busy && deletion.mutate(projectToDelete)}
          >
            {busy ? t('start.deleting') : t('start.deleteAction')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

const rowHeight = 52

/** The projects, each opening on a click. Only the rows in view exist, as a library can hold
 * thousands of projects. */
function ProjectList({
  projects,
  fetching,
  busy,
  onOpen,
  onDelete,
}: {
  projects: string[]
  fetching: boolean
  busy: boolean
  onOpen: (project: string) => void
  onDelete: (project: string) => void
}) {
  const { t } = useTranslation()
  const viewport = useRef<HTMLDivElement>(null)
  const virtualizer = useVirtualizer({
    count: projects.length,
    getScrollElement: () => viewport.current,
    getItemKey: (index) => projects[index] ?? index,
    estimateSize: () => rowHeight,
    gap: 2,
    paddingStart: 8,
    paddingEnd: 8,
    overscan: 8,
  })

  return (
    <ScrollArea
      viewportRef={viewport}
      className='min-h-0 flex-1'
      viewportClassName='px-2'
      aria-busy={fetching}
    >
      <ul
        aria-label={t('start.projectList')}
        className='relative'
        style={{ height: virtualizer.getTotalSize() }}
      >
        {virtualizer.getVirtualItems().map((row) => {
          const project = projects[row.index]
          if (project === undefined) return null
          return (
            <li
              key={row.key}
              aria-setsize={projects.length}
              aria-posinset={row.index + 1}
              className='group absolute inset-x-0 top-0 flex h-13 min-w-0 items-center rounded-lg hover:bg-foreground/[0.045]'
              style={{ transform: `translateY(${row.start}px)` }}
            >
              <button
                type='button'
                className='flex h-full min-w-0 flex-1 items-center gap-3 rounded-lg px-3 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset'
                onClick={() => onOpen(project)}
                disabled={busy}
              >
                <span className='grid size-8 shrink-0 place-items-center rounded-lg bg-accent text-accent-foreground'>
                  <Folder className='size-4' />
                </span>
                <span className='min-w-0'>
                  <span className='block truncate text-[11px] font-medium'>{project}</span>
                  <span className='mt-0.5 block text-[9px] text-muted-foreground'>
                    {t('start.projectKind')}
                  </span>
                </span>
              </button>
              <Button
                type='button'
                size='icon-sm'
                variant='ghost'
                className='mr-2 size-7 shrink-0 text-muted-foreground opacity-0 group-hover:opacity-100 hover:text-foreground focus-visible:opacity-100'
                aria-label={t('start.deleteLabel', { name: project })}
                onClick={() => onDelete(project)}
                disabled={busy}
              >
                <Trash2 className='size-3.5' />
              </Button>
            </li>
          )
        })}
      </ul>
    </ScrollArea>
  )
}
