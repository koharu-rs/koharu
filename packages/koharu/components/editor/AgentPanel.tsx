'use client'

import { Channel } from '@tauri-apps/api/core'
import { Bot, CircleStop, Send } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call, savePreferences } from '@/lib/backend'
import { trace } from '@/lib/sentry'
import { useKoharuStore } from '@/lib/store'
import { modelName, REASONING } from '@/lib/translation'
import { commands, type AgentConfig, type AgentEvent } from '@koharu/bridge/protocol'
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

/** The desktop's MCP server, which serves the open project from launch. */
const MCP_URL = 'http://127.0.0.1:7878/mcp'

interface ConversationMessage {
  id: string
  role: 'user' | 'assistant'
  text: string
  error?: boolean
}

/** The in-app agent's conversation about the open project, its model, and the MCP server that
 * serves the same tools to agents in other programs. */
export function AgentPanel() {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<ConversationMessage[]>([])
  const [prompt, setPrompt] = useState('')
  const [running, setRunning] = useState<number | null>(null)
  const [activity, setActivity] = useState<string | null>(null)
  const models = useKoharuStore((state) => state.models).filter((model) => model.tools)
  const preferences = useKoharuStore((state) => state.preferences)

  if (!preferences) {
    return (
      <div className='grid min-h-0 flex-1 place-items-center p-6 text-[11px] text-muted-foreground'>
        {t('agent.loading')}
      </div>
    )
  }

  const config = preferences.agent
  const model = config.model ?? null

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <div className='grid shrink-0 gap-2 border-b px-3 py-2.5'>
        <div className='grid grid-cols-[minmax(0,1fr)_7rem] gap-1.5'>
          <Select
            value={model ?? ''}
            items={Object.fromEntries(models.map((choice) => [choice.reference, choice.name]))}
            onValueChange={(next) => next && void saveConfig({ ...config, model: next })}
          >
            <SelectTrigger size='sm' className='w-full min-w-0' aria-label={t('agent.model')}>
              <SelectValue placeholder={t('inference.noModel')} />
            </SelectTrigger>
            <SelectContent align='start'>
              {models.map((choice) => (
                <SelectItem key={choice.reference} value={choice.reference}>
                  {choice.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={config.reasoning ?? 'default'}
            items={Object.fromEntries([
              ['default', t('inference.providerDefault')],
              ...REASONING.map((level) => [level, t(`reasoning.${level}`)]),
            ])}
            onValueChange={(reasoning) =>
              reasoning &&
              void saveConfig({
                ...config,
                reasoning: reasoning === 'default' ? null : reasoning,
              })
            }
          >
            <SelectTrigger size='sm' className='w-full min-w-0' aria-label={t('agent.reasoning')}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent align='end'>
              <SelectItem value='default'>{t('inference.providerDefault')}</SelectItem>
              {REASONING.map((level) => (
                <SelectItem key={level} value={level}>
                  {t(`reasoning.${level}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className='flex min-w-0 items-center gap-2 text-[10px] text-muted-foreground'>
          <span className='shrink-0'>{t('agent.mcp')}</span>
          <span className='min-w-0 truncate font-mono text-foreground select-text'>{MCP_URL}</span>
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
              {model ? modelName(models, model) : t('inference.noModel')}
            </Badge>
            {running !== null ? (
              <Button
                variant='outline'
                size='icon-sm'
                aria-label={t('agent.cancel')}
                onClick={cancel}
              >
                <CircleStop />
              </Button>
            ) : (
              <Button
                size='icon-sm'
                aria-label={t('agent.send')}
                disabled={!prompt.trim() || !model}
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

  async function saveConfig(agent: AgentConfig) {
    if (preferences) await savePreferences({ ...preferences, agent })
  }

  function cancel() {
    if (running === null) return
    void call(commands.cancelAgent, running).catch(() => undefined)
    // A cancelled run tells nothing more.
    setRunning(null)
    setActivity(null)
    setMessages((current) =>
      current.map((item, index) =>
        index === current.length - 1 && item.role === 'assistant'
          ? { ...item, text: item.text || t('agent.cancelled') }
          : item,
      ),
    )
  }

  async function send() {
    const message = prompt.trim()
    if (!message || running !== null) return
    const id = crypto.randomUUID()
    const assistant = crypto.randomUUID()
    setPrompt('')
    setMessages((current) => [
      ...current,
      { id, role: 'user', text: message },
      { id: assistant, role: 'assistant', text: '' },
    ])
    setActivity(t('agent.working'))
    // A run can end before its id arrives.
    let settled = false

    const channel = new Channel<AgentEvent>()
    channel.onmessage = (update) => {
      if (update.type === 'completed') {
        settled = true
        updateAssistant(() => update.answer)
        setRunning(null)
        setActivity(null)
        return
      }
      if (update.type === 'failed') {
        settled = true
        updateAssistant(() => update.error, true)
        setRunning(null)
        setActivity(null)
        return
      }
      const { event } = update
      switch (event.type) {
        case 'text':
          updateAssistant((text) => text + event.delta)
          break
        case 'reasoning':
          setActivity(t('agent.thinking'))
          break
        case 'tool_started':
          setActivity(t('agent.applying'))
          break
        case 'tool_finished':
          setActivity(t('agent.working'))
          break
      }
    }
    try {
      const run = await call(commands.runAgent, message, channel, trace())
      if (!settled) setRunning(run)
    } catch (error) {
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
