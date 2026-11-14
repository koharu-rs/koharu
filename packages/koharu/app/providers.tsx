'use client'

import { QueryClientProvider } from '@tanstack/react-query'
import { useEffect, useRef, type ReactNode } from 'react'
import { I18nextProvider } from 'react-i18next'

import { StartupView } from '@/components/app/StartupView'
import { Updater } from '@/components/app/Updater'
import ClientOnly from '@/components/ClientOnly'
import { refreshModels, refreshPreferences } from '@/lib/backend'
import i18n from '@/lib/i18n'
import { pageKey, projectKey, queryClient, refresh, type ProjectView } from '@/lib/queries'
import {
  receiveCanvasSelection,
  receiveDownload,
  receivePickedColor,
  receivePipeline,
  receiveRuns,
  rgbaToHex,
  useKoharuStore,
} from '@/lib/store'
import { commands, events, type Change, type PageId } from '@koharu/bridge/protocol'
import { toast, Toaster } from '@koharu/ui/components/toast'
import { TooltipProvider } from '@koharu/ui/components/tooltip'

export function Providers({ children }: { children: ReactNode }) {
  const runtime = useRef({ active: false, bound: false })

  useEffect(() => {
    const lifecycle = runtime.current
    lifecycle.active = true
    if (!lifecycle.bound) {
      lifecycle.bound = true
      const listen = <T,>(
        event: (handler: (payload: T) => void) => Promise<unknown>,
        receive: (payload: T) => void,
      ) =>
        event((payload) => {
          if (lifecycle.active) receive(payload)
        })

      void refreshPreferences().catch(() => undefined)
      void refreshModels().catch(() => undefined)

      // Listening first, so nothing told while the runs load is missed.
      void Promise.all([
        listen(events.change, receiveChange),
        listen(events.project, receiveProject),
        listen(events.page, receivePage),
        listen(events.selection, receiveCanvasSelection),
        listen(events.camera, (camera) => useKoharuStore.setState({ camera })),
        listen(events.color, (color) => receivePickedColor(rgbaToHex(color))),
        listen(events.brushDiameter, (diameter) => {
          const store = useKoharuStore.getState()
          store.setBrush({ ...store.brush, diameter })
        }),
        listen(events.notice, (notice) =>
          toast.add({ type: 'info', title: i18n.t(`canvas.notice.${notice}`) }),
        ),
        listen(events.pipeline, receivePipeline),
        listen(events.download, receiveDownload),
      ])
        .then(() => commands.getRuns())
        .then((runs) => {
          if (lifecycle.active) receiveRuns(runs)
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

/** Another project opened, or the open one closed: its selection starts empty. A project told
 * again, after changes the interface missed, is read again. */
function receiveProject(name: string | null): void {
  const previous = queryClient.getQueryData<ProjectView | null>(projectKey)
  if (previous?.name !== name) {
    const store = useKoharuStore.getState()
    store.selectPages([])
    store.selectLayers([])
  }
  if (name === null) {
    queryClient.setQueryData(projectKey, null)
    queryClient.setQueryData(pageKey, null)
  } else {
    void refresh(projectKey, pageKey).catch(() => undefined)
  }
}

/** The page the canvas shows. */
function receivePage(page: PageId | null): void {
  queryClient.setQueryData<ProjectView | null>(projectKey, (project) =>
    project ? { ...project, page } : project,
  )
  if (page !== null && useKoharuStore.getState().selectedPages.length === 0) {
    useKoharuStore.getState().selectPages([page])
  }
  void refresh(pageKey).catch(() => undefined)
}

/** An edit, undo, or redo, from whichever writer: the project and its shown page are read again. */
function receiveChange(change: Change): void {
  queryClient.setQueryData<ProjectView | null>(projectKey, (project) =>
    project ? { ...project, history: change.history, revision: change.revision } : project,
  )
  void refresh(projectKey, pageKey).catch(() => undefined)
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
