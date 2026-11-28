import { expect, it } from 'vitest'

import { translationMatches } from '@/lib/translation-matches'
import type { Layer, Page } from '@koharu/ipc/protocol'

function block(id: string, ocr: string, translation = ''): Layer {
  return {
    id,
    kind: 'block',
    source: { content: [{ insert: ocr }] },
    translation: { content: [{ insert: translation }] },
  } as Layer
}

it('matches reordered OCR without shifting past a missing balloon or guessing duplicates', () => {
  const source = {
    layers: [
      block('a', '待って', 'Wait'),
      block('b', '危ない', 'Danger'),
      block('c', '走れ', 'Run'),
      block('d', 'うん', 'Yes'),
      block('e', 'うん'),
      block('f', '声', 'Voice'),
    ],
  } as Page
  const target = {
    layers: [
      block('1', '走れ'),
      block('2', 'うん'),
      block('3', '待って'),
      block('4', '声'),
      block('5', '声', 'Already edited'),
    ],
  } as Page
  expect(translationMatches(source, target).map(({ target, text }) => [target, text])).toEqual([
    ['1', 'Run'],
    ['3', 'Wait'],
  ])
})

it('preserves filled fields and inherited locks and matches canonically equivalent OCR', () => {
  const source = {
    layers: [block('a', 'Café', 'Coffee'), block('b', '空', 'Sky'), block('c', '海', 'Sea')],
  } as Page
  const target = {
    layers: [
      block('1', 'Cafe\u0301'),
      block('2', '空', 'Existing'),
      { kind: 'group', locked: true, children: [block('3', '海')] },
    ],
  } as Page
  expect(translationMatches(source, target).map(({ target }) => target)).toEqual(['1'])
})
