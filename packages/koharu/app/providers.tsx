'use client'

import { QueryClientProvider } from '@tanstack/react-query'
import { Channel } from '@tauri-apps/api/core'
import { useEffect, useRef, type ReactNode } from 'react'
import { I18nextProvider } from 'react-i18next'

import { StartupView } from '@/components/app/StartupView'
import { Updater } from '@/components/app/Updater'
import ClientOnly from '@/components/ClientOnly'
import { refreshModels, refreshPreferences } from '@/lib/backend'
import i18n from '@/lib/i18n'
import { pageKey, projectKey, queryClient, refresh } from '@/lib/queries'
import {
  receiveCanvasSelection,
  receiveDownload,
  receiveJob,
  receivePickedColor,
  receiveStartupState,
  rgbaToHex,
  useKoharuStore,
} from '@/lib/store'
import {
  commands,
  type CanvasEvent,
  type Job,
  type ProjectInfo,
  type Transfer,
} from '@koharu/bridge/protocol'
import { toast, Toaster } from '@koharu/ui/components/toast'
import { TooltipProvider } from '@koharu/ui/components/tooltip'

export function Providers({ children }: { children: ReactNode }) {
  const runtime = useRef({ active: false, bound: false })

  useEffect(() => {
    const lifecycle = runtime.current
    lifecycle.active = true
    if (!lifecycle.bound) {
      lifecycle.bound = true
      const channel = <T,>(receive: (value: T) => void) =>
        new Channel<T>((value) => {
          if (lifecycle.active) receive(value)
        })

      void refreshPreferences().catch(() => undefined)
      void refreshModels().catch(() => undefined)

      void commands
        .subscribe(
          channel<ProjectInfo | null>(receiveProject),
          channel<CanvasEvent>(receiveCanvas),
          channel<Job>(receiveJob),
          channel<Transfer>(receiveDownload),
        )
        .then((state) => {
          if (!lifecycle.active) return
          receiveProject(state.project)
          receiveStartupState(state)
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

/** The open project, after any change: another project's selection starts empty. */
function receiveProject(project: ProjectInfo | null): void {
  const previous = queryClient.getQueryData<ProjectInfo | null>(projectKey)
  queryClient.setQueryData(projectKey, project)
  if (previous?.name !== project?.name) {
    const store = useKoharuStore.getState()
    store.selectPages(project?.page ? [project.page] : [])
    store.selectLayers([])
  }
  if (project) void refresh(pageKey).catch(() => undefined)
  else queryClient.setQueryData(pageKey, null)
}

function receiveCanvas(event: CanvasEvent): void {
  const store = useKoharuStore.getState()
  switch (event.type) {
    case 'camera':
      useKoharuStore.setState({ camera: { zoom: event.zoom, x: event.x, y: event.y } })
      break
    case 'selection':
      receiveCanvasSelection(event.layers)
      break
    case 'color':
      receivePickedColor(rgbaToHex(event.color))
      break
    case 'brush_diameter':
      store.setBrush({ ...store.brush, diameter: event.diameter })
      break
    case 'notice':
      toast.add({ type: 'info', title: i18n.t(`canvas.notice.${event.notice}`) })
      break
  }
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
