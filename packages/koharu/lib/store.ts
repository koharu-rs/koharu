'use client'

import { create } from 'zustand'

import type {
  Job,
  LayerId,
  ModelInfo,
  PageId,
  Preferences,
  Rgba,
  Size,
  Stage,
  StartupState,
  Tool,
  Transfer,
} from '@koharu/bridge/protocol'
import { toast } from '@koharu/ui/components/toast'

export type CanvasTool = Tool
export const MIN_BRUSH_DIAMETER = 1
export function maxBrushDiameter(size: Size): number {
  // Assume that for all sizes below 2048 width, 128 is sufficient.
  // For anything above, scale by percentage greater than 2048 width.
  return Math.round(Math.max(size.width / 2048, 1.0) * 128)
}

export function isBrushTool(tool: CanvasTool): boolean {
  return tool === 'draw' || tool === 'eraser' || tool === 'remove'
}

export interface CanvasBrush {
  diameter: number
  color: string
}

/** Where the page shows: a page point `p` sits at `p * zoom + (x, y)` in CSS pixels from the
 * canvas element's corner, as the native canvas publishes it. */
export interface Camera {
  zoom: number
  x: number
  y: number
}

export type InspectorSection = 'copy' | 'type' | 'layers'
export type ShortcutAction = CanvasTool | 'fit'
export type Shortcuts = Record<ShortcutAction, string>
export type PipelineScope = 'page' | 'selected-pages' | 'project'
export const pipelineStages: readonly Stage[] = ['detect', 'recognize', 'inpaint', 'translate']

interface KoharuStore {
  initialized: boolean
  preferences: Preferences | null
  models: ModelInfo[]
  jobs: Record<string, Job>
  downloads: Record<string, Transfer>
  camera: Camera
  /** The colour the canvas's picker picked last, numbered so a repeat counts. */
  picked: { color: string; serial: number } | null
  selectedLayers: LayerId[]
  /** The selection the canvas has, so one it published isn't sent back to it. */
  canvasSelection: LayerId[]
  selectedPages: PageId[]
  tool: CanvasTool
  brush: CanvasBrush
  inspector: InspectorSection
  processingScope: PipelineScope
  processingStages: Stage[]
  settingsOpen: boolean
  shortcuts: Shortcuts
  selectPages: (pages: PageId[]) => void
  showInspector: (section: InspectorSection) => void
  setProcessingScope: (scope: PipelineScope) => void
  setProcessingStages: (stages: Stage[]) => void
  setSettingsOpen: (open: boolean) => void
  selectLayers: (layers: LayerId[]) => void
  setTool: (tool: CanvasTool) => void
  setBrush: (brush: CanvasBrush) => void
  setShortcut: (action: ShortcutAction, key: string) => void
  dismissJob: (id: string) => void
  dismissDownload: (id: number) => void
}

export const defaultShortcuts: Shortcuts = {
  select: 'v',
  text: 't',
  draw: 'b',
  eraser: 'e',
  color_picker: 'i',
  remove: 'j',
  pan: 'h',
  fit: '0',
}

export const useKoharuStore = create<KoharuStore>()((set) => ({
  initialized: false,
  preferences: null,
  models: [],
  jobs: {},
  downloads: {},
  camera: { zoom: 1, x: 0, y: 0 },
  picked: null,
  selectedLayers: [],
  canvasSelection: [],
  selectedPages: [],
  tool: 'select',
  brush: { diameter: 48, color: '#FFFFFF' },
  inspector: 'copy',
  processingScope: 'selected-pages',
  processingStages: [...pipelineStages],
  settingsOpen: false,
  shortcuts: defaultShortcuts,
  selectPages: (selectedPages) => set({ selectedPages: [...new Set(selectedPages)] }),
  showInspector: (inspector) => set({ inspector }),
  setProcessingScope: (processingScope) => set({ processingScope }),
  setProcessingStages: (processingStages) => set({ processingStages }),
  setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
  selectLayers: (selectedLayers) => set({ selectedLayers: [...new Set(selectedLayers)] }),
  setTool: (tool) => set({ tool }),
  setBrush: (brush) => set({ brush }),
  setShortcut: (action, key) =>
    set((state) => ({
      shortcuts: { ...state.shortcuts, [action]: key.toLowerCase().slice(0, 1) },
    })),
  dismissJob: (id) =>
    set((state) => {
      const jobs = { ...state.jobs }
      delete jobs[id]
      return { jobs }
    }),
  dismissDownload: (id) =>
    set((state) => {
      const downloads = { ...state.downloads }
      delete downloads[String(id)]
      return { downloads }
    }),
}))

export function receiveStartupState(state: StartupState): void {
  useKoharuStore.setState({ initialized: true, jobs: byId(state.jobs) })
}

export function receiveJob(job: Job): void {
  useKoharuStore.setState((state) => ({ jobs: { ...state.jobs, [job.id]: job } }))
}

export function receiveDownload(download: Transfer): void {
  useKoharuStore.setState((state) => {
    const downloads = { ...state.downloads }
    if (download.state === 'Finished') delete downloads[String(download.id)]
    else downloads[String(download.id)] = download
    return { downloads }
  })
}

/** A selection the canvas made. */
export function receiveCanvasSelection(layers: LayerId[]): void {
  useKoharuStore.setState({ selectedLayers: [...new Set(layers)], canvasSelection: layers })
}

export function receivePickedColor(color: string): void {
  useKoharuStore.setState((state) => ({
    picked: { color, serial: (state.picked?.serial ?? 0) + 1 },
  }))
}

export function receivePreferences(preferences: Preferences): void {
  useKoharuStore.setState({ preferences })
}

export function receiveModels(models: ModelInfo[]): void {
  useKoharuStore.setState({ models })
}

export function receiveError(message: string): void {
  toast.add({ type: 'error', title: 'Could not complete that action', description: message })
}

/** A CSS hex colour as the canvas's straight RGBA, from 0 to 1. */
export function hexToRgba(hex: string): Rgba {
  const value = hex.replace('#', '')
  const channel = (start: number) => Number.parseInt(value.slice(start, start + 2), 16) / 255
  return [channel(0), channel(2), channel(4), 1]
}

/** The canvas's RGBA as a CSS hex colour, its alpha left out. */
export function rgbaToHex(color: Rgba): string {
  return `#${color
    .slice(0, 3)
    .map((channel) =>
      Math.round(Math.min(1, Math.max(0, channel)) * 255)
        .toString(16)
        .padStart(2, '0'),
    )
    .join('')}`.toUpperCase()
}

function byId<T extends { id: string | number }>(items: T[]): Record<string, T> {
  return Object.fromEntries(items.map((item) => [String(item.id), item]))
}
