'use client'

import {
  autoUpdate,
  flip,
  offset,
  shift,
  useFloating,
  type MiddlewareState,
} from '@floating-ui/react-dom'
import { useLayoutEffect, type ComponentProps } from 'react'

import { useKoharuStore } from '@/lib/store'
import type { Bounds } from '@koharu/ipc/protocol'
import { cn } from '@koharu/ui/lib/utils'

/** The space between the bar and what it's beside, and the canvas's edges, in CSS pixels. */
const GAP = 8

/** A bar beside a selection on the canvas, as Photoshop's Contextual Task Bar sits by one: centred
 * below `bounds`, in page pixels the camera places, or above when there's no room below, kept
 * inside the canvas element, its parent. A bar the canvas is too narrow for starts at its left edge
 * and runs past its right one, which clips no fixed element. */
export function ContextBar({
  bounds,
  className,
  style,
  ...props
}: { bounds: Bounds } & ComponentProps<'div'>) {
  const camera = useKoharuStore((state) => state.camera)
  const { refs, floatingStyles, isPositioned } = useFloating({
    placement: 'bottom',
    strategy: 'fixed',
    middleware: [
      offset(GAP),
      flip(inCanvas),
      shift((state) => ({ ...inCanvas(state), crossAxis: true })),
    ],
    whileElementsMounted: autoUpdate,
  })

  // The selection as the canvas shows it, which moves and resizes with the canvas.
  useLayoutEffect(() => {
    const canvas = refs.floating.current?.parentElement
    if (!canvas) return
    refs.setReference({
      contextElement: canvas,
      getBoundingClientRect: () => {
        const { left, top } = canvas.getBoundingClientRect()
        return new DOMRect(
          left + bounds.x * camera.zoom + camera.x,
          top + bounds.y * camera.zoom + camera.y,
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
        'z-20 flex w-max items-center gap-1 rounded-xl border border-border bg-[var(--surface-floating)] p-1.5 shadow-[var(--shadow-toolrail)]',
        className,
      )}
      style={{ ...style, ...floatingStyles, visibility: isPositioned ? 'visible' : 'hidden' }}
      {...props}
    />
  )
}

/** The canvas, which a fixed bar's own clipping ancestors leave out. */
function inCanvas({ elements }: MiddlewareState) {
  return { boundary: elements.floating.parentElement ?? undefined, padding: GAP }
}
