import type { Text, Transform } from '@koharu/bridge/protocol'

import type { Camera } from './store'

export interface Box {
  x: number
  y: number
  width: number
  height: number
}

const IDENTITY: Transform = [
  [1, 0, 0],
  [0, 1, 0],
]

/** Where a text's box lies on the page, its transform applied: the box's bounding rectangle,
 * or a point text's origin with no size. `null` for text on a path. */
export function textBounds(text: Text): Box | null {
  const [[a, b, c], [d, e, f]] = text.transform ?? IDENTITY
  const frame = text.frame ?? 'point'
  if (frame === 'point') return { x: c, y: f, width: 0, height: 0 }
  if (!('box' in frame) || !frame.box) return null
  const { x, y, width, height } = frame.box.bounds
  const corners = [
    [x, y],
    [x + width, y],
    [x, y + height],
    [x + width, y + height],
  ].map(([px, py]) => [a * px! + b * py! + c, d * px! + e * py! + f] as const)
  const xs = corners.map(([px]) => px)
  const ys = corners.map(([, py]) => py)
  const left = Math.min(...xs)
  const top = Math.min(...ys)
  return { x: left, y: top, width: Math.max(...xs) - left, height: Math.max(...ys) - top }
}

/** A page box in CSS pixels from the canvas element's corner. */
export function cssBox(box: Box, camera: Camera): Box {
  return {
    x: box.x * camera.zoom + camera.x,
    y: box.y * camera.zoom + camera.y,
    width: box.width * camera.zoom,
    height: box.height * camera.zoom,
  }
}
