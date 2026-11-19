import { describe, expect, it } from 'vitest'

import { dropMoves, nudgeable } from '@/lib/document'
import type { Layer } from '@koharu/ipc/protocol'

/** A shown, unlocked layer of `kind` holding `children`, with only what dropping reads. */
function layer(id: string, kind: Layer['kind'], children: Layer[] = []): Layer {
  return { id, kind, visible: true, locked: false, opacity: 1, children } as unknown as Layer
}

describe('dropMoves', () => {
  it('counts each move after the ones before it, so dragged layers keep their order', () => {
    const page = ['a', 'b', 'c', 'd'].map((id) => layer(id, 'raster'))
    expect(dropMoves(page, ['a', 'b'], { target: 'd', zone: 'after' })).toEqual([
      { layer: 'a', parent: null, index: 3 },
      { layer: 'b', parent: null, index: 3 },
    ])
    expect(dropMoves(page, ['a'], { target: 'b', zone: 'before' })).toEqual([])
  })

  it('puts a layer only where the scene lets it sit', () => {
    const page = [
      layer('panel', 'panel', [layer('balloon', 'balloon', [layer('text', 'block')])]),
      layer('raster', 'raster'),
      layer('free', 'block'),
    ]
    expect(dropMoves(page, ['raster'], { target: 'balloon', zone: 'inside' })).toBeNull()
    expect(dropMoves(page, ['free'], { target: 'text', zone: 'inside' })).toBeNull()
    expect(dropMoves(page, ['free'], { target: 'balloon', zone: 'inside' })).toEqual([
      { layer: 'free', parent: 'balloon', index: 1 },
    ])
    expect(dropMoves(page, ['panel'], { target: 'balloon', zone: 'after' })).toBeNull()
  })
})

describe('nudgeable', () => {
  it('moves what the scene lets move, with the layers inside a container', () => {
    const locked = { ...layer('locked', 'raster'), locked: true } as Layer
    const frozen = {
      ...layer('frozen', 'group', [layer('inside', 'raster')]),
      locked: true,
    } as Layer
    const page = [
      layer('panel', 'panel', [
        layer('block', 'block'),
        layer('folder', 'group', [layer('deep', 'raster')]),
      ]),
      layer('free', 'raster'),
      locked,
      frozen,
    ]
    expect(nudgeable(page, ['panel', 'block', 'free'])).toEqual(['panel', 'block', 'deep', 'free'])
    expect(nudgeable(page, ['locked', 'frozen', 'inside', 'gone'])).toEqual([])
  })
})
