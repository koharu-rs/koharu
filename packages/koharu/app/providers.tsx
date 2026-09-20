'use client'

import { QueryClientProvider } from '@tanstack/react-query'
import { useEffect, useRef, type ReactNode } from 'react'
import { I18nextProvider } from 'react-i18next'

import { StartupView } from '@/components/app/StartupView'
import { Updater } from '@/components/app/Updater'
import ClientOnly from '@/components/ClientOnly'
import { refreshTranslationModels } from '@/lib/backend'
import i18n from '@/lib/i18n'
import { glossaryKey, pageKey, pagesKey, projectKey, queryClient, refresh } from '@/lib/queries'
import {
  receiveCanvas,
  receiveDownload,
  receiveStartupState,
  receiveJob,
  receiveResources,
  useKoharuStore,
} from '@/lib/store'
import { Channel, commands } from '@koharu/bridge'
import type {
  CanvasState,
  Download,
  Job,
  ModelResources,
  ProjectInfo,
} from '@koharu/bridge/protocol'
import { Toaster } from '@koharu/ui/components/toast'
import { TooltipProvider } from '@koharu/ui/components/tooltip'

export function Providers({ children }: { children: ReactNode }) {
  const runtime = useRef({ active: false, bound: false })

  useEffect(() => {
    const lifecycle = runtime.current
    lifecycle.active = true
    if (!lifecycle.bound) {
      lifecycle.bound = true
      const observedJobs = new Map<string, Job>()
      const pendingJobs: Job[] = []
      let jobsReady = false
      const receiveObservedJob = (job: Job) => {
        const previous = observedJobs.get(job.id)
        observedJobs.set(job.id, job)
        receiveJob(job)
        if (job.completed > (previous?.completed ?? 0) || job.state !== 'running') {
          void refresh(projectKey, pagesKey, pageKey).catch(() => undefined)
        }
        if (
          previous?.state === 'running' &&
          job.state !== 'running' &&
          (job.kind === 'glossary_scan' || job.kind === 'glossary_translation')
        ) {
          const projectName = queryClient.getQueryData<ProjectInfo | null>(projectKey)?.name
          if (projectName) {
            void queryClient.invalidateQueries({
              queryKey: glossaryKey(projectName),
              exact: true,
            })
          }
        }
      }
      const channel = <T,>(receive: (value: T) => void) =>
        new Channel<T>((value) => {
          if (lifecycle.active) receive(value)
        })

      void refreshTranslationModels().catch(() => undefined)

      void commands
        .subscribe(
          channel<CanvasState>(receiveCanvas),
          channel<Job>((job) => {
            if (jobsReady) receiveObservedJob(job)
            else pendingJobs.push(job)
          }),
          channel<Download>(receiveDownload),
          channel<ModelResources>(receiveResources),
          channel<ProjectInfo | null>((project) => {
            const previous = queryClient.getQueryData<ProjectInfo | null>(projectKey)
            if (previous?.name !== project?.name) {
              observedJobs.clear()
              pendingJobs.splice(0)
            }
            if (previous?.name && previous.name !== project?.name) {
              const previousKey = glossaryKey(previous.name)
              void queryClient.cancelQueries({ queryKey: previousKey, exact: true }).then(() => {
                if (
                  queryClient.getQueryData<ProjectInfo | null>(projectKey)?.name !== previous.name
                ) {
                  queryClient.removeQueries({ queryKey: previousKey, exact: true })
                }
              })
            }
            queryClient.setQueryData(projectKey, project)
            if (previous?.name !== project?.name || previous?.revision !== project?.revision) {
              if (project?.name) {
                void queryClient.invalidateQueries({
                  queryKey: glossaryKey(project.name),
                  exact: true,
                })
              }
            }
            if (previous?.name !== project?.name) {
              const store = useKoharuStore.getState()
              store.selectPages(project?.active_page ? [project.active_page] : [])
              store.selectLayers([])
            }
            if (project) {
              void refresh(pagesKey, pageKey).catch(() => undefined)
            } else {
              queryClient.setQueryData(pagesKey, [])
              queryClient.setQueryData(pageKey, null)
            }
          }),
        )
        .then((state) => {
          observedJobs.clear()
          for (const job of state.jobs) observedJobs.set(job.id, job)
          jobsReady = true
          if (!lifecycle.active) return
          receiveStartupState(state)
          for (const job of pendingJobs.splice(0)) receiveObservedJob(job)
        })
        .catch(() => undefined)
    }

    return () => {
      lifecycle.active = false
    }
  }, [])

  useEffect(() => {
    const setLanguage = (language: string) => {
      document.documentElement.lang = language
    }
    setLanguage(i18n.language)
    i18n.on('languageChanged', setLanguage)
    return () => i18n.off('languageChanged', setLanguage)
  }, [])

  useEffect(() => {
    // Prevent the host webview from applying browser zoom; keep Ctrl+wheel for app handlers.
    const preventViewportScaling = (event: WheelEvent) => {
      if (event.ctrlKey) event.preventDefault()
    }

    window.addEventListener('wheel', preventViewportScaling, { capture: true, passive: false })
    return () => window.removeEventListener('wheel', preventViewportScaling, { capture: true })
  }, [])

  return (
    <QueryClientProvider client={queryClient}>
      <I18nextProvider i18n={i18n}>
        <TooltipProvider delay={0}>
          <ClientOnly>
            <StartupBoundary>{children}</StartupBoundary>
            <Toaster />
          </ClientOnly>
        </TooltipProvider>
      </I18nextProvider>
    </QueryClientProvider>
  )
}

function StartupBoundary({ children }: { children: ReactNode }) {
  const initialized = useKoharuStore((state) => state.initialized)
  return initialized ? (
    <>
      {children}
      <Updater />
    </>
  ) : (
    <StartupView />
  )
}

export default Providers
