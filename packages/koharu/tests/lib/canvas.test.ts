import { describe, expect, it, vi } from 'vitest'

import {
  activateCanvas,
  prefetchCanvasPages,
  previewCanvasOpacity,
  showCanvasPage,
  type Canvas,
} from '@koharu/bridge/canvas'

const protocolCommands = vi.hoisted(() => ({
  prepareCanvasPage: vi.fn(),
  getCanvasPageManifest: vi.fn(),
  getCanvasPageResource: vi.fn(),
}))

const commands = vi.hoisted(() => ({
  prepareCanvasPage: vi.fn(),
  getCanvasPageManifest: vi.fn(),
  getCanvasPageResource: vi.fn(),
}))

vi.mock('@koharu/bridge/protocol', () => ({ commands: protocolCommands }))
vi.mock('@koharu/bridge', () => ({ commands }))

describe('canvas runtime', () => {
  it('routes inspector previews only to the active canvas', () => {
    const previewOpacity = vi.fn()
    const canvas = { previewOpacity } as unknown as Canvas
    const deactivate = activateCanvas(canvas)

    previewCanvasOpacity('layer', 0.5)
    deactivate()
    previewCanvasOpacity('layer', null)

    expect(previewOpacity).toHaveBeenCalledOnce()
    expect(previewOpacity).toHaveBeenCalledWith('layer', 0.5)
  })

  it('shows a cached page immediately on the active canvas', () => {
    const activatePage = vi.fn(() => true)
    const canvas = { activatePage } as unknown as Canvas
    const deactivate = activateCanvas(canvas)

    expect(showCanvasPage('page', 4)).toBe(true)
    deactivate()

    expect(activatePage).toHaveBeenCalledWith('page', 4)
  })

  it('prepares and installs adjacent page resources before caching the frame', async () => {
    const order: string[] = []
    const prepared = {
      revision: 4,
      page: { id: 'page' },
    }
    commands.prepareCanvasPage.mockResolvedValue(prepared)
    commands.getCanvasPageManifest.mockResolvedValue(new ArrayBuffer(1))
    commands.getCanvasPageResource.mockResolvedValue(new ArrayBuffer(1))
    const canvas = {
      stageManifest: vi.fn(() => ({ token: 7, missing: ['resource'] })),
      installResource: vi.fn(async () => void order.push('resource')),
      cacheFrame: vi.fn(() => {
        order.push('frame')
        return true
      }),
    } as unknown as Canvas
    const deactivate = activateCanvas(canvas)

    const result = await prefetchCanvasPages(['page'])
    deactivate()

    expect(commands.prepareCanvasPage).toHaveBeenCalledWith('page')
    expect(commands.getCanvasPageManifest).toHaveBeenCalledWith('page', 4)
    expect(commands.getCanvasPageResource).toHaveBeenCalledWith('page', 4, 'resource')
    expect(protocolCommands.prepareCanvasPage).not.toHaveBeenCalled()
    expect(protocolCommands.getCanvasPageManifest).not.toHaveBeenCalled()
    expect(protocolCommands.getCanvasPageResource).not.toHaveBeenCalled()
    expect(canvas.cacheFrame).toHaveBeenCalledWith(7, 'page')
    expect(order).toEqual(['resource', 'frame'])
    expect(result).toEqual([prepared])
  })

  it('does not expose page data when the frame was rejected by the cache', async () => {
    commands.prepareCanvasPage.mockResolvedValue({ revision: 4, page: { id: 'page' } })
    commands.getCanvasPageManifest.mockResolvedValue(new ArrayBuffer(1))
    const canvas = {
      stageManifest: vi.fn(() => ({ token: 7, missing: [] })),
      cacheFrame: vi.fn(() => false),
    } as unknown as Canvas
    const deactivate = activateCanvas(canvas)

    const result = await prefetchCanvasPages(['page'])
    deactivate()

    expect(result).toEqual([])
  })
})
