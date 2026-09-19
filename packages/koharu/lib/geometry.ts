import type { EntityId, Frame, Layer, Point, TransformFrame } from '@koharu/bridge/protocol'

import { effectiveLayerVisibility, isTextLayer } from './document'

const minimumFrameSize = 1e-6

export interface Camera {
  zoom: number
  translation: [number, number]
}

export interface CssFrame {
  left: number
  top: number
  width: number
  height: number
  angle: number
}

export type ResizeHandle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w'

const resizeDirections: Record<ResizeHandle, Point> = {
  nw: { x: -1, y: -1 },
  n: { x: 0, y: -1 },
  ne: { x: 1, y: -1 },
  e: { x: 1, y: 0 },
  se: { x: 1, y: 1 },
  s: { x: 0, y: 1 },
  sw: { x: -1, y: 1 },
  w: { x: -1, y: 0 },
}

export function physicalPoint(clientX: number, clientY: number, bounds: DOMRect): Point {
  const dpr = window.devicePixelRatio
  return { x: (clientX - bounds.x) * dpr, y: (clientY - bounds.y) * dpr }
}

export function pagePoint(
  clientX: number,
  clientY: number,
  bounds: DOMRect,
  camera: Camera,
): Point {
  const point = physicalPoint(clientX, clientY, bounds)
  return {
    x: (point.x - camera.translation[0]) / camera.zoom,
    y: (point.y - camera.translation[1]) / camera.zoom,
  }
}

export function draftFrame(start: Point, end: Point): Frame {
  return {
    x: Math.min(start.x, end.x),
    y: Math.min(start.y, end.y),
    width: Math.max(1, Math.abs(end.x - start.x)),
    height: Math.max(1, Math.abs(end.y - start.y)),
    angle_degrees: 0,
  }
}

export function selectableLayer(layer: Layer): boolean {
  return layer.type === 'text' || layer.type === 'image'
}

export function layerFrame(layer: Layer): Frame | null {
  const points =
    layer.type === 'text' || layer.type === 'image' || layer.type === 'artwork'
      ? layer.geometry?.points
      : null
  if (!points?.length || points.some((point) => !finite(point.x, point.y))) return null
  if (layer.type === 'text' && layer.angle_degrees !== null) {
    if (!finite(layer.angle_degrees)) return null
    const angle = (layer.angle_degrees * Math.PI) / 180
    const cos = Math.cos(angle)
    const sin = Math.sin(angle)
    const xs = points.map((point) => point.x * cos + point.y * sin)
    const ys = points.map((point) => -point.x * sin + point.y * cos)
    const left = Math.min(...xs)
    const top = Math.min(...ys)
    const right = Math.max(...xs)
    const bottom = Math.max(...ys)
    const width = right - left
    const height = bottom - top
    const centerX = (left + right) / 2
    const centerY = (top + bottom) / 2
    return width > minimumFrameSize && height > minimumFrameSize
      ? {
          x: centerX * cos - centerY * sin - width / 2,
          y: centerX * sin + centerY * cos - height / 2,
          width,
          height,
          angle_degrees: layer.angle_degrees,
        }
      : null
  }
  if (points.length === 4) {
    const [topLeft, topRight, bottomRight, bottomLeft] = points
    const top: [number, number] = [topRight.x - topLeft.x, topRight.y - topLeft.y]
    const right: [number, number] = [bottomRight.x - topRight.x, bottomRight.y - topRight.y]
    const bottom: [number, number] = [bottomLeft.x - bottomRight.x, bottomLeft.y - bottomRight.y]
    const left: [number, number] = [topLeft.x - bottomLeft.x, topLeft.y - bottomLeft.y]
    const width = Math.hypot(...top)
    const height = Math.hypot(...right)
    if (width > minimumFrameSize && height > minimumFrameSize) {
      const scale = Math.max(width, height, 1)
      const oppositeLengthsMatch =
        Math.abs(Math.hypot(...bottom) - width) <= scale * 1e-6 &&
        Math.abs(Math.hypot(...left) - height) <= scale * 1e-6
      const perpendicular = Math.abs(top[0] * right[0] + top[1] * right[1]) <= width * height * 1e-6
      const diagonalsBisect =
        Math.abs(topLeft.x + bottomRight.x - topRight.x - bottomLeft.x) <= scale * 1e-6 &&
        Math.abs(topLeft.y + bottomRight.y - topRight.y - bottomLeft.y) <= scale * 1e-6
      if (oppositeLengthsMatch && perpendicular && diagonalsBisect) {
        const centerX = points.reduce((sum, point) => sum + point.x, 0) * 0.25
        const centerY = points.reduce((sum, point) => sum + point.y, 0) * 0.25
        return {
          x: centerX - width * 0.5,
          y: centerY - height * 0.5,
          width,
          height,
          angle_degrees: (Math.atan2(top[1], top[0]) * 180) / Math.PI,
        }
      }
    }
  }

  const xs = points.map((point) => point.x)
  const ys = points.map((point) => point.y)
  const x = Math.min(...xs)
  const y = Math.min(...ys)
  const width = Math.max(...xs) - x
  const height = Math.max(...ys) - y
  return width > minimumFrameSize && height > minimumFrameSize
    ? { x, y, width, height, angle_degrees: 0 }
    : null
}

export function controlFrame(
  layer: Layer,
  frames: Readonly<Record<EntityId, Frame>>,
): Frame | null {
  const frame = isTextLayer(layer) ? frames[layer.id] : undefined
  return frame && validFrame(frame) ? frame : layerFrame(layer)
}

export function hitTestLayers(
  layers: Layer[],
  point: Point,
  frames: Readonly<Record<EntityId, Frame>>,
): Layer | null {
  for (let index = layers.length - 1; index >= 0; index -= 1) {
    const layer = layers[index]
    const visibility = effectiveLayerVisibility(layers, layer)
    if (!selectableLayer(layer) || !visibility.visible || visibility.opacity <= 0) continue
    const frame = controlFrame(layer, frames)
    if (frame && frameContains(frame, point)) return layer
  }
  return null
}

export function frameContains(frame: Frame, point: Point): boolean {
  const centerX = frame.x + frame.width * 0.5
  const centerY = frame.y + frame.height * 0.5
  const angle = (-frame.angle_degrees * Math.PI) / 180
  const cos = Math.cos(angle)
  const sin = Math.sin(angle)
  const x = point.x - centerX
  const y = point.y - centerY
  const localX = x * cos - y * sin
  const localY = x * sin + y * cos
  return Math.abs(localX) <= frame.width * 0.5 && Math.abs(localY) <= frame.height * 0.5
}

export function cssFrame(frame: Frame, camera: Camera): CssFrame {
  const dpr = window.devicePixelRatio
  const scale = camera.zoom / dpr
  return {
    left: (frame.x * camera.zoom + camera.translation[0]) / dpr,
    top: (frame.y * camera.zoom + camera.translation[1]) / dpr,
    width: frame.width * scale,
    height: frame.height * scale,
    angle: frame.angle_degrees,
  }
}

export function resizeFrame(
  frame: Frame,
  handle: ResizeHandle,
  point: Point,
  minimumSize: number,
): Frame {
  const direction = resizeDirections[handle]
  const angle = (frame.angle_degrees * Math.PI) / 180
  const widthAxis = { x: Math.cos(angle), y: Math.sin(angle) }
  const heightAxis = { x: -widthAxis.y, y: widthAxis.x }
  const center = { x: frame.x + frame.width * 0.5, y: frame.y + frame.height * 0.5 }
  const anchor = {
    x:
      center.x -
      widthAxis.x * direction.x * frame.width * 0.5 -
      heightAxis.x * direction.y * frame.height * 0.5,
    y:
      center.y -
      widthAxis.y * direction.x * frame.width * 0.5 -
      heightAxis.y * direction.y * frame.height * 0.5,
  }
  const delta = { x: point.x - anchor.x, y: point.y - anchor.y }
  const width = direction.x
    ? Math.max(minimumSize, direction.x * dot(delta, widthAxis))
    : frame.width
  const height = direction.y
    ? Math.max(minimumSize, direction.y * dot(delta, heightAxis))
    : frame.height
  const nextCenter = {
    x:
      anchor.x +
      widthAxis.x * direction.x * width * 0.5 +
      heightAxis.x * direction.y * height * 0.5,
    y:
      anchor.y +
      widthAxis.y * direction.x * width * 0.5 +
      heightAxis.y * direction.y * height * 0.5,
  }
  return {
    ...frame,
    x: nextCenter.x - width * 0.5,
    y: nextCenter.y - height * 0.5,
    width,
    height,
  }
}

export function rotateFrame(frame: Frame, start: Point, point: Point): Frame {
  const center = { x: frame.x + frame.width * 0.5, y: frame.y + frame.height * 0.5 }
  const from = { x: start.x - center.x, y: start.y - center.y }
  const to = { x: point.x - center.x, y: point.y - center.y }
  if (
    Math.hypot(from.x, from.y) <= minimumFrameSize ||
    Math.hypot(to.x, to.y) <= minimumFrameSize
  ) {
    return frame
  }
  const delta = (Math.atan2(from.x * to.y - from.y * to.x, dot(from, to)) * 180) / Math.PI
  return { ...frame, angle_degrees: normalizeDegrees(frame.angle_degrees + delta) }
}

export function translateFrames(originals: TransformFrame[], delta: Point): TransformFrame[] {
  return originals.map(({ element, frame }) => ({
    element,
    frame: { ...frame, x: frame.x + delta.x, y: frame.y + delta.y },
  }))
}

function finite(...values: number[]): boolean {
  return values.every(Number.isFinite)
}

function dot(left: Point, right: Point): number {
  return left.x * right.x + left.y * right.y
}

function normalizeDegrees(value: number): number {
  return ((((value + 180) % 360) + 360) % 360) - 180
}

function validFrame(frame: Frame): boolean {
  return (
    finite(frame.x, frame.y, frame.width, frame.height, frame.angle_degrees) &&
    frame.width > minimumFrameSize &&
    frame.height > minimumFrameSize
  )
}
