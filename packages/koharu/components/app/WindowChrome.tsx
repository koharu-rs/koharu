'use client'

import { getCurrentWindow } from '@tauri-apps/api/window'
import { Copy, Minus, Square, X } from 'lucide-react'
import { useEffect, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { isTauriRuntime } from '@koharu/bridge/runtime'

const resizeHandles = [
  { direction: 'North', className: 'top-0 right-2 left-2 h-1 cursor-n-resize' },
  { direction: 'South', className: 'right-2 bottom-0 left-2 h-1 cursor-s-resize' },
  { direction: 'East', className: 'top-2 right-0 bottom-2 w-1 cursor-e-resize' },
  { direction: 'West', className: 'top-2 bottom-2 left-0 w-1 cursor-w-resize' },
  { direction: 'NorthEast', className: 'top-0 right-0 size-2 cursor-ne-resize' },
  { direction: 'NorthWest', className: 'top-0 left-0 size-2 cursor-nw-resize' },
  { direction: 'SouthEast', className: 'right-0 bottom-0 size-2 cursor-se-resize' },
  { direction: 'SouthWest', className: 'bottom-0 left-0 size-2 cursor-sw-resize' },
] as const

export function useMacOS() {
  const [macOS, setMacOS] = useState(false)

  useEffect(() => {
    setMacOS(
      navigator.userAgent.includes('Macintosh') ||
        navigator.platform.toLowerCase().startsWith('mac'),
    )
  }, [])

  return macOS
}

export function WindowControls() {
  const { t } = useTranslation()
  const native = isTauriRuntime()
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    if (!native) return

    const window = getCurrentWindow()
    let disposed = false
    let unlisten: (() => void) | undefined
    const synchronize = () => {
      void window.isMaximized().then((value) => {
        if (!disposed) setMaximized(value)
      })
    }
    synchronize()
    queueMicrotask(() => {
      if (disposed) return
      void window.onResized(synchronize).then((stop) => {
        if (disposed) void Promise.resolve(stop()).catch(() => undefined)
        else unlisten = stop
      })
    })
    return () => {
      disposed = true
      if (unlisten) void Promise.resolve(unlisten()).catch(() => undefined)
    }
  }, [native])

  const toggleMaximize = async () => {
    if (!native) return
    const window = getCurrentWindow()
    await window.toggleMaximize()
    setMaximized(await window.isMaximized())
  }

  return (
    <>
      {!maximized && native && <WindowResizeHandles />}
      <div className='flex h-full shrink-0'>
        <WindowButton
          label={t('window.minimize')}
          onClick={() => {
            if (native) void getCurrentWindow().minimize()
          }}
        >
          <Minus />
        </WindowButton>
        <WindowButton
          label={t(maximized ? 'window.restore' : 'window.maximize')}
          onClick={() => void toggleMaximize()}
        >
          {maximized ? <Copy /> : <Square />}
        </WindowButton>
        <WindowButton
          label={t('window.close')}
          className='hover:text-destructive-foreground hover:bg-destructive'
          onClick={() => {
            if (native) void getCurrentWindow().close()
          }}
        >
          <X />
        </WindowButton>
      </div>
    </>
  )
}

function WindowResizeHandles() {
  const startResize =
    (direction: (typeof resizeHandles)[number]['direction']) =>
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return
      event.preventDefault()
      event.stopPropagation()
      void getCurrentWindow()
        .startResizeDragging(direction)
        .catch(() => undefined)
    }

  return resizeHandles.map(({ direction, className }) => (
    <div
      key={direction}
      aria-hidden='true'
      data-window-resize-handle={direction}
      className={`window-resize-handle fixed z-50 ${className}`}
      onPointerDown={startResize(direction)}
    />
  ))
}

function WindowButton({
  label,
  className = '',
  children,
  onClick,
}: {
  label: string
  className?: string
  children: ReactNode
  onClick: () => void
}) {
  return (
    <button
      type='button'
      aria-label={label}
      className={`grid h-full w-11 place-items-center text-muted-foreground transition-colors hover:bg-primary/10 hover:text-foreground [&_svg]:size-3.5 ${className}`}
      onClick={onClick}
    >
      {children}
    </button>
  )
}
