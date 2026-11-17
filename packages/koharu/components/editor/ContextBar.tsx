'use client'

import { useLayoutEffect, useRef, useState, type ComponentProps } from 'react'

import { useKoharuStore } from '@/lib/store'
import type { Bounds } from '@koharu/ipc/protocol'
import { cn } from '@koharu/ui/lib/utils'

interface Size {
  width: number
  height: number
}

/** The space between the bar and what it's beside, and the canvas's edges, in CSS pixels. */
const GAP = 8

/** A bar beside a selection on the canvas, as Photoshop's Contextual Task Bar sits by one: centred
 * below `bounds`, in page pixels the camera places, or above when there's no room below, kept
 * inside the canvas. */
export function ContextBar({
  bounds,
  className,
  style,
  ...props
}: { bounds: Bounds } & ComponentProps<'div'>) {
  const camera = useKoharuStore((state) => state.camera)
  const bar = useRef<HTMLDivElement>(null)
  // The bar's size and the canvas's, measured again whenever either changes, as when the bar
  // wraps to a second row or the window is resized.
  const [sizes, setSizes] = useState<{ bar: Size; room: Size } | null>(null)

  useLayoutEffect(() => {
    const element = bar.current
    const room = element?.parentElement
    if (!element || !room) return
    const measure = () =>
      setSizes({
        bar: { width: element.offsetWidth, height: element.offsetHeight },
        room: { width: room.clientWidth, height: room.clientHeight },
      })
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    observer.observe(room)
    return () => observer.disconnect()
  }, [])

  const area = {
    x: bounds.x * camera.zoom + camera.x,
    y: bounds.y * camera.zoom + camera.y,
    width: bounds.width * camera.zoom,
    height: bounds.height * camera.zoom,
  }
  const place = sizes && {
    top:
      area.y + area.height + GAP + sizes.bar.height <= sizes.room.height - GAP
        ? area.y + area.height + GAP
        : Math.max(
            GAP,
            Math.min(area.y - GAP - sizes.bar.height, sizes.room.height - GAP - sizes.bar.height),
          ),
    left: Math.max(
      GAP,
      Math.min(
        area.x + area.width / 2 - sizes.bar.width / 2,
        sizes.room.width - GAP - sizes.bar.width,
      ),
    ),
  }

  return (
    <div
      ref={bar}
      role='toolbar'
      className={cn(
        'absolute z-20 flex max-w-[calc(100%-16px)] items-center gap-1 rounded-xl border border-border bg-[var(--surface-floating)] p-1.5 shadow-[var(--shadow-toolrail)]',
        className,
      )}
      style={{
        ...style,
        left: place?.left ?? 0,
        top: place?.top ?? 0,
        visibility: place ? 'visible' : 'hidden',
      }}
      {...props}
    />
  )
}
