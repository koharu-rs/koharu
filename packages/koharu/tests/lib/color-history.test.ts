import { afterEach, describe, expect, it } from 'vitest'

import {
  COLOR_HISTORY_LIMIT,
  COLOR_HISTORY_STORAGE_KEY,
  nextColorHistory,
  rememberColor,
  useColorHistory,
} from '@/lib/colorHistory'

describe('color history', () => {
  afterEach(() => {
    useColorHistory.setState({ colors: [] })
    window.localStorage.removeItem(COLOR_HISTORY_STORAGE_KEY)
  })

  it('moves a repeated color to the front and keeps a short list', () => {
    expect(nextColorHistory(['#111111'], '#ABCDEF')).toEqual(['#ABCDEF', '#111111'])
    expect(nextColorHistory(['#ABCDEF', '#111111'], '#111111')).toEqual(['#111111', '#ABCDEF'])
    expect(nextColorHistory(['#ABCDEF'], '#abc')).toEqual(['#AABBCC', '#ABCDEF'])

    const filled = Array.from({ length: COLOR_HISTORY_LIMIT }, (_, index) => `#00000${index}`)
    expect(nextColorHistory(filled, '#FFFFFF')).toEqual(['#FFFFFF', ...filled.slice(0, -1)])
  })

  it('persists committed colors and ignores invalid values', () => {
    rememberColor('#fff')
    rememberColor('not-a-color')
    rememberColor('#123456')

    expect(useColorHistory.getState().colors).toEqual(['#123456', '#FFFFFF'])
    expect(JSON.parse(window.localStorage.getItem(COLOR_HISTORY_STORAGE_KEY) ?? '[]')).toEqual([
      '#123456',
      '#FFFFFF',
    ])
  })
})
