import { QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { createElement, type ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import {
  importPaths,
  queryClient,
  send,
  useEditPreferences,
  useModelCatalog,
  usePreferences,
  useSavePreferences,
} from '@/lib/queries'
import { commands, type ModelCatalog, type Preferences } from '@koharu/ipc/protocol'
import { toast } from '@koharu/ui/components/toast'

describe('send', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('lets the cache drop each command as soon as it settles, failed or not', async () => {
    const add = vi.spyOn(toast, 'add')
    send(() => Promise.resolve('done'))
    send(() => Promise.reject(new Error('boom')))

    // Flush the settled promises and zero-delay timers only, well short of TanStack Query's
    // default gcTime for mutations (5 minutes).
    await vi.advanceTimersByTimeAsync(0)

    expect(add).toHaveBeenCalledTimes(1)
    expect(queryClient.getMutationCache().getAll()).toHaveLength(0)
  })
})

describe('useSavePreferences', () => {
  const preferences = (font: string): Preferences => ({
    pipeline: {},
    renderer: { styles: { antique: { Latn: { family: font } } } },
    agent: {},
    providers: {},
    export: { cbz_codec: 'png', quality: 90, lossless: false },
  })

  it('shows what the desktop last answered once a save fails, but not over a newer save', async () => {
    // The file reads once, then doesn't, as when the save fails because it's invalid.
    vi.spyOn(commands, 'getPreferences')
      .mockResolvedValueOnce(preferences('Read'))
      .mockRejectedValue(new Error('invalid config.toml'))
    const failed = Promise.withResolvers<Preferences>()
    const newer = Promise.withResolvers<Preferences>()
    const save = vi
      .spyOn(commands, 'savePreferences')
      .mockResolvedValueOnce(preferences('Saved'))
      .mockReturnValueOnce(failed.promise)
      .mockReturnValueOnce(newer.promise)
    const { result } = renderHook(() => ({ shown: usePreferences(), save: useSavePreferences() }), {
      wrapper: ({ children }: { children: ReactNode }) =>
        createElement(QueryClientProvider, { client: queryClient }, children),
    })
    await waitFor(() => expect(result.current.shown.data).toEqual(preferences('Read')))

    act(() => result.current.save.mutate(preferences('Typed')))
    await waitFor(() => expect(result.current.shown.data).toEqual(preferences('Saved')))

    act(() => {
      result.current.save.mutate(preferences('Failed'))
      result.current.save.mutate(preferences('Newer'))
    })
    await waitFor(() => expect(result.current.shown.data).toEqual(preferences('Newer')))
    await act(async () => failed.reject(new Error('config.toml is locked')))
    await waitFor(() => expect(save).toHaveBeenCalledTimes(3))
    expect(result.current.shown.data).toEqual(preferences('Newer'))

    await act(async () => newer.reject(new Error('config.toml is locked')))
    await waitFor(() => expect(result.current.save.isError).toBe(true))
    expect(result.current.shown.data).toEqual(preferences('Saved'))
  })

  it('invalidates modelCatalogKey when translator.vision toggles via useEditPreferences', async () => {
    const prefs = (vision: boolean): Preferences => ({
      pipeline: { translator: { model: 'koharu/gemma4-e2b-it', vision } },
      renderer: { styles: { antique: { Latn: { family: 'CCWildWords' } } } },
      agent: {},
      providers: {},
      export: { cbz_codec: 'png', quality: 90, lossless: false },
    })
    const emptyCatalog: ModelCatalog = {
      available_vram_bytes: null,
      total_vram_bytes: null,
      detect: [],
      recognize: [],
      inpaint: [],
      translate: [],
    }
    vi.spyOn(commands, 'getPreferences').mockResolvedValueOnce(prefs(false))
    const save = vi.spyOn(commands, 'savePreferences').mockImplementation(async (next) => next)
    const catalogSpy = vi.spyOn(commands, 'listModelCatalog').mockResolvedValue(emptyCatalog)

    const { result } = renderHook(
      () => ({
        prefs: usePreferences(),
        catalog: useModelCatalog(),
        editor: useEditPreferences(),
      }),
      {
        wrapper: ({ children }: { children: ReactNode }) =>
          createElement(QueryClientProvider, { client: queryClient }, children),
      },
    )

    await waitFor(() => expect(result.current.prefs.data).toEqual(prefs(false)))
    await waitFor(() => expect(catalogSpy).toHaveBeenCalledTimes(1))

    // Unrelated edit with vision unchanged must not invalidate modelCatalogKey.
    act(() => {
      result.current.editor.edit({
        ...prefs(false),
        renderer: { styles: { antique: { Latn: { family: 'AnimeAce' } } } },
      })
      result.current.editor.flush()
    })
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1))
    expect(catalogSpy).toHaveBeenCalledTimes(1)

    // Toggling translator.vision via useEditPreferences must invalidate modelCatalogKey once saved.
    act(() => {
      result.current.editor.edit(prefs(true))
      result.current.editor.flush()
    })
    await waitFor(() => expect(save).toHaveBeenCalledTimes(2))
    await waitFor(() => expect(catalogSpy).toHaveBeenCalledTimes(2))
  })
})

describe('importPaths', () => {
  it('shows a warning toast summarizing skipped unsupported files', async () => {
    const add = vi.spyOn(toast, 'add')
    vi.spyOn(commands, 'importPaths').mockResolvedValueOnce({
      added_pages: 2,
      skipped_files: ['/tmp/a.txt', '/tmp/b.md', '/tmp/c.doc', '/tmp/d.bin'],
    })

    await importPaths(['/tmp/page1.png', '/tmp/a.txt'], 1)

    expect(commands.importPaths).toHaveBeenCalledWith(['/tmp/page1.png', '/tmp/a.txt'], 1)
    expect(add).toHaveBeenCalledWith({
      type: 'warning',
      title: 'Skipped 4 unsupported files',
      description: 'a.txt, b.md, c.doc, and 1 more',
    })
  })
})
