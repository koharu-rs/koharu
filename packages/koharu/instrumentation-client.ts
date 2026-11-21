import * as Sentry from '@sentry/nextjs'

import { tauriOptions, traceHeaders } from '@/lib/sentry'
import { setRunner } from '@koharu/ipc/protocol'

// Everything the SDK records, sent through the Rust SDK, which has the DSN, the release,
// and the user. Koharu reports page content, so replays aren't masked. The feedback form
// opens from the Help menu with the interface's own strings.
Sentry.init({
  ...tauriOptions,
  integrations: (defaults) => [
    ...tauriOptions.integrations(defaults),
    Sentry.replayIntegration({ maskAllText: false, maskAllInputs: false, blockAllMedia: false }),
    Sentry.feedbackIntegration({ autoInject: false, colorScheme: 'system' }),
    Sentry.consoleLoggingIntegration(),
  ],
  tracesSampleRate: 1,
  replaysSessionSampleRate: 1,
  replaysOnErrorSampleRate: 1,
})

// Every command runs in a span, timing its round trip, and carries that span's trace, which
// the backend's work for it continues.
setRunner((command, invoke) =>
  Sentry.startSpan({ name: command, op: 'ipc' }, () => invoke({ headers: traceHeaders() })),
)

export const onRouterTransitionStart = Sentry.captureRouterTransitionStart
