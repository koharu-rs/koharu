'use client'

import { Bot, CircleStop, LogOut, Send, Sparkles } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { pageKey, pagesKey, projectKey, refresh } from '@/lib/queries'
import { Channel, commands } from '@koharu/bridge'
import type {
  AgentStatus,
  Config,
  Event,
  LoginEvent,
  Reasoning,
  RunId,
} from '@koharu/bridge/protocol'
import { Badge } from '@koharu/ui/components/badge'
import { Button } from '@koharu/ui/components/button'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'
import { Textarea } from '@koharu/ui/components/textarea'

interface ConversationMessage {
  id: string
  role: 'user' | 'assistant'
  text: string
  error?: boolean
}

export function AgentPanel() {
  const { t } = useTranslation()
  const [status, setStatus] = useState<AgentStatus | null>(null)
  const [loginEvent, setLoginEvent] = useState<LoginEvent | null>(null)
  const [loggingIn, setLoggingIn] = useState(false)
  const [messages, setMessages] = useState<ConversationMessage[]>([])
  const [prompt, setPrompt] = useState('')
  const [running, setRunning] = useState<RunId | null>(null)
  const [activity, setActivity] = useState<string | null>(null)

  useEffect(() => {
    void call(commands.getAgentStatus)
      .then(setStatus)
      .catch(() => undefined)
  }, [])

  const selectedModel = useMemo(
    () =>
      status?.models.find((model) => model.id === status.config.model) ?? status?.models[0] ?? null,
    [status],
  )

  if (!status) {
    return (
      <div className='grid min-h-0 flex-1 place-items-center p-6 text-[11px] text-muted-foreground'>
        {t('agent.loading')}
      </div>
    )
  }

  if (!status.account) {
    const code = loginEvent?.type === 'device_code' ? loginEvent.user_code : null
    return (
      <div className='flex min-h-0 flex-1 flex-col items-center justify-center p-6 text-center'>
        <div className='grid size-11 place-items-center rounded-2xl bg-primary/10 text-primary'>
          <Sparkles className='size-5' />
        </div>
        <h3 className='mt-4 text-sm font-semibold'>{t('agent.signInTitle')}</h3>
        <p className='mt-1 max-w-64 text-[11px] leading-5 text-muted-foreground'>
          {t('agent.signInDescription')}
        </p>
        {code ? (
          <div className='mt-4 w-full max-w-64 rounded-xl border bg-muted/40 p-3'>
            <p className='text-[10px] text-muted-foreground'>{t('agent.deviceCode')}</p>
            <p className='mt-1 font-mono text-lg font-semibold tracking-[0.18em] select-text'>
              {code}
            </p>
            <p className='mt-1 text-[10px] leading-4 text-muted-foreground'>
              {t('agent.deviceCodeHint')}
            </p>
          </div>
        ) : null}
        <Button className='mt-4 w-full max-w-64' disabled={loggingIn} onClick={() => void login()}>
          {loggingIn ? t('agent.signingIn') : t('agent.signIn')}
        </Button>
      </div>
    )
  }

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <div className='grid shrink-0 gap-2 border-b px-3 py-2.5'>
        <div className='flex min-w-0 items-center gap-2'>
          <div className='min-w-0 flex-1'>
            <p className='truncate text-[11px] font-medium'>{status.account.email ?? 'Codex'}</p>
            <p className='truncate text-[9px] text-muted-foreground'>
              {status.account.plan ?? t('agent.account')}
            </p>
          </div>
          <Button
            variant='ghost'
            size='icon-sm'
            aria-label={t('agent.signOut')}
            disabled={running !== null}
            onClick={() => void logout()}
          >
            <LogOut />
          </Button>
        </div>
        <div className='grid grid-cols-[minmax(0,1fr)_7rem] gap-1.5'>
          <Select
            value={status.config.model ?? 'automatic'}
            onValueChange={(model) => {
              const selected =
                status.models.find((candidate) => candidate.id === model) ?? status.models[0]
              const reasoning =
                selected &&
                selected.reasoning.length > 0 &&
                !selected.reasoning.includes(status.config.reasoning)
                  ? selected.reasoning[0]
                  : status.config.reasoning
              void saveConfig({
                model: model === 'automatic' ? null : model,
                reasoning,
              })
            }}
          >
            <SelectTrigger size='sm' className='w-full min-w-0' aria-label={t('agent.model')}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent align='start'>
              <SelectItem value='automatic'>{t('agent.automatic')}</SelectItem>
              {status.models.map((model) => (
                <SelectItem key={model.id} value={model.id}>
                  {model.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={status.config.reasoning}
            onValueChange={(reasoning) =>
              void saveConfig({ ...status.config, reasoning: reasoning as Reasoning })
            }
          >
            <SelectTrigger size='sm' className='w-full min-w-0' aria-label={t('agent.reasoning')}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent align='end'>
              {(selectedModel?.reasoning ?? [status.config.reasoning]).map((reasoning) => (
                <SelectItem key={reasoning} value={reasoning}>
                  {t(`agent.reasoningLevels.${reasoning}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      <ScrollArea className='min-h-0 flex-1'>
        {messages.length ? (
          <div className='grid gap-3 p-3'>
            {messages.map((message) => (
              <div
                key={message.id}
                data-role={message.role}
                className='flex data-[role=user]:justify-end'
              >
                <div
                  data-role={message.role}
                  data-error={message.error || undefined}
                  className='max-w-[90%] rounded-xl bg-muted px-3 py-2 text-[11px] leading-5 whitespace-pre-wrap select-text data-[error=true]:bg-destructive/10 data-[error=true]:text-destructive data-[role=user]:bg-primary data-[role=user]:text-primary-foreground'
                >
                  {message.text || (message.role === 'assistant' ? t('agent.working') : '')}
                </div>
              </div>
            ))}
            {activity ? (
              <div className='flex items-center gap-1.5 text-[10px] text-muted-foreground'>
                <span className='size-1.5 rounded-full bg-primary' />
                {activity}
              </div>
            ) : null}
          </div>
        ) : (
          <div className='grid min-h-64 place-items-center p-6 text-center'>
            <div>
              <Bot className='mx-auto size-6 text-primary' />
              <h3 className='mt-3 text-xs font-semibold'>{t('agent.emptyTitle')}</h3>
              <p className='mt-1 max-w-56 text-[10px] leading-4 text-muted-foreground'>
                {t('agent.emptyDescription')}
              </p>
            </div>
          </div>
        )}
      </ScrollArea>

      <div className='shrink-0 border-t p-2.5'>
        <div className='rounded-xl border bg-background p-1.5 focus-within:border-ring'>
          <Textarea
            id='agent-message'
            value={prompt}
            disabled={running !== null}
            aria-label={t('agent.message')}
            placeholder={t('agent.placeholder')}
            className='max-h-36 min-h-16 resize-none border-0 bg-transparent px-1.5 py-1 text-[11px] leading-5 shadow-none focus-visible:ring-0'
            onChange={(event) => setPrompt(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.shiftKey) {
                event.preventDefault()
                void send()
              }
            }}
          />
          <div className='flex items-center justify-between gap-2 pt-1'>
            <Badge
              variant='outline'
              className='h-5 max-w-44 truncate px-1.5 text-[9px] font-normal'
            >
              {selectedModel?.name ?? t('agent.automatic')}
            </Badge>
            {running ? (
              <Button
                variant='outline'
                size='icon-sm'
                aria-label={t('agent.cancel')}
                onClick={() => void call(commands.cancelAgent, running).catch(() => undefined)}
              >
                <CircleStop />
              </Button>
            ) : (
              <Button
                size='icon-sm'
                aria-label={t('agent.send')}
                disabled={!prompt.trim()}
                onClick={() => void send()}
              >
                <Send />
              </Button>
            )}
          </div>
        </div>
      </div>
    </div>
  )

  async function login() {
    setLoggingIn(true)
    setLoginEvent({ type: 'progress', message: t('agent.signingIn') })
    const channel = new Channel<LoginEvent>()
    channel.onmessage = setLoginEvent
    try {
      setStatus(await call(commands.loginAgent, channel))
      setLoginEvent(null)
    } finally {
      setLoggingIn(false)
    }
  }

  async function logout() {
    setStatus(await call(commands.logoutAgent))
    setMessages([])
    setActivity(null)
  }

  async function saveConfig(config: Config) {
    const saved = await call(commands.saveAgentConfig, config)
    setStatus((current) => (current ? { ...current, config: saved } : current))
  }

  async function send() {
    const message = prompt.trim()
    if (!message || running) return
    const id = crypto.randomUUID()
    const assistant = crypto.randomUUID()
    let settled = false
    setPrompt('')
    setMessages((current) => [
      ...current,
      { id, role: 'user', text: message },
      { id: assistant, role: 'assistant', text: '' },
    ])
    setActivity(t('agent.working'))

    const channel = new Channel<Event>()
    channel.onmessage = (event) => {
      switch (event.type) {
        case 'started':
          setRunning(event.run)
          break
        case 'text_delta':
          updateAssistant((text) => text + event.delta)
          break
        case 'reasoning_delta':
          setActivity(t('agent.thinking'))
          break
        case 'tool_started':
          setActivity(t('agent.applying'))
          break
        case 'tool_finished':
          if (event.changed) void refresh(projectKey, pagesKey, pageKey)
          setActivity(t('agent.working'))
          break
        case 'completed':
          settled = true
          updateAssistant(() => event.message)
          setRunning(null)
          setActivity(null)
          void refresh(projectKey, pagesKey, pageKey)
          break
        case 'failed':
          settled = true
          updateAssistant(() => event.message, true)
          setRunning(null)
          setActivity(null)
          break
        case 'cancelled':
          settled = true
          updateAssistant((text) => text || t('agent.cancelled'))
          setRunning(null)
          setActivity(null)
          break
      }
    }
    try {
      const run = await call(commands.runAgent, message, channel)
      if (!settled) setRunning(run)
    } catch (error) {
      settled = true
      updateAssistant(() => (error instanceof Error ? error.message : t('agent.failed')), true)
      setRunning(null)
      setActivity(null)
    }

    function updateAssistant(update: (text: string) => string, error = false) {
      setMessages((current) =>
        current.map((item) =>
          item.id === assistant
            ? { ...item, text: update(item.text), error: error || item.error }
            : item,
        ),
      )
    }
  }
}
