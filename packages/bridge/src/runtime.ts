const rpcPrefix = '/rpc'

export class Channel<T = unknown> {
  onmessage: (response: T) => void

  constructor(onmessage?: (response: T) => void) {
    this.onmessage = onmessage ?? (() => {})
  }
}

type Json = Record<string, unknown>
type ChannelMap = Record<string, Channel<unknown>>

let nativeDialogs = false
let activeSocket: WebSocket | null = null
const channelSinks: ChannelMap = {}

export function invoke<T>(name: string, args: Json = {}): Promise<T> {
  const body: Json = {}
  for (const [key, value] of Object.entries(args)) {
    if (value instanceof Channel) {
      channelSinks[channelName(name, key)] = value as Channel<unknown>
    } else {
      body[key] = value
    }
  }
  if (name === 'subscribe') {
    return subscribe() as Promise<T>
  }
  if (name === 'import') {
    return httpImport(body.source as string, body.paths as string[] | null) as Promise<T>
  }
  if (name === 'export') {
    return httpExport(body.format as string, body.destination as string | null) as Promise<T>
  }
  return httpInvoke<T>(name, body)
}

async function httpInvoke<T>(name: string, body: Json): Promise<T> {
  const request: RequestInit = { method: 'POST' }
  if (Object.keys(body).length > 0) {
    request.headers = { 'Content-Type': 'application/json' }
    request.body = JSON.stringify(body)
  }
  const response = await fetch(`${rpcPrefix}/${name}`, request)
  return parseResponse<T>(response)
}

async function parseResponse<T>(response: Response): Promise<T> {
  if (!response.ok) {
    throw new Error(await readError(response))
  }
  if (response.status === 204) {
    return undefined as T
  }
  const contentType = response.headers.get('content-type') ?? ''
  if (contentType.includes('application/json')) {
    return JSON.parse(await response.text()) as T
  }
  const buffer = await response.arrayBuffer()
  return Array.from(new Uint8Array(buffer)) as T
}

async function readError(response: Response): Promise<string> {
  const text = await response.text()
  try {
    const json = JSON.parse(text) as { error?: string }
    if (json.error) {
      return json.error
    }
  } catch {
    // fall through
  }
  return text || response.statusText
}

function subscribe(): Promise<unknown> {
  activeSocket?.close()
  return new Promise((resolve, reject) => {
    const protocol = location.protocol === 'https:' ? 'wss' : 'ws'
    const socket = new WebSocket(`${protocol}://${location.host}${rpcPrefix}/subscribe`)
    activeSocket = socket
    socket.onmessage = (event) => {
      const payload = JSON.parse(String(event.data)) as {
        channel?: string
        error?: string
        payload?: unknown
      }
      if (payload.error) {
        reject(new Error(payload.error))
        socket.close()
        return
      }
      if (payload.channel === 'startup') {
        const startup = payload.payload as { native_dialogs?: boolean } | undefined
        nativeDialogs = Boolean(startup?.native_dialogs)
        resolve(payload.payload)
        return
      }
      if (payload.channel) {
        channelSinks[payload.channel]?.onmessage(payload.payload)
      }
    }
    socket.onerror = () => {
      reject(new Error('subscribe websocket failed'))
    }
    socket.onclose = () => {
      if (activeSocket === socket) {
        activeSocket = null
      }
    }
  })
}

function channelName(command: string, parameter: string): string {
  const name = `${command}/${parameter}`
  switch (name) {
    case 'subscribe/onCanvas':
      return 'canvas'
    case 'subscribe/onJob':
      return 'job'
    case 'subscribe/onDownload':
      return 'download'
    case 'subscribe/onResources':
      return 'resources'
    case 'subscribe/onProject':
      return 'project'
    case 'login_agent/onEvent':
      return 'agent_login'
    case 'run_agent/onEvent':
      return 'agent_event'
    default:
      throw new Error(`unknown channel ${name}`)
  }
}

async function httpImport(source: string, paths: string[] | null): Promise<unknown> {
  if (paths === null && !nativeDialogs) {
    const files = await chooseImportFiles(source)
    if (files.length === 0) {
      return null
    }
    const form = new FormData()
    form.set('source', source)
    for (const file of files) {
      form.append('files', file)
    }
    const response = await fetch(`${rpcPrefix}/import`, { method: 'POST', body: form })
    return parseResponse(response)
  }
  return httpInvoke('import', { source, paths })
}

async function httpExport(format: string, destination: string | null): Promise<null> {
  const response = await fetch(`${rpcPrefix}/export`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ format, destination }),
  })
  if (!response.ok) {
    throw new Error(await readError(response))
  }
  const contentType = response.headers.get('content-type') ?? ''
  if (contentType.includes('application/json') || response.status === 204) {
    return null
  }
  const blob = await response.blob()
  const filename = filenameFromDisposition(response.headers.get('content-disposition')) ?? `export.${format}`
  downloadBlob(blob, filename)
  return null
}

async function chooseImportFiles(source: string): Promise<File[]> {
  return new Promise((resolve) => {
    const input = document.createElement('input')
    input.type = 'file'
    input.multiple = true
    if (source === 'folder') {
      input.webkitdirectory = true
    }
    input.onchange = () => resolve([...(input.files ?? [])])
    input.oncancel = () => resolve([])
    input.click()
  })
}

function filenameFromDisposition(header: string | null): string | null {
  if (!header) {
    return null
  }
  const match = /filename\*?=(?:UTF-8'')?("?)([^";]+)\1/i.exec(header)
  return match?.[2] ? decodeURIComponent(match[2]) : null
}

function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  anchor.click()
  URL.revokeObjectURL(url)
}
