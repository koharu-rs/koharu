'use client'

import { useEffect, useRef, type RefObject } from 'react'
import { useHotkeys } from 'react-hotkeys-hook'
import { useTranslation } from 'react-i18next'

import { useColorSampling } from '@/components/controls/ColorSampling'
import { CanvasCommandBar } from '@/components/editor/CanvasCommandBar'
import { FillBar } from '@/components/editor/FillBar'
import { HistoryWindow } from '@/components/editor/HistoryWindow'
import { StatusBar } from '@/components/editor/StatusBar'
import { ToolBar } from '@/components/editor/ToolBar'
import { TypeBar } from '@/components/editor/TypeBar'
import { toRgba } from '@/lib/color'
import { send, usePage } from '@/lib/queries'
import { deleteSelection } from '@/lib/scene'
import { shortcuts, toolKeys } from '@/lib/shortcuts'
import { useKoharuStore, type CanvasTool } from '@/lib/store'
import { commands, type Rgba } from '@koharu/ipc/protocol'

/** A command's shortcut matches the letter its key types, and keeps the key from the webview. */
const command = { preventDefault: true, useKey: true } as const
const tools = Object.keys(toolKeys) as CanvasTool[]

/** Each arrow key's step in page pixels, y growing downward. */
const nudges: Record<string, [number, number] | undefined> = {
  arrowleft: [-1, 0],
  arrowright: [1, 0],
  arrowup: [0, -1],
  arrowdown: [0, 1],
}

/** Tells the native canvas where `surface` lies in the window, for as long as it's mounted. */
export function useCanvasViewport(surface: RefObject<HTMLDivElement | null>) {
  useEffect(() => {
    const element = surface.current
    if (!element) return
    const report = () => {
      const { x, y, width, height } = element.getBoundingClientRect()
      send(commands.setViewport, x, y, width, height)
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
      send(commands.setViewport, 0, 0, 0, 0)
    }
  }, [surface])
}

/** The canvas's place in the window. The native canvas draws the page under the web interface
 * wherever this element and its ancestors paint nothing, and takes the pointer there; the
 * interface sets its tool, brush, and colours, and hears its camera and selection. */
export function CanvasWorkspace() {
  const { t } = useTranslation()
  const surface = useRef<HTMLDivElement>(null)
  const page = usePage().data
  const tool = useKoharuStore((state) => state.tool)
  const brush = useKoharuStore((state) => state.brush)
  const colorSampling = useColorSampling()

  useCanvasViewport(surface)

  useEffect(() => send(commands.setTool, tool), [tool])
  // The canvas numbers the texts and outlines their frames while Story shows.
  const panel = useKoharuStore((state) => state.panel)
  const storyHover = useKoharuStore((state) => state.storyHover)
  useEffect(
    () => send(commands.setStory, { shown: panel === 'story', hovered: storyHover }),
    [panel, storyHover],
  )

  useEffect(
    () =>
      send(commands.setBrush, {
        diameter: brush.diameter,
        color: toRgba(brush.color),
      }),
    [brush],
  )

  useEffect(() => {
    const sendColors = () =>
      send(commands.setColors, {
        workspace: themeColor('--surface-canvas'),
        selection: themeColor('--canvas-selection'),
      })
    sendColors()
    const theme = new MutationObserver(sendColors)
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
        send(commands.select, state.selectedLayers)
      }),
    [],
  )

  // Photoshop's shortcuts, which leave a focused field's or control's keys to it.
  const selectedLayers = useKoharuStore((state) => state.selectedLayers)
  const { selectLayers, setTool } = useKoharuStore.getState()
  useHotkeys(shortcuts.undo, () => send(commands.undo), command)
  useHotkeys(shortcuts.redo, () => send(commands.redo), command)
  useHotkeys(
    shortcuts.selectAll,
    () => page && selectLayers(page.layers.map((layer) => layer.id)),
    command,
  )
  useHotkeys(shortcuts.deselect, () => send(commands.deselect), command)
  useHotkeys(shortcuts.fit, () => send(commands.fit), command)
  useHotkeys(
    [shortcuts.delete, 'backspace'],
    () => {
      if (selectedLayers.length) send(deleteSelection, selectedLayers, page?.layers ?? [])
    },
    { preventDefault: () => selectedLayers.length > 0 },
  )
  // Photoshop's Move tool nudges the selection a pixel, and ten with Shift; the canvas moves what
  // a drag would. A focused control that took the key, such as a slider, keeps it.
  useHotkeys(
    Object.keys(nudges).flatMap((key) => [key, `shift+${key}`]),
    (_, { keys, shift }) => {
      const [dx, dy] = nudges[keys?.[0] ?? ''] ?? [0, 0]
      const distance = shift ? 10 : 1
      send(commands.nudge, dx * distance, dy * distance)
    },
    {
      enabled: tool === 'select' && selectedLayers.length > 0,
      preventDefault: true,
      ignoreEventWhen: (event) => event.defaultPrevented,
    },
  )
  useHotkeys('escape', () => {
    colorSampling?.cancel()
    selectLayers([])
  })
  useHotkeys(
    Object.values(toolKeys),
    (_, { keys }) => {
      const tool = tools.find((tool) => toolKeys[tool] === keys?.[0])
      if (tool) setTool(tool)
    },
    { useKey: true },
  )
  // Alt alone would hand the keys to the window's menu, wherever the focus is.
  useHotkeys('alt', () => undefined, {
    keydown: true,
    keyup: true,
    preventDefault: true,
    enableOnFormTags: true,
    enableOnContentEditable: true,
  })

  // Space holds the Hand tool, and letting it go puts back the tool before, as Photoshop's does.
  const held = useRef<CanvasTool | null>(null)
  useHotkeys(
    'space',
    (event) => {
      const { tool } = useKoharuStore.getState()
      if (event.type === 'keydown' && held.current === null && tool !== 'pan') {
        held.current = tool
        setTool('pan')
      } else if (event.type === 'keyup' && held.current !== null) {
        setTool(held.current)
        held.current = null
      }
    },
    { keydown: true, keyup: true, preventDefault: true },
  )
  // A window that loses the focus mid-hold never hears Space let go.
  useEffect(() => {
    const release = () => {
      if (held.current !== null) setTool(held.current)
      held.current = null
    }
    window.addEventListener('blur', release)
    return () => window.removeEventListener('blur', release)
  }, [setTool])

  return (
    <main className='relative flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-tl-2xl'>
      <CanvasCommandBar />
      <div className='relative flex min-h-0 min-w-0 flex-1'>
        <ToolBar />
        <div
          ref={surface}
          data-testid='canvas-viewport'
          // Clipped, never scrolled: a focused control past its edge would scroll it, and every
          // overlay with it, off the canvas the overlays sit on.
          className='relative min-h-0 min-w-0 flex-1 overflow-clip'
        >
          {page && <TypeBar />}
          {page && <FillBar />}
          {page && <HistoryWindow />}
          {!page && (
            <div className='pointer-events-none absolute inset-0 grid place-items-center'>
              <p className='text-[12px] text-muted-foreground'>{t('canvas.empty')}</p>
            </div>
          )}
        </div>
      </div>
      <StatusBar onZoomChange={(zoom) => send(commands.setZoom, zoom)} />
    </main>
  )
}

function sameLayers(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((layer, index) => layer === b[index])
}

/** A theme colour, whatever CSS notation it's written in, as the canvas's RGBA. */
function themeColor(variable: string): Rgba {
  return toRgba(getComputedStyle(document.documentElement).getPropertyValue(variable))
}
