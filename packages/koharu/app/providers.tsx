'use client'

import { QueryClientProvider } from '@tanstack/react-query'
import { useEffect, useRef, type ReactNode } from 'react'
import { I18nextProvider } from 'react-i18next'

import { StartupView } from '@/components/app/StartupView'
import { Updater } from '@/components/app/Updater'
import ClientOnly from '@/components/ClientOnly'
import { hear } from '@/lib/events'
import i18n from '@/lib/i18n'
import { queryClient } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { Toaster } from '@koharu/ui/components/toast'
import { TooltipProvider } from '@koharu/ui/components/tooltip'

export function Providers({ children }: { children: ReactNode }) {
  const runtime = useRef({ active: false, bound: false })

  useEffect(() => {
    const lifecycle = runtime.current
    lifecycle.active = true
    if (!lifecycle.bound) {
      lifecycle.bound = true
      void hear(() => lifecycle.active).catch(() => undefined)
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
    // The webview is the app's window, not a browser tab: it applies no browser zoom, keeping
    // Ctrl+wheel for the app's handlers, and opens no browser menu on a right-click, where the
    // app shows its own menu or none. A text field keeps its edit menu, as Clip Studio Paint's do.
    const preventViewportScaling = (event: WheelEvent) => {
      if (event.ctrlKey) event.preventDefault()
    }
    const preventBrowserMenu = (event: MouseEvent) => {
      if (!(event.target instanceof Element && event.target.matches(':read-write'))) {
        event.preventDefault()
      }
    }

    window.addEventListener('wheel', preventViewportScaling, { capture: true, passive: false })
    window.addEventListener('contextmenu', preventBrowserMenu)
    return () => {
      window.removeEventListener('wheel', preventViewportScaling, { capture: true })
      window.removeEventListener('contextmenu', preventBrowserMenu)
    }
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
