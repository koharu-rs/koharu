import { QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { createElement, type ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { queryClient, send, usePreferences, useSavePreferences } from '@/lib/queries'
import { commands, type Preferences } from '@koharu/ipc/protocol'
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
    export: {},
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
})
