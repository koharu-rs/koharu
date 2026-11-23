import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { queryClient, send } from '@/lib/queries'
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
