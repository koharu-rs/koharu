'use client'

import { Folder, FolderPlus, Plus, Settings, Trash2 } from 'lucide-react'
import { useCallback, useEffect, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { pageKey, pagesKey, projectKey, refresh } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { commands, type ProjectSummary } from '@koharu/bridge/protocol'
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
import { Input } from '@koharu/ui/components/input'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

export function StartView() {
  const { t } = useTranslation()
  const setSettingsOpen = useKoharuStore((state) => state.setSettingsOpen)
  const [projects, setProjects] = useState<ProjectSummary[]>([])
  const [name, setName] = useState('')
  const [busy, setBusy] = useState<string | null>('list')
  const [projectToDelete, setProjectToDelete] = useState<string | null>(null)

  const reload = useCallback(async () => {
    setBusy('list')
    try {
      setProjects(await call(commands.listProjects))
    } finally {
      setBusy(null)
    }
  }, [])

  useEffect(() => {
    void reload().catch(() => undefined)
  }, [reload])

  const createProject = async (event: FormEvent) => {
    event.preventDefault()
    const projectName = name.trim()
    if (!projectName || busy) return
    setBusy('create')
    try {
      await call(commands.createProject, projectName)
      await refresh(projectKey, pagesKey, pageKey)
      setName('')
    } finally {
      setBusy(null)
    }
  }

  const openProject = async (projectName: string) => {
    if (busy) return
    setBusy(projectName)
    try {
      await call(commands.openProject, projectName)
      await refresh(projectKey, pagesKey, pageKey)
    } finally {
      setBusy(null)
    }
  }

  const deleteProject = async () => {
    const projectName = projectToDelete
    if (busy || !projectName) return
    setBusy(projectName)
    try {
      await call(commands.deleteProject, projectName)
      await reload()
      setProjectToDelete(null)
    } finally {
      setBusy(null)
    }
  }

  return (
    <AlertDialog
      open={projectToDelete !== null}
      onOpenChange={(open) => {
        if (!open && busy === null) setProjectToDelete(null)
      }}
    >
      <main className='min-h-0 flex-1 overflow-hidden bg-[var(--surface-canvas)]'>
        <ScrollArea className='size-full'>
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

            <div className='mt-7 grid min-h-[390px] overflow-hidden rounded-2xl border border-border/80 bg-[var(--surface-panel)] md:grid-cols-[300px_minmax(0,1fr)]'>
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
                    disabled={busy !== null}
                    className='h-9 bg-background text-[11px]'
                  />
                  <Button
                    type='submit'
                    size='sm'
                    className='h-9 justify-center gap-1.5 text-[11px]'
                    disabled={!name.trim() || busy !== null}
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
                </header>

                <ScrollArea
                  className='min-h-0 flex-1'
                  viewportClassName='p-2'
                  aria-busy={busy === 'list'}
                >
                  {busy === 'list' && projects.length === 0 ? (
                    <p className='grid min-h-full place-items-center text-[11px] text-muted-foreground'>
                      {t('start.loadingProjects')}
                    </p>
                  ) : projects.length === 0 ? (
                    <div
                      className='grid min-h-full place-items-center px-6 text-center'
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
                  ) : (
                    <ul className='grid gap-0.5' aria-label={t('start.projectList')}>
                      {projects.map((project) => (
                        <li
                          key={project.name}
                          className='group flex min-w-0 items-center rounded-lg hover:bg-foreground/[0.045]'
                        >
                          <button
                            type='button'
                            className='flex min-w-0 flex-1 items-center gap-3 rounded-lg px-3 py-2.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset'
                            onClick={() => void openProject(project.name).catch(() => undefined)}
                            disabled={busy !== null}
                          >
                            <span className='grid size-8 shrink-0 place-items-center rounded-lg bg-accent text-accent-foreground'>
                              <Folder className='size-4' />
                            </span>
                            <span className='min-w-0'>
                              <span className='block truncate text-[11px] font-medium'>
                                {project.name}
                              </span>
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
                            aria-label={t('start.deleteLabel', { name: project.name })}
                            onClick={() => setProjectToDelete(project.name)}
                            disabled={busy !== null}
                          >
                            <Trash2 className='size-3.5' />
                          </Button>
                        </li>
                      ))}
                    </ul>
                  )}
                </ScrollArea>
              </section>
            </div>
          </section>
        </ScrollArea>
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
          <AlertDialogCancel disabled={busy !== null}>{t('common.cancel')}</AlertDialogCancel>
          <AlertDialogAction
            variant='destructive'
            disabled={busy !== null}
            aria-busy={busy !== null}
            onClick={() => void deleteProject().catch(() => undefined)}
          >
            {busy !== null ? t('start.deleting') : t('start.deleteAction')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
