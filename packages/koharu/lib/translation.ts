import type {
  LanguageChoice,
  Model,
  ModelSelection,
  Provider,
  ProviderPreference,
} from '@koharu/bridge/protocol'

export function providerName(entries: ProviderPreference[], provider: Provider): string {
  return entries.find((entry) => entry.config.provider === provider)?.name ?? provider
}

export function modelKey(model: Model | ModelSelection): string {
  return `${model.provider}:${model.model ?? ''}`
}

export function modelSelection(model: Model, quantization: string | null): ModelSelection {
  return {
    provider: model.provider,
    model: model.model,
    quantization,
    vision: model.vision,
    reasoning: model.reasoning,
  }
}

export function orderedLanguageChoices(
  languages: readonly LanguageChoice[],
): Array<{ tag: string; name: string }> {
  return languages
    .map((language) => ({ tag: language.tag, name: language.name }))
    .sort((left, right) =>
      left.name.localeCompare(right.name, undefined, { numeric: true, sensitivity: 'base' }),
    )
}
