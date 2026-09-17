import { QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { createElement, type ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { queryClient, useImportPages } from '@/lib/queries'
import { commands } from '@koharu/bridge'

function wrapper({ children }: { children: ReactNode }) {
  return createElement(QueryClientProvider, { client: queryClient }, children)
}

function jsonResponse(body = 'null') {
  return {
    ok: true,
    status: 200,
    headers: { get: () => 'application/json' },
    text: async () => body,
    arrayBuffer: async () => new ArrayBuffer(0),
    blob: async () => new Blob(),
  }
}

function binaryResponse(bytes: Uint8Array<ArrayBuffer>, contentType: string, disposition?: string) {
  return {
    ok: true,
    status: 200,
    headers: {
      get: (name: string) => {
        if (name.toLowerCase() === 'content-type') return contentType
        if (name.toLowerCase() === 'content-disposition') return disposition ?? null
        return null
      },
    },
    text: async () => '',
    arrayBuffer: async () =>
      bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
    blob: async () => new Blob([bytes], { type: contentType }),
  }
}

type MockFetchInit = {
  method?: string
  headers?: Record<string, string>
  body?: FormData | string
}

type MockFetchResponse =
  | ReturnType<typeof jsonResponse>
  | ReturnType<typeof binaryResponse>
  | {
      ok: false
      status: number
      statusText: string
      text: () => Promise<string>
    }

describe('browser import and export', () => {
  const fetchMock =
    vi.fn<(input: RequestInfo | URL, init: MockFetchInit) => Promise<MockFetchResponse>>()
  let createdInputs: HTMLInputElement[]

  beforeEach(() => {
    createdInputs = []
    vi.stubGlobal('fetch', fetchMock)
    vi.spyOn(HTMLInputElement.prototype, 'click').mockImplementation(
      function (this: HTMLInputElement) {
        createdInputs.push(this)
      },
    )
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('opens a picker and posts multipart when paths is null', async () => {
    const file = new File(['page-bytes'], 'page.png', { type: 'image/png' })
    fetchMock.mockResolvedValue(jsonResponse())
    const { result } = renderHook(() => useImportPages(), { wrapper })

    act(() => result.current.importPages('files', null))
    await waitFor(() => expect(createdInputs).toHaveLength(1))
    expect(createdInputs[0]).toMatchObject({ type: 'file', multiple: true })
    expect(fetchMock).not.toHaveBeenCalled()

    Object.defineProperty(createdInputs[0], 'files', {
      configurable: true,
      value: [file],
    })
    await act(async () => {
      createdInputs[0].dispatchEvent(new Event('change'))
    })
    await waitFor(() => expect(fetchMock).toHaveBeenCalledOnce())

    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/rpc/import')
    expect(init.method).toBe('POST')
    expect(init.headers?.['Content-Type']).toBeUndefined()
    expect(init.body).toBeInstanceOf(FormData)
    const form = init.body as FormData
    expect(form.get('source')).toBe('files')
    const uploaded = form.getAll('files')
    expect(uploaded).toHaveLength(1)
    expect((uploaded[0] as File).name).toBe('page.png')
    await expect((uploaded[0] as File).text()).resolves.toBe('page-bytes')
  })

  it('does nothing when the browser file picker is cancelled', async () => {
    const { result } = renderHook(() => useImportPages(), { wrapper })

    act(() => result.current.importPages('folder', null))
    await waitFor(() => expect(createdInputs).toHaveLength(1))
    await act(async () => {
      createdInputs[0].dispatchEvent(new Event('cancel'))
    })

    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('downloads an export blob when destination is null', async () => {
    const bytes = new Uint8Array([137, 80, 78, 71])
    fetchMock.mockResolvedValue(
      binaryResponse(bytes, 'image/png', 'attachment; filename="chapter.png"'),
    )
    const downloads: Array<{ href: string; name: string }> = []
    vi.mocked(HTMLAnchorElement.prototype.click).mockImplementation(
      function (this: HTMLAnchorElement) {
        downloads.push({ href: this.href, name: this.download })
      },
    )

    await commands.export('png', null)

    expect(fetchMock).toHaveBeenCalledOnce()
    expect(fetchMock.mock.calls[0][0]).toBe('/rpc/export')
    expect(JSON.parse(fetchMock.mock.calls[0][1].body as string)).toEqual({
      format: 'png',
      destination: null,
    })
    expect(URL.createObjectURL).toHaveBeenCalledOnce()
    expect(downloads).toEqual([{ href: 'blob:koharu-thumbnail', name: 'chapter.png' }])
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:koharu-thumbnail')
  })

  it('does not download when export returns no output', async () => {
    fetchMock.mockResolvedValue(jsonResponse('null'))

    await expect(commands.export('cbz', null)).resolves.toBeNull()

    expect(URL.createObjectURL).not.toHaveBeenCalled()
    expect(HTMLAnchorElement.prototype.click).not.toHaveBeenCalled()
  })
})
