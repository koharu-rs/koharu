// What the generated `protocol.ts` builds on: the headers every command sends, and a
// typed function for each command and each event, from Tauri's `invoke` and `listen`.
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

let headers: () => Record<string, string> = () => ({})

/** Sets what every command sends as its headers, such as the interface's trace. */
export function setHeaders(provide: () => Record<string, string>): void {
  headers = provide
}

/** The command `name` as a function of its arguments, which `keys` name in order, resolving
 * to its `Ok` value; an `Err` rejects the promise. The function is named `name`, which the
 * interface's span of each call takes. */
export function command<Args extends unknown[], Result>(
  name: string,
  keys: readonly string[],
): (...args: Args) => Promise<Result> {
  const call = (...args: Args) =>
    invoke<Result>(name, Object.fromEntries(keys.map((key, index) => [key, args[index]])), {
      headers: headers(),
    })
  return Object.defineProperty(call, 'name', { value: name })
}

/** The event `name` as a function of its handler, which hears its payload, resolving to the
 * function that stops it. */
export function event<Payload>(
  name: string,
): (handler: (payload: Payload) => void) => Promise<UnlistenFn> {
  return (handler) => listen<Payload>(name, (event) => handler(event.payload))
}
