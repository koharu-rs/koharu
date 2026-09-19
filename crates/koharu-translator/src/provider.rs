use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use specta::Type;
use strum::{Display, EnumIter, EnumString, IntoStaticStr, VariantArray};

use crate::{
    local::LocalConfig,
    remote::{
        CaiyunConfig, ClaudeConfig, DeepLConfig, DeepSeekConfig, GeminiConfig, GoogleCloudConfig,
        GrokConfig, LmStudioConfig, MiniMaxConfig, OpenAiCompatibleConfig, OpenAiConfig,
        OpenRouterConfig,
    },
};

macro_rules! define_providers {
    ($(
        $variant:ident {
            id: $id:literal,
            name: $name:literal,
            field: $field:ident,
            config: $config:ty,
            system_prompt: $system_prompt:literal,
        }
    )+) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Display,
            EnumIter,
            EnumString,
            Eq,
            Hash,
            IntoStaticStr,
            PartialEq,
            Serialize,
            Deserialize,
            Type,
            VariantArray,
        )]
        pub enum Provider {
            $(
                #[serde(rename = $id)]
                #[strum(serialize = $id)]
                $variant,
            )+
        }

        impl Provider {
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }

            #[must_use]
            pub const fn supports_system_prompt(self) -> bool {
                match self {
                    $(Self::$variant => $system_prompt,)+
                }
            }
        }

        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
        #[serde(tag = "provider", content = "settings")]
        pub enum ProviderConfig {
            $(
                #[serde(rename = $id)]
                $variant($config),
            )+
        }

        impl ProviderConfig {
            #[must_use]
            pub const fn provider(&self) -> Provider {
                match self {
                    $(Self::$variant(_) => Provider::$variant,)+
                }
            }
        }

        #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
        #[serde(default)]
        pub struct ProvidersConfig {
            $(
                #[serde(rename = $id)]
                pub $field: $config,
            )+
        }

        impl ProvidersConfig {
            #[must_use]
            pub fn entries(&self) -> Vec<ProviderConfig> {
                vec![$(ProviderConfig::$variant(self.$field.clone()),)+]
            }

            pub fn from_entries(entries: impl IntoIterator<Item = ProviderConfig>) -> Result<Self> {
                let mut config = Self::default();
                let mut seen = std::collections::HashSet::new();
                for entry in entries {
                    let provider = entry.provider();
                    if !seen.insert(provider) {
                        bail!("duplicate provider configuration for {provider}");
                    }
                    match entry {
                        $(ProviderConfig::$variant(settings) => config.$field = settings,)+
                    }
                }
                if seen.len() != Provider::VARIANTS.len() {
                    let missing = Provider::VARIANTS
                        .iter()
                        .filter(|provider| !seen.contains(provider))
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    bail!("missing provider configurations: {missing}");
                }
                Ok(config)
            }
        }
    };
}

define_providers! {
    Local {
        id: "local",
        name: "Local",
        field: local,
        config: LocalConfig,
        system_prompt: true,
    }
    OpenAi {
        id: "openai",
        name: "OpenAI",
        field: openai,
        config: OpenAiConfig,
        system_prompt: true,
    }
    Gemini {
        id: "gemini",
        name: "Gemini",
        field: gemini,
        config: GeminiConfig,
        system_prompt: true,
    }
    Claude {
        id: "claude",
        name: "Claude",
        field: claude,
        config: ClaudeConfig,
        system_prompt: true,
    }
    Grok {
        id: "grok",
        name: "Grok",
        field: grok,
        config: GrokConfig,
        system_prompt: true,
    }
    MiniMax {
        id: "minimax",
        name: "MiniMax",
        field: minimax,
        config: MiniMaxConfig,
        system_prompt: true,
    }
    DeepSeek {
        id: "deepseek",
        name: "DeepSeek",
        field: deepseek,
        config: DeepSeekConfig,
        system_prompt: true,
    }
    OpenAiCompatible {
        id: "openai-compatible",
        name: "OpenAI-compatible",
        field: openai_compatible,
        config: OpenAiCompatibleConfig,
        system_prompt: true,
    }
    OpenRouter {
        id: "openrouter",
        name: "OpenRouter",
        field: openrouter,
        config: OpenRouterConfig,
        system_prompt: true,
    }
    LmStudio {
        id: "lm-studio",
        name: "LM Studio",
        field: lm_studio,
        config: LmStudioConfig,
        system_prompt: true,
    }
    DeepL {
        id: "deepl",
        name: "DeepL",
        field: deepl,
        config: DeepLConfig,
        system_prompt: false,
    }
    GoogleCloudTranslation {
        id: "google-cloud-translation",
        name: "Google Cloud Translation",
        field: google_cloud_translation,
        config: GoogleCloudConfig,
        system_prompt: false,
    }
    Caiyun {
        id: "caiyun",
        name: "Caiyun",
        field: caiyun,
        config: CaiyunConfig,
        system_prompt: false,
    }
}

impl ProvidersConfig {
    pub fn load() -> anyhow::Result<koharu_config::Config<Self>> {
        koharu_config::load("providers")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_capability_matches_provider_architecture() {
        for provider in [
            Provider::Local,
            Provider::OpenAi,
            Provider::Gemini,
            Provider::Claude,
            Provider::Grok,
            Provider::MiniMax,
            Provider::DeepSeek,
            Provider::OpenAiCompatible,
            Provider::OpenRouter,
            Provider::LmStudio,
        ] {
            assert!(provider.supports_system_prompt(), "{provider}");
        }
        for provider in [
            Provider::DeepL,
            Provider::GoogleCloudTranslation,
            Provider::Caiyun,
        ] {
            assert!(!provider.supports_system_prompt(), "{provider}");
        }
    }
}
