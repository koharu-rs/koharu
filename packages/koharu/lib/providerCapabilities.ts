import type { Model, Provider, ProviderPreference } from '@koharu/bridge/protocol'

/**
 * The request shapes an entry point can actually send. A selector is bound to
 * exactly one of these, and only models the catalog proves compatible appear.
 */
export type EntryPoint =
  | { kind: 'text' }
  | { kind: 'multimodal'; modality: 'image' | 'audio' | 'video' }

/**
 * OrcaRouter is the only provider whose model list declares input modalities
 * from a live catalog and fails closed when a record omits them. Other providers
 * keep their existing picker behaviour, so filtering is scoped to it rather than
 * silently changing what the other providers offer.
 */
export function declaresModalities(provider: Provider): boolean {
  return provider === 'orcarouter'
}

export function isOrcaRouter(provider: Provider): boolean {
  return provider === 'orcarouter'
}

/**
 * Whether a model may be offered to an entry point.
 *
 * A model that does not declare the modality the entry point uploads is not
 * compatible: an undeclared capability is not a capability.
 */
export function supports(model: Model, entry: EntryPoint): boolean {
  if (entry.kind === 'text') return true
  if (entry.modality !== 'image') {
    // Koharu uploads page images only. Audio and video entry points do not exist
    // yet, so nothing may be offered for them.
    return false
  }
  return model.vision
}

/**
 * The options a selector must be bound to for the current entry point.
 *
 * Providers that do not declare modalities are returned untouched so their
 * existing interaction is preserved.
 */
export function optionsFor(models: Model[], entry: EntryPoint): Model[] {
  if (entry.kind === 'text') return models
  return models.filter((model) => !declaresModalities(model.provider) || supports(model, entry))
}

/**
 * Drop a selection that the current entry point can no longer justify, so the
 * picker never keeps a value that would fail at request time.
 *
 * Returns `null` when the caller must clear the selection and ask again.
 */
export function retainSelection(
  models: Model[],
  entry: EntryPoint,
  model: Model | null | undefined,
): Model | null | undefined {
  if (!model) return model
  if (!declaresModalities(model.provider)) return model
  return supports(model, entry) ? model : null
}

/**
 * True when the user must pick again because their model no longer fits.
 */
export function selectionInvalidated(
  models: Model[],
  entry: EntryPoint,
  model: Model | null | undefined,
): boolean {
  if (!model || !declaresModalities(model.provider)) return false
  if (!models.some((candidate) => candidate.provider === model.provider)) return false
  return !supports(model, entry)
}

export function providerLabel(providers: ProviderPreference[], provider: Provider): string {
  return providers.find((entry) => entry.config.provider === provider)?.name ?? provider
}
