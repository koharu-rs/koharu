'use client'

import { autoUpdate, flip, offset, shift, useFloating } from '@floating-ui/react-dom'
import { useLayoutEffect, type ComponentProps } from 'react'

import { useKoharuStore } from '@/lib/store'
import type { Bounds } from '@koharu/ipc/protocol'
import { cn } from '@koharu/ui/lib/utils'

/** The space between the bar and what it's beside, and the canvas's edges, in CSS pixels. */
const GAP = 8

/** A bar beside a selection on the canvas, as Photoshop's Contextual Task Bar sits by one: centred
 * below `bounds`, in page pixels the camera places, or above when there's no room below, kept
 * inside the canvas, which clips it. */
export function ContextBar({
  bounds,
  className,
  style,
  ...props
}: { bounds: Bounds } & ComponentProps<'div'>) {
  const camera = useKoharuStore((state) => state.camera)
  const { refs, floatingStyles, isPositioned } = useFloating({
    placement: 'bottom',
    middleware: [offset(GAP), flip({ padding: GAP }), shift({ padding: GAP, crossAxis: true })],
    whileElementsMounted: autoUpdate,
  })

  // The selection as the canvas element shows it, the bar's parent.
  useLayoutEffect(() => {
    refs.setReference({
      getBoundingClientRect: () => {
        const canvas = refs.floating.current?.parentElement?.getBoundingClientRect()
        return new DOMRect(
          (canvas?.left ?? 0) + bounds.x * camera.zoom + camera.x,
          (canvas?.top ?? 0) + bounds.y * camera.zoom + camera.y,
          bounds.width * camera.zoom,
          bounds.height * camera.zoom,
        )
      },
    })
  }, [bounds, camera, refs])

  return (
    <div
      ref={refs.setFloating}
      role='toolbar'
      className={cn(
        'z-20 flex max-w-[calc(100%-16px)] items-center gap-1 rounded-xl border border-border bg-[var(--surface-floating)] p-1.5 shadow-[var(--shadow-toolrail)]',
        className,
      )}
      style={{ ...style, ...floatingStyles, visibility: isPositioned ? 'visible' : 'hidden' }}
      {...props}
    />
  )
}
