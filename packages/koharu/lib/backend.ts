'use client'

import * as Sentry from '@sentry/nextjs'

import { commands } from '@koharu/bridge/protocol'
import type { Preferences } from '@koharu/bridge/protocol'

import { receiveError, receiveModels, receivePreferences } from './store'

type Command<Args extends unknown[], Result> = (...args: Args) => Promise<Result>

let modelsRequest: Promise<void> | null = null
let modelsGeneration = 0
let preferencesWriteGeneration = 0
let preferencesWriteQueue: Promise<void> = Promise.resolve()

/** Runs `command` in a span named after it, so each command's round trip is timed. */
export async function call<Args extends unknown[], Result>(
  command: Command<Args, Result>,
  ...args: Args
): Promise<Result> {
  try {
    return await Sentry.startSpan({ name: command.name || 'command', op: 'ipc' }, () =>
      command(...args),
    )
  } catch (error) {
    throw report(error)
  }
}

export function dispatch<Args extends unknown[], Result>(
  command: Command<Args, Result>,
  ...args: Args
): void {
  void call(command, ...args).catch(() => undefined)
}

export async function refreshPreferences(): Promise<void> {
  const generation = preferencesWriteGeneration
  await preferencesWriteQueue
  const preferences = await call(commands.getPreferences)
  if (generation === preferencesWriteGeneration) receivePreferences(preferences)
}

/** Saves the preferences after any save before it, and keeps what the file holds. */
export function savePreferences(preferences: Preferences): Promise<Preferences> {
  preferencesWriteGeneration += 1
  const pending = preferencesWriteQueue
    .catch(() => undefined)
    .then(() => call(commands.savePreferences, preferences))
    .then((saved) => {
      receivePreferences(saved)
      return saved
    })
  preferencesWriteQueue = pending.then(
    () => undefined,
    () => undefined,
  )
  return pending
}

/** The models the translator and the agent can use, listed again when `force`. */
export function refreshModels(force = false): Promise<void> {
  if (!force && modelsRequest) return modelsRequest

  const generation = ++modelsGeneration
  const request = call(commands.getModels)
    .then((models) => {
      if (generation === modelsGeneration) receiveModels(models)
    })
    .finally(() => {
      if (modelsRequest === request) modelsRequest = null
    })
  modelsRequest = request
  return request
}

function report(error: unknown): Error {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === 'string'
        ? error
        : 'The native application returned an unknown error.'
  receiveError(message)
  return error instanceof Error ? error : new Error(message)
}
