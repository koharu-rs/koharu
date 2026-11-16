import type { Text, Transform } from '@koharu/ipc/protocol'

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

/** Where a text's frame lies on the page, its transform applied: the bounding rectangle of its
 * box or its path, or a point text's origin with no size. `null` for a path with no points. */
export function textBounds(text: Text): Box | null {
  const [[a, b, c], [d, e, f]] = text.transform ?? IDENTITY
  const frame = text.frame ?? 'point'
  if (frame === 'point') return { x: c, y: f, width: 0, height: 0 }
  const local = 'box' in frame && frame.box ? frame.box.bounds : pathBounds(frame.path.path)
  if (!local) return null
  const { x, y, width, height } = local
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

/** The rectangle around an SVG path's points, its curves' control points included, which holds
 * the curves too. A frame's path is kurbo's SVG, all absolute commands, so every number pair is a
 * point. */
function pathBounds(path: string): Box | null {
  const numbers = (path.match(/-?(?:\d+\.?\d*|\.\d+)(?:e[-+]?\d+)?/gi) ?? []).map(Number)
  const xs = numbers.filter((_, index) => index % 2 === 0)
  const ys = numbers.filter((_, index) => index % 2 === 1)
  if (!ys.length) return null
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
