import { QueryClientProvider } from '@tanstack/react-query'
import { act, render as testingRender, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { ThemeProvider } from 'next-themes'
import { describe, expect, it, vi } from 'vitest'

import { OrcaRouterAuthorization } from '@/components/preferences/OrcaRouterConnection'
import { ProviderPreferences } from '@/components/preferences/ProviderPreferences'
import { queryClient } from '@/lib/queries'
import { commands, type ProviderPreferences as ProviderSettings } from '@koharu/bridge/protocol'

function render(ui: React.ReactNode) {
  return testingRender(<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>)
}

const emptyCredential = () => ({ configured: false, value: null, clear: false })

function orcaRouterSettings(overrides?: Partial<ProviderSettings['entries'][number]>) {
  const entry = {
    name: 'OrcaRouter',
    config: { provider: 'orcarouter' as const, settings: {} },
    credential: emptyCredential(),
    ...overrides,
  }
  const settings: ProviderSettings = {
    entries: [
      {
        name: 'OpenAI',
        config: { provider: 'openai', settings: {} },
        credential: emptyCredential(),
      },
      entry,
    ],
  }
  return { settings, entry }
}

describe('OrcaRouter connection settings', () => {
  it('offers an API-key field and a browser authorization as two separate choices', async () => {
    const { settings } = orcaRouterSettings()
    render(
      <ThemeProvider attribute='class'>
        <ProviderPreferences value={settings} onChange={() => undefined} />
      </ThemeProvider>,
    )

    // Both entries are present and independently labelled.
    const keyField = screen.getByLabelText('OrcaRouter - API credential')
    expect(keyField).toBeInTheDocument()
    // The repo masks credential input with text-security disc rather than a
    // password field, so the value is never rendered in the clear.
    expect(keyField).toHaveClass('[-webkit-text-security:disc]')
    expect(keyField).toHaveAttribute('type', 'text')
    expect(screen.getByRole('button', { name: 'Connect with OrcaRouter' })).toBeEnabled()

    // The pasted key never renders in the clear.
    expect(keyField).toHaveAttribute('autocomplete', 'off')
  })

  it('keeps the API-key entry usable without ever starting an authorization', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    const connect = vi.spyOn(commands, 'connectOrcaRouter')
    const { settings } = orcaRouterSettings()
    render(
      <ThemeProvider attribute='class'>
        <ProviderPreferences value={settings} onChange={onChange} />
      </ThemeProvider>,
    )

    await user.type(screen.getByLabelText('OrcaRouter - API credential'), 'sk-orca-test-pasted')
    expect(onChange).toHaveBeenCalled()
    // Storing a key must not kick off a PKCE login.
    expect(connect).not.toHaveBeenCalled()
  })

  it('runs the authorization through the connect command when the user chooses it', async () => {
    const user = userEvent.setup()
    const connect = vi
      .spyOn(commands, 'connectOrcaRouter')
      .mockResolvedValue({ authorization_url: 'https://www.orcarouter.ai/auth?state=x' })
    const onConnected = vi.fn()
    render(<OrcaRouterAuthorization connect={() => connect()} onConnected={onConnected} />)

    await user.click(screen.getByRole('button', { name: 'Connect with OrcaRouter' }))
    await waitFor(() => expect(connect).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(onConnected).toHaveBeenCalled())
    expect(
      screen.getByRole('link', { name: 'https://www.orcarouter.ai/auth?state=x' }),
    ).toBeInTheDocument()
  })

  it('releases the busy state when the user denies and reports it', async () => {
    const user = userEvent.setup()
    const connect = vi.fn().mockRejectedValue(new Error('access_denied'))
    const cancel = vi.fn().mockResolvedValue(undefined)
    render(<OrcaRouterAuthorization connect={connect} cancel={cancel} />)

    await user.click(screen.getByRole('button', { name: 'Connect with OrcaRouter' }))
    await waitFor(() => expect(screen.getByRole('alert')).toBeInTheDocument())
    // Not left permanently busy after a denial.
    expect(screen.getByRole('button', { name: 'Connect with OrcaRouter' })).toBeEnabled()
  })

  it('surfaces a reauthentication requirement without claiming a refresh', async () => {
    render(<OrcaRouterAuthorization needsReauth />)
    expect(screen.getByRole('alert')).toHaveTextContent(/reconnect/i)
  })

  it('cancels an in-flight authorization when the page is hidden', async () => {
    const user = userEvent.setup()
    const connect = vi.fn().mockResolvedValue({ authorization_url: 'https://example.test/auth' })
    const cancel = vi.fn().mockResolvedValue(undefined)
    render(<OrcaRouterAuthorization connect={connect} cancel={cancel} />)

    await user.click(screen.getByRole('button', { name: 'Connect with OrcaRouter' }))
    await waitFor(() => expect(connect).toHaveBeenCalled())

    // The back-forward cache restores the page without remounting, so busy state
    // and the hint must clear in the pagehide handler itself.
    act(() => {
      window.dispatchEvent(new Event('pagehide'))
    })
    await waitFor(() => expect(cancel).toHaveBeenCalled())
    expect(screen.getByRole('button', { name: 'Connect with OrcaRouter' })).toBeEnabled()

    // A second login can start without remounting the component.
    await user.click(screen.getByRole('button', { name: 'Connect with OrcaRouter' }))
    await waitFor(() => expect(connect).toHaveBeenCalledTimes(2))
  })

  it('cancels explicitly without leaving the control busy', async () => {
    const user = userEvent.setup()
    let finish: ((value: { authorization_url?: string }) => void) | undefined
    const pending = new Promise<{ authorization_url?: string }>((resolve) => {
      finish = resolve
    })
    const connect = vi.fn().mockImplementation(() => pending)
    const cancel = vi.fn().mockResolvedValue(undefined)
    render(<OrcaRouterAuthorization connect={connect} cancel={cancel} />)

    await user.click(screen.getByRole('button', { name: 'Connect with OrcaRouter' }))
    // The authorization is still in flight, so the cancel control is available.
    const stop = await screen.findByRole('button', { name: 'Cancel authorization' })
    await user.click(stop)
    await waitFor(() => expect(cancel).toHaveBeenCalled())
    expect(screen.getByRole('button', { name: 'Connect with OrcaRouter' })).toBeEnabled()
    // A late completion of the cancelled attempt must not resurrect the control.
    await act(async () => {
      finish?.({ authorization_url: 'https://example.test/late' })
      await pending
    })
    expect(screen.queryByRole('link', { name: 'https://example.test/late' })).toBeNull()
  })

  it('ignores a stale authorization response after a newer attempt starts', async () => {
    const user = userEvent.setup()
    let resolveFirst: ((value: { authorization_url: string }) => void) | undefined
    const first = new Promise<{ authorization_url: string }>((resolve) => {
      resolveFirst = resolve
    })
    const connect = vi
      .fn()
      .mockImplementationOnce(() => first)
      .mockResolvedValueOnce({ authorization_url: 'https://example.test/second' })
    render(
      <OrcaRouterAuthorization connect={connect} cancel={vi.fn().mockResolvedValue(undefined)} />,
    )

    await user.click(screen.getByRole('button', { name: 'Connect with OrcaRouter' }))
    act(() => {
      window.dispatchEvent(new Event('pagehide'))
    })
    await user.click(screen.getByRole('button', { name: 'Connect with OrcaRouter' }))
    await waitFor(() => expect(connect).toHaveBeenCalledTimes(2))

    await act(async () => {
      resolveFirst?.({ authorization_url: 'https://example.test/stale' })
      await first
    })
    // The late response from the invalidated attempt must not surface.
    await waitFor(() =>
      expect(screen.getByRole('link', { name: 'https://example.test/second' })).toBeInTheDocument(),
    )
    expect(screen.queryByRole('link', { name: 'https://example.test/stale' })).toBeNull()
  })

  it('clears a stored key through the shared credential path', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    const { settings } = orcaRouterSettings({
      credential: { configured: true, value: null, clear: false },
    })
    render(
      <ThemeProvider attribute='class'>
        <ProviderPreferences value={settings} onChange={onChange} />
      </ThemeProvider>,
    )

    await user.click(screen.getByRole('button', { name: 'Clear OrcaRouter - API credential' }))
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        entries: expect.arrayContaining([
          expect.objectContaining({
            credential: { configured: false, value: null, clear: true },
          }),
        ]),
      }),
    )
  })

  it('does not render the authorization control for other providers', () => {
    const settings: ProviderSettings = {
      entries: [
        {
          name: 'OpenAI',
          config: { provider: 'openai', settings: {} },
          credential: emptyCredential(),
        },
      ],
    }
    render(
      <ThemeProvider attribute='class'>
        <ProviderPreferences value={settings} onChange={() => undefined} />
      </ThemeProvider>,
    )
    expect(screen.queryByRole('button', { name: 'Connect with OrcaRouter' })).toBeNull()
  })
})
