// Vendored from getsentry/sentry-tauri js/scope.ts at f83b7225301e327884c3ba62a51bba6d324c3871
// (v0.9.0).
import { getCurrentScope, getGlobalScope, getIsolationScope } from '@sentry/nextjs';
import type { Integration, ScopeData } from '@sentry/core';
import { invoke } from '@tauri-apps/api/core';

type Fields = 'user' | 'tags' | 'extra' | 'contexts';

/**
 * The changes since the last update. A `null` value removes the key from
 * the Rust scope, and a `null` user clears the user.
 */
export interface ScopeUpdate {
  user?: Record<string, unknown> | null;
  tags?: Record<string, unknown>;
  extra?: Record<string, unknown>;
  contexts?: Record<string, unknown>;
}

function mergedScope(): Pick<ScopeData, Fields> {
  const scopes = [getGlobalScope(), getIsolationScope(), getCurrentScope()].map((s) => s.getScopeData());
  const merge = <K extends Fields>(key: K) => Object.assign({}, ...scopes.map((s) => s[key])) as ScopeData[K];
  return { user: merge('user'), tags: merge('tags'), extra: merge('extra'), contexts: merge('contexts') };
}

function toJson(value: unknown): string | undefined {
  try {
    return JSON.stringify(value);
  } catch {
    return undefined;
  }
}

function diff(sent: Map<string, string>, next: Record<string, unknown>): Record<string, unknown> | undefined {
  const changes: Record<string, unknown> = {};
  const keys = new Set<string>();

  for (const [key, value] of Object.entries(next)) {
    const json = value === undefined ? undefined : toJson(value);
    if (json === undefined) {
      continue;
    }
    keys.add(key);
    if (sent.get(key) !== json) {
      sent.set(key, json);
      changes[key] = value;
    }
  }

  for (const key of sent.keys()) {
    if (!keys.has(key)) {
      sent.delete(key);
      changes[key] = null;
    }
  }

  return Object.keys(changes).length ? changes : undefined;
}

/**
 * Creates a tracker that returns only what changed since the last call.
 */
export function createScopeDiffer(): (scope: Pick<ScopeData, Fields>) => ScopeUpdate | undefined {
  const sentUser = new Map<string, string>();
  const sentTags = new Map<string, string>();
  const sentExtra = new Map<string, string>();
  const sentContexts = new Map<string, string>();

  return (scope) => {
    const update: ScopeUpdate = {};

    if (diff(sentUser, (scope.user || {}) as Record<string, unknown>)) {
      update.user = sentUser.size ? Object.fromEntries([...sentUser].map(([k, v]) => [k, JSON.parse(v)])) : null;
    }

    const tags = diff(sentTags, scope.tags || {});
    if (tags) update.tags = tags;
    const extra = diff(sentExtra, scope.extra || {});
    if (extra) update.extra = extra;
    const contexts = diff(sentContexts, scope.contexts || {});
    if (contexts) update.contexts = contexts;

    return Object.keys(update).length ? update : undefined;
  };
}

/**
 * Sends user, tags, extra and contexts to the Rust SDK when they change.
 * Breadcrumbs go through `beforeBreadcrumb` instead.
 */
export function scopeToRustIntegration(): Integration {
  return {
    name: 'ScopeToRust',
    setup() {
      const differ = createScopeDiffer();
      let failed = false;
      let queued = false;

      const send = () => {
        queued = false;
        const scope = differ(mergedScope());
        if (!scope || failed) {
          return;
        }
        invoke('plugin:sentry|scope', { scope }).catch((e) => {
          console.error('Failed to send scope to Rust:', e);
          failed = true;
        });
      };

      // Many scope changes in one task cause one IPC call
      const listener = () => {
        if (!queued) {
          queued = true;
          queueMicrotask(send);
        }
      };

      getGlobalScope().addScopeListener(listener);
      getIsolationScope().addScopeListener(listener);
      getCurrentScope().addScopeListener(listener);
      listener();
    },
  };
}
