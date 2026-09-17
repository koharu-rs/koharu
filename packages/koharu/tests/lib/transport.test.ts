import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { Channel, commands } from '@koharu/bridge'

class FakeWebSocket {
  static instances: FakeWebSocket[] = []
  url: string
  onmessage: ((event: { data: string }) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  onopen: ((event: Event) => void) | null = null
  onclose: ((event: Event) => void) | null = null
  readyState = 1

  constructor(url: string) {
    this.url = url
    FakeWebSocket.instances.push(this)
  }

  send() {}
  close() {
    if (this.readyState === 3) return
    this.readyState = 3
    this.onclose?.({} as Event)
  }

  emit(message: unknown) {
    this.onmessage?.({ data: JSON.stringify(message) })
  }
}

describe('HTTP/WS transport', () => {
  const fetchMock = vi.fn()

  beforeEach(() => {
    FakeWebSocket.instances = []
    vi.stubGlobal('fetch', fetchMock)
    vi.stubGlobal('WebSocket', FakeWebSocket)
  })

  afterEach(() => {
    for (const socket of FakeWebSocket.instances) socket.close()
    vi.unstubAllGlobals()
  })

  it('posts process JSON path and body', async () => {
    fetchMock.mockResolvedValue(jsonResponse({ body: JSON.stringify('job-1') }))

    await expect(commands.process({ scope: 'project' }, { operation: 'full' })).resolves.toBe(
      'job-1',
    )

    expect(fetchMock).toHaveBeenCalledOnce()
    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/rpc/process')
    expect(init).toMatchObject({
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
    })
    expect(JSON.parse(init.body)).toEqual({
      scope: { scope: 'project' },
      operation: { operation: 'full' },
    })
  })

  it('posts multi-word camelCase fields', async () => {
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      headers: { get: () => 'application/octet-stream' },
      arrayBuffer: async () => new Uint8Array([1, 2]).buffer,
      text: async () => '',
    })

    await expect(commands.getFontPreview('Inter')).resolves.toEqual([1, 2])
    expect(fetchMock.mock.calls[0][0]).toBe('/rpc/get_font_preview')
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ familyName: 'Inter' })

    fetchMock.mockResolvedValue(
      jsonResponse({ body: JSON.stringify({ revision: 1, layer: 'layer' }) }),
    )
    await commands.commitPaint(3, null, [{ x: 1, y: 2 }], {
      diameter: 4,
      color: [1, 2, 3, 4],
    })
    expect(fetchMock.mock.calls[1][0]).toBe('/rpc/commit_paint')
    expect(JSON.parse(fetchMock.mock.calls[1][1].body)).toEqual({
      expectedRevision: 3,
      layer: null,
      points: [{ x: 1, y: 2 }],
      brush: { diameter: 4, color: [1, 2, 3, 4] },
    })
  })

  it('posts zero-payload commands without a JSON body', async () => {
    fetchMock.mockResolvedValue(jsonResponse({ body: 'null' }))
    await expect(commands.getProject()).resolves.toBeNull()
    expect(fetchMock.mock.calls[0][0]).toBe('/rpc/get_project')
    expect(fetchMock.mock.calls[0][1]).toEqual({ method: 'POST' })
  })

  it('fans subscribe envelopes to one websocket', async () => {
    const onJob = vi.fn()
    const pending = commands.subscribe(
      new Channel(),
      new Channel(onJob),
      new Channel(),
      new Channel(),
      new Channel(),
    )
    expect(FakeWebSocket.instances).toHaveLength(1)
    expect(FakeWebSocket.instances[0].url).toMatch(/\/rpc\/subscribe$/)

    const startup = startupState()
    FakeWebSocket.instances[0].emit({ channel: 'startup', payload: startup })
    await expect(pending).resolves.toEqual(startup)

    FakeWebSocket.instances[0].emit({
      channel: 'job',
      payload: { id: 'job', state: 'running' },
    })
    expect(onJob).toHaveBeenCalledWith({ id: 'job', state: 'running' })
  })

  it('delivers agent envelopes to exported Channel sinks', async () => {
    const onLogin = vi.fn()
    const onEvent = vi.fn()
    const pending = commands.subscribe(
      new Channel(),
      new Channel(),
      new Channel(),
      new Channel(),
      new Channel(),
    )
    FakeWebSocket.instances[0].emit({ channel: 'startup', payload: startupState() })
    await pending

    fetchMock.mockResolvedValue(jsonResponse({ body: JSON.stringify({ account: null }) }))
    await commands.loginAgent(new Channel(onLogin))
    FakeWebSocket.instances[0].emit({
      channel: 'agent_login',
      payload: { type: 'progress', message: 'hi' },
    })
    expect(onLogin).toHaveBeenCalledWith({ type: 'progress', message: 'hi' })

    fetchMock.mockResolvedValue(jsonResponse({ body: JSON.stringify('run-1') }))
    await commands.runAgent('do it', new Channel(onEvent))
    FakeWebSocket.instances[0].emit({
      channel: 'agent_event',
      payload: { type: 'started', run: 'run-1' },
    })
    expect(onEvent).toHaveBeenCalledWith({ type: 'started', run: 'run-1' })
  })

  it('rejects subscribe when the startup envelope carries an error', async () => {
    const pending = commands.subscribe(
      new Channel(),
      new Channel(),
      new Channel(),
      new Channel(),
      new Channel(),
    )
    FakeWebSocket.instances[0].emit({ channel: 'startup', error: 'no active subscribe session' })
    await expect(pending).rejects.toThrow(/no active subscribe session/)
  })
})

function startupState() {
  return {
    preferences: null,
    jobs: [],
    canvas: { page: null, revision: null, generation: 0, size: [0, 0], element_frames: [] },
  }
}

function jsonResponse({ body, status = 200 }: { body: string; status?: number }) {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: { get: () => 'application/json' },
    text: async () => body,
    arrayBuffer: async () => new ArrayBuffer(0),
  }
}
