// Vendored from getsentry/sentry-tauri js/normalize.ts at f83b7225301e327884c3ba62a51bba6d324c3871
// (v0.9.0).
import type { Event, IntegrationFn } from '@sentry/core';
import { defineIntegration } from '@sentry/core';

const APP_PREFIX = 'app:///';

function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * Replaces the webview origin in URLs in an event with `app:///`. Mutates the
 * passed in event.
 *
 * @param event The event to normalize.
 * @param origin The webview origin, for example `tauri://localhost`.
 */
export function normalizeUrls(event: Event, origin: string): Event {
  // URLs with a custom scheme can have an empty path, for example
  // `tauri://localhost`, so the slash after the origin is optional.
  const pattern = new RegExp(`${escapeRegExp(origin)}(/|(?=[?#\\s])|$)`, 'g');
  const normalize = (url: string) => url.replace(pattern, APP_PREFIX);

  for (const exception of event.exception?.values || []) {
    for (const frame of exception.stacktrace?.frames || []) {
      if (frame.filename) {
        frame.filename = normalize(frame.filename);
      }
      if (frame.abs_path) {
        frame.abs_path = normalize(frame.abs_path);
      }
    }
  }

  for (const image of event.debug_meta?.images || []) {
    if (image.type === 'sourcemap') {
      image.code_file = normalize(image.code_file);
    }
  }

  if (event.transaction) {
    event.transaction = normalize(event.transaction);
  }

  if (event.request?.url) {
    event.request.url = normalize(event.request.url);
  }

  for (const span of event.spans || []) {
    if (span.description) {
      span.description = normalize(span.description);
    }
    for (const key of ['url', 'url.full', 'http.url']) {
      const value = span.data?.[key];
      if (typeof value === 'string') {
        span.data[key] = normalize(value);
      }
    }
  }

  const urls = (event as { urls?: unknown }).urls;
  if (Array.isArray(urls)) {
    (event as { urls: unknown[] }).urls = urls.map((url) => (typeof url === 'string' ? normalize(url) : url));
  }

  return event;
}

const _normalizeUrlsIntegration = (() => {
  return {
    name: 'TauriNormalizeUrls',
    processEvent(event) {
      return normalizeUrls(event, window.location.origin);
    },
  };
}) satisfies IntegrationFn;

/**
 * Replaces the webview origin with `app:///` so URLs match on every platform.
 * Tauri serves the app from `tauri://localhost` on macOS, iOS and Linux and
 * from `http://tauri.localhost` on Windows and Android.
 */
export const normalizeUrlsIntegration = defineIntegration(_normalizeUrlsIntegration);
