'use client'

import '@/lib/i18n'
import * as Sentry from '@sentry/nextjs'
import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'

export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string }
  reset: () => void
}) {
  const { t, i18n } = useTranslation()

  useEffect(() => {
    Sentry.captureException(error)
  }, [error])

  return (
    <html lang={i18n.resolvedLanguage ?? i18n.language}>
      <body className='grid min-h-screen place-items-center bg-background text-foreground'>
        <main className='max-w-lg rounded-xl border border-border bg-card p-6 text-center'>
          <h1 className='text-lg font-semibold'>{t('errors.fatalTitle')}</h1>
          <p className='mt-2 text-sm text-muted-foreground'>{error.message}</p>
          <button
            className='mt-4 rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground'
            onClick={reset}
          >
            {t('errors.tryAgain')}
          </button>
        </main>
      </body>
    </html>
  )
}
