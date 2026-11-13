'use client'

import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useColorSampling } from '@/components/controls/ColorSampling'
import { CanvasCommandBar } from '@/components/editor/CanvasCommandBar'
import { StatusBar } from '@/components/editor/StatusBar'
import { ToolBar } from '@/components/editor/ToolBar'
import { call, dispatch } from '@/lib/backend'
import { findLayer, isBlockLayer, isTextLayer, plainText } from '@/lib/document'
import { cssBox, textBounds } from '@/lib/geometry'
import { usePage } from '@/lib/queries'
import { hexToRgba, useKoharuStore, type CanvasTool } from '@/lib/store'
import { commands, type Layer, type Rgba, type Text } from '@koharu/bridge/protocol'

const TOOLS = ['select', 'text', 'draw', 'eraser', 'color_picker', 'remove', 'pan'] as const

/** The canvas's place in the window. The native canvas draws the page under the web interface
 * wherever this element and its ancestors paint nothing, and takes the pointer there; the
 * interface sets its tool, brush, and colours, and hears its camera and selection. */
export function CanvasWorkspace() {
  const { t } = useTranslation()
  const surface = useRef<HTMLDivElement>(null)
  const page = usePage().data
  const tool = useKoharuStore((state) => state.tool)
  const brush = useKoharuStore((state) => state.brush)
  const selected = useKoharuStore((state) => state.selectedLayers)
  const colorSampling = useColorSampling()

  useEffect(() => {
    const element = surface.current
    if (!element) return
    const report = () => {
      const { x, y, width, height } = element.getBoundingClientRect()
      dispatch(commands.setCanvasViewport, { x, y, width, height })
    }
    report()
    const resize = new ResizeObserver(report)
    resize.observe(element)
    window.addEventListener('resize', report)
    window.visualViewport?.addEventListener('resize', report)
    return () => {
      resize.disconnect()
      window.removeEventListener('resize', report)
      window.visualViewport?.removeEventListener('resize', report)
      // Nothing to draw while the editor is away, as under the preferences.
      dispatch(commands.setCanvasViewport, { x: 0, y: 0, width: 0, height: 0 })
    }
  }, [])

  useEffect(() => dispatch(commands.setTool, tool), [tool])

  useEffect(
    () => dispatch(commands.setBrush, { diameter: brush.diameter, color: hexToRgba(brush.color) }),
    [brush],
  )

  useEffect(() => {
    const send = () =>
      dispatch(commands.setColors, {
        workspace: cssColor('--surface-canvas'),
        selection: cssColor('--canvas-selection'),
      })
    send()
    const theme = new MutationObserver(send)
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
    return () => theme.disconnect()
  }, [])

  // A selection the interface made reaches the canvas; one the canvas published doesn't go back.
  useEffect(
    () =>
      useKoharuStore.subscribe((state, previous) => {
        if (state.selectedLayers === previous.selectedLayers) return
        if (sameLayers(state.selectedLayers, state.canvasSelection)) return
        useKoharuStore.setState({ canvasSelection: state.selectedLayers })
        dispatch(commands.selectLayers, state.selectedLayers)
      }),
    [],
  )

  useEffect(() => {
    const editable = (target: EventTarget | null) =>
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      (target instanceof HTMLElement && target.isContentEditable)
    // The tool Space replaced, put back when it's released, as Photoshop's hand tool.
    let held: CanvasTool | null = null

    const down = (event: KeyboardEvent) => {
      if (event.key === 'Alt') {
        event.preventDefault()
        return
      }
      if (editable(event.target)) return
      const state = useKoharuStore.getState()
      if (event.code === 'Space') {
        event.preventDefault()
        if (held === null && state.tool !== 'pan') {
          held = state.tool
          state.setTool('pan')
        }
        return
      }
      const command = event.ctrlKey || event.metaKey
      if (command && event.key.toLowerCase() === 'z') {
        event.preventDefault()
        dispatch(event.shiftKey ? commands.redo : commands.undo)
        return
      }
      if (command && event.key.toLowerCase() === 'a' && page) {
        event.preventDefault()
        state.selectLayers(page.layers.map((layer) => layer.id))
        return
      }
      if ((event.key === 'Delete' || event.key === 'Backspace') && state.selectedLayers.length) {
        event.preventDefault()
        dispatch(commands.deleteLayers, state.selectedLayers)
        return
      }
      if (event.key.toLowerCase() === state.shortcuts.fit) {
        dispatch(commands.fitCanvas)
        return
      }
      if (event.key === 'Escape') {
        colorSampling?.cancel()
        state.selectLayers([])
        return
      }
      const next = TOOLS.find((action) => state.shortcuts[action] === event.key.toLowerCase())
      if (next) state.setTool(next)
    }

    const up = (event: KeyboardEvent) => {
      if (event.key === 'Alt') event.preventDefault()
      if (event.code === 'Space' && held !== null) {
        useKoharuStore.getState().setTool(held)
        held = null
      }
    }
    const blur = () => {
      if (held === null) return
      useKoharuStore.getState().setTool(held)
      held = null
    }

    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    window.addEventListener('blur', blur)
    return () => {
      window.removeEventListener('keydown', down)
      window.removeEventListener('keyup', up)
      window.removeEventListener('blur', blur)
    }
  }, [colorSampling, page])

  const typed = tool === 'text' && selected.length === 1 && page
    ? findLayer(page.layers, selected[0]!)?.layer
    : undefined

  return (
    <main className='relative flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-tl-2xl'>
      <CanvasCommandBar />
      <div className='relative flex min-h-0 min-w-0 flex-1'>
        <ToolBar />
        <div
          ref={surface}
          data-testid='canvas-viewport'
          className='relative min-h-0 min-w-0 flex-1 overflow-hidden'
        >
          {typed && (isTextLayer(typed) || isBlockLayer(typed)) && (
            <TextBox key={typed.id} layer={typed} />
          )}
          {!page && (
            <div className='pointer-events-none absolute inset-0 grid place-items-center'>
              <p className='text-[12px] text-muted-foreground'>{t('canvas.empty')}</p>
            </div>
          )}
        </div>
      </div>
      <StatusBar onZoomChange={(zoom) => dispatch(commands.setZoom, zoom)} />
    </main>
  )
}

/** Typing into a text or a block's translation: an HTML text box over its frame, so the input
 * method composes in Chromium. It commits when it loses the focus, as a press on the canvas
 * takes it, and Escape leaves the text as it was. */
function TextBox({ layer }: { layer: Layer }) {
  const { t } = useTranslation()
  const camera = useKoharuStore((state) => state.camera)
  const text: Text | null = isTextLayer(layer)
    ? layer
    : isBlockLayer(layer)
      ? (layer.translation ?? layer.source)
      : null
  const original = isBlockLayer(layer) ? plainText(layer.translation) : plainText(text)
  const [value, setValue] = useState(original)
  const cancelled = useRef(false)
  const bounds = text ? textBounds(text) : null
  if (!bounds) return null
  const box = cssBox(bounds, camera)
  const size = (text?.style?.chars?.size ?? 16) * camera.zoom

  const commit = () => {
    if (cancelled.current || value === original) return
    void call(commands.setText, layer.id, value).catch(() => undefined)
  }

  return (
    <textarea
      autoFocus
      aria-label={t('canvas.typeText')}
      value={value}
      onChange={(event) => setValue(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          cancelled.current = true
          event.currentTarget.blur()
        } else if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
          event.currentTarget.blur()
        }
      }}
      className='absolute z-10 resize-none rounded-sm border border-[var(--canvas-selection)] bg-popover/95 p-1 text-foreground shadow-sm outline-none'
      style={{
        left: box.x,
        top: box.y,
        width: Math.max(box.width, 160),
        height: Math.max(box.height, size * 1.6 + 8),
        fontSize: Math.max(size, 12),
        lineHeight: 1.3,
      }}
    />
  )
}

function sameLayers(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((layer, index) => layer === b[index])
}

/** A theme colour, whatever CSS notation it's written in, as the canvas's RGBA. */
function cssColor(variable: string): Rgba {
  const probe = document.createElement('canvas')
  probe.width = probe.height = 1
  const context = probe.getContext('2d', { willReadFrequently: true })
  if (!context) return [0, 0, 0, 1]
  context.fillStyle =
    getComputedStyle(document.documentElement).getPropertyValue(variable).trim() || '#000000'
  context.fillRect(0, 0, 1, 1)
  const [r = 0, g = 0, b = 0, a = 255] = context.getImageData(0, 0, 1, 1).data
  return [r / 255, g / 255, b / 255, a / 255]
}
