// Sentry's Tauri plugin, the interface's half: its events, breadcrumbs, and scope go to
// the Rust SDK (crates/koharu-sentry), which sends them with its own release, user, and
// context. Vendored from getsentry/sentry-tauri js/index.ts at
// f83b7225301e327884c3ba62a51bba6d324c3871 (v0.9.0). Divergence: the app starts
// @sentry/nextjs itself (instrumentation-client.ts) with these options, so nothing is
// injected, and `trace` continues the interface's trace in the backend.
import type {
  BaseTransportOptions,
  Breadcrumb,
  Transport,
  TransportMakeRequestResponse,
  TransportRequest,
} from '@sentry/core'
import { createTransport, getTraceData, type BrowserOptions } from '@sentry/nextjs'
import { invoke } from '@tauri-apps/api/core'

import { normalizeUrlsIntegration } from './normalize'
import { scopeToRustIntegration } from './scope'

// If IPC fails this is usually due to permissions. We quit sending because we
// can't recover from this and we don't want to spam the console with errors.
let ipcFailed = false

/** Creates a Transport that passes envelopes to the Tauri Rust process. */
function makeRendererTransport(options: BaseTransportOptions): Transport {
  return createTransport(
    options,
    async (request: TransportRequest): Promise<TransportMakeRequestResponse> => {
      if (ipcFailed) {
        return { statusCode: 200 }
      }

      try {
        await invoke('plugin:sentry|envelope', { envelope: request.body })
      } catch (e) {
        console.error('Failed to send envelope to Rust:', e)
        ipcFailed = true
      }

      // Since the Rust process handles sending of envelopes and rate limiting, we always
      // return 200 OK to the renderers.
      return { statusCode: 200 }
    },
  )
}

/** A `beforeBreadcrumb` hook that sends the breadcrumb to the Rust process via Tauri invoke. */
function sendBreadcrumbToRust(breadcrumb: Breadcrumb): Breadcrumb | null {
  if (ipcFailed) {
    return null
  }

  // Ignore IPC breadcrumbs otherwise we'll make an infinite loop
  if (
    typeof breadcrumb.data?.url === 'string' &&
    (breadcrumb.data.url.startsWith('ipc://') ||
      breadcrumb.data.url.match(/^https?:\/\/ipc\.localhost/))
  ) {
    return null
  }

  invoke('plugin:sentry|breadcrumb', { breadcrumb }).catch((e) => {
    console.error('Failed to send breadcrumb to Rust:', e)
    ipcFailed = true
  })

  // We don't collect breadcrumbs in the renderer since they are passed to Rust
  return null
}

/** The options that send the interface's events through the Rust SDK. */
export const tauriOptions = {
  // We don't send from the browser but a DSN is required for the SDK to start
  dsn: 'https://123456@dummy.dsn/0',
  // We want to track app sessions rather than browser sessions
  integrations: (integrations) => [
    ...integrations.filter((i) => i.name !== 'BrowserSession'),
    scopeToRustIntegration(),
    normalizeUrlsIntegration(),
  ],
  transport: makeRendererTransport,
  beforeBreadcrumb: sendBreadcrumbToRust,
  // The Rust SDK cannot parse streamed span envelope items
  traceLifecycle: 'static',
} satisfies BrowserOptions

/** The current trace as a `sentry-trace` value, for a command whose backend work continues
 * it. */
export function trace(): string | null {
  return getTraceData()['sentry-trace'] ?? null
}
