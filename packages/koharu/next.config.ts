import { withSentryConfig } from '@sentry/nextjs/config'
import type { NextConfig } from 'next'

const nextConfig: NextConfig = {
  devIndicators: false,
  transpilePackages: ['@koharu/bridge', '@koharu/ui'],
  output: 'export',
  images: {
    unoptimized: true,
  },
}

export default withSentryConfig(nextConfig, {
  org: process.env.SENTRY_ORG,
  // The interface's events reach Sentry through the Rust SDK, so its source maps go to
  // the Rust project, where they resolve.
  project: 'rust',
  authToken: process.env.SENTRY_AUTH_TOKEN,
  silent: !process.env.CI,
})
