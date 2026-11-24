'use client'

import { create } from 'zustand'

/** What a turn of the agent's showed, in the order it came. */
export type Part =
  | { type: 'text'; text: string }
  /** Its reasoning, with when it began and, once text or a tool follows, ended. */
  | { type: 'thinking'; text: string; started: number; ended: number | null }
  | {
      type: 'tool'
      tool: string
      arguments: unknown
      state: 'running' | 'done' | 'failed'
      error: string | null
    }

export type Turn =
  | { id: string; role: 'user'; text: string }
  | {
      id: string
      role: 'assistant'
      parts: Part[]
      state: 'running' | 'done' | 'failed' | 'cancelled'
      error?: string
    }

interface Conversation {
  turns: Turn[]
  /** The message being written. */
  prompt: string
  /** The run going on, once the desktop answers its id. */
  running: number | null
}

/** The open project's conversation with the agent, which goes when the project closes, as the
 * desktop's does. It outlives the panel, whose place settings and the reader take while a run goes
 * on. */
export const useConversation = create<Conversation>()(() => ({
  turns: [],
  prompt: '',
  running: null,
}))
