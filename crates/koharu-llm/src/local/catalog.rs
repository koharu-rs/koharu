//! The GGUF models Koharu offers, each pinned to a Hugging Face revision.
//! Ported from `koharu-translator`'s catalog.

use koharu_runtime::Repository;

use super::Sampling;

pub(crate) const DEFAULT_MODEL: &str = "gemma4-12b-it";
pub(crate) const DEFAULT_QUANTIZATION: &str = "Q4_K_XL";

pub(crate) struct Entry {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) repository: Repository,
    /// The first is the default.
    pub(crate) quantizations: &'static [Quantization],
    pub(crate) projector: Option<&'static str>,
    /// Whether its chat template takes `enable_thinking`.
    pub(crate) reasoning: bool,
    pub(crate) sampling: Sampling,
    /// The languages it translates into, as BCP 47 tags; `None` for all.
    pub(crate) languages: Option<&'static [&'static str]>,
}

pub(crate) struct Quantization {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) file: &'static str,
}

impl Quantization {
    const fn new(id: &'static str, name: &'static str, file: &'static str) -> Self {
        Self { id, name, file }
    }
}

/// The settings every entry shares: min-p 0.05, a 1,000-token answer, and a
/// repeat penalty of 1.2.
const fn sampling(temperature: f64, top_k: u32, top_p: f64) -> Sampling {
    Sampling {
        temperature: Some(temperature),
        max_tokens: Some(1000),
        top_p: Some(top_p),
        seed: None,
        top_k: Some(top_k),
        min_p: Some(0.05),
        repeat_penalty: Some(1.2),
        repeat_last_n: None,
        frequency_penalty: None,
        presence_penalty: None,
        enable_thinking: None,
    }
}

impl Entry {
    pub(crate) fn quantization(&self, id: Option<&str>) -> anyhow::Result<&'static Quantization> {
        let Some(id) = id else {
            return Ok(&self.quantizations[0]);
        };
        self.quantizations
            .iter()
            .find(|quantization| quantization.id == id)
            .ok_or_else(|| anyhow::anyhow!("{} has no {id} quantization", self.id))
    }
}

pub(crate) fn entry(id: &str) -> anyhow::Result<&'static Entry> {
    MODELS
        .iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| anyhow::anyhow!("the catalog has no {id}"))
}

pub(crate) static MODELS: &[Entry] = &[
    Entry {
        id: "lfm2.5-1.2b-instruct",
        name: "LFM2.5 1.2B Instruct",
        repository: Repository::model(
            "LiquidAI/LFM2.5-1.2B-Instruct-GGUF",
            "afbd8eaeab5dd94ba0b079ebfb02517d19641e38",
        ),
        quantizations: &[
            Quantization::new("Q4_K_M", "Q4_K_M", "LFM2.5-1.2B-Instruct-Q4_K_M.gguf"),
            Quantization::new("BF16", "BF16", "LFM2.5-1.2B-Instruct-BF16.gguf"),
            Quantization::new("F16", "F16", "LFM2.5-1.2B-Instruct-F16.gguf"),
            Quantization::new("Q4_0", "Q4_0", "LFM2.5-1.2B-Instruct-Q4_0.gguf"),
            Quantization::new("Q5_K_M", "Q5_K_M", "LFM2.5-1.2B-Instruct-Q5_K_M.gguf"),
            Quantization::new("Q6_K", "Q6_K", "LFM2.5-1.2B-Instruct-Q6_K.gguf"),
            Quantization::new("Q8_0", "Q8_0", "LFM2.5-1.2B-Instruct-Q8_0.gguf"),
        ],
        projector: None,
        reasoning: false,
        sampling: sampling(0.1, 50, 0.9),
        languages: Some(&[
            "en-US", "ar-SA", "zh-CN", "fr-FR", "de-DE", "ja-JP", "ko-KR", "pt-PT", "es-ES",
        ]),
    },
    Entry {
        id: "ministral-3-8b-instruct",
        name: "Ministral 3 8B Instruct",
        repository: Repository::model(
            "mistralai/Ministral-3-8B-Instruct-2512-GGUF",
            "0102285ad796bd99af90f58de616092e5630e970",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_M",
                "Q4_K M",
                "Ministral-3-8B-Instruct-2512-Q4_K_M.gguf",
            ),
            Quantization::new("BF16", "BF16", "Ministral-3-8B-Instruct-2512-BF16.gguf"),
            Quantization::new(
                "Q5_K_M",
                "Q5_K M",
                "Ministral-3-8B-Instruct-2512-Q5_K_M.gguf",
            ),
            Quantization::new("Q8_0", "Q8_0", "Ministral-3-8B-Instruct-2512-Q8_0.gguf"),
        ],
        projector: None,
        reasoning: false,
        sampling: sampling(0.05, 40, 0.9),
        languages: Some(&[
            "en-US", "ar-SA", "zh-CN", "fr-FR", "de-DE", "it-IT", "ja-JP", "ko-KR", "pt-PT",
            "es-ES", "nl-NL",
        ]),
    },
    Entry {
        id: "gemma4-e2b-it",
        name: "Gemma 4 E2B Instruct",
        repository: Repository::model(
            "unsloth/gemma-4-E2B-it-qat-GGUF",
            "66a399f68ddd113b06dff02fca9523e55465d11d",
        ),
        quantizations: &[
            Quantization::new("Q4_K_XL", "Q4_K XL", "gemma-4-E2B-it-qat-UD-Q4_K_XL.gguf"),
            Quantization::new("Q2_K_XL", "Q2_K XL", "gemma-4-E2B-it-qat-UD-Q2_K_XL.gguf"),
        ],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 64, 0.9),
        languages: None,
    },
    Entry {
        id: "gemma4-e4b-it",
        name: "Gemma 4 E4B Instruct",
        repository: Repository::model(
            "unsloth/gemma-4-E4B-it-qat-GGUF",
            "8c5a9e4fd5482e2be20fe0bf013b4c262a8f4265",
        ),
        quantizations: &[
            Quantization::new("Q4_K_XL", "Q4_K XL", "gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf"),
            Quantization::new("Q2_K_XL", "Q2_K XL", "gemma-4-E4B-it-qat-UD-Q2_K_XL.gguf"),
        ],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 64, 0.9),
        languages: None,
    },
    Entry {
        id: "gemma4-12b-it",
        name: "Gemma 4 12B Instruct",
        repository: Repository::model(
            "unsloth/gemma-4-12B-it-qat-GGUF",
            "980b060c40a8539ac159e0501a3e0f66a6365af3",
        ),
        quantizations: &[Quantization::new(
            "Q4_K_XL",
            "Q4_K XL",
            "gemma-4-12B-it-qat-UD-Q4_K_XL.gguf",
        )],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 64, 0.9),
        languages: None,
    },
    Entry {
        id: "gemma4-26b-a4b-it",
        name: "Gemma 4 26B A4B Instruct",
        repository: Repository::model(
            "unsloth/gemma-4-26B-A4B-it-qat-GGUF",
            "7b92b5b28818151e8669af2e45e88d6086f490dd",
        ),
        quantizations: &[Quantization::new(
            "Q4_K_XL",
            "Q4_K XL",
            "gemma-4-26B-A4B-it-qat-UD-Q4_K_XL.gguf",
        )],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 64, 0.9),
        languages: None,
    },
    Entry {
        id: "gemma4-31b-it",
        name: "Gemma 4 31B Instruct",
        repository: Repository::model(
            "unsloth/gemma-4-31B-it-qat-GGUF",
            "43cc1aeb31adf47ec06a854507ce552cd9862e6f",
        ),
        quantizations: &[Quantization::new(
            "Q4_K_XL",
            "Q4_K XL",
            "gemma-4-31B-it-qat-UD-Q4_K_XL.gguf",
        )],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 64, 0.9),
        languages: None,
    },
    Entry {
        id: "gemma4-e2b-uncensored",
        name: "Gemma 4 E2B Uncensored",
        repository: Repository::model(
            "HauhauCS/Gemma-4-E2B-Uncensored-HauhauCS-Aggressive",
            "da8593c3e407afcd3e7da94ff2d69d77e2a28a48",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_P",
                "Q4_K P",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-Q4_K_P.gguf",
            ),
            Quantization::new(
                "IQ3_M",
                "IQ3 M",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf",
            ),
            Quantization::new(
                "Q2_K_P",
                "Q2_K P",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-Q2_K_P.gguf",
            ),
            Quantization::new(
                "Q3_K_P",
                "Q3_K P",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-Q3_K_P.gguf",
            ),
            Quantization::new(
                "Q5_K_P",
                "Q5_K P",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-Q5_K_P.gguf",
            ),
            Quantization::new(
                "Q6_K_P",
                "Q6_K P",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-Q6_K_P.gguf",
            ),
            Quantization::new(
                "Q8_K_P",
                "Q8_K P",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-Q8_K_P.gguf",
            ),
        ],
        projector: Some("mmproj-Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-f16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 64, 0.9),
        languages: None,
    },
    Entry {
        id: "gemma4-e4b-uncensored",
        name: "Gemma 4 E4B Uncensored",
        repository: Repository::model(
            "HauhauCS/Gemma-4-E4B-Uncensored-HauhauCS-Aggressive",
            "45b6a334b4bcd1d7f37179df58b3b1d66a184e5d",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_P",
                "Q4_K P",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-Q4_K_P.gguf",
            ),
            Quantization::new(
                "IQ3_M",
                "IQ3 M",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf",
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4 XS",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-IQ4_XS.gguf",
            ),
            Quantization::new(
                "Q2_K_P",
                "Q2_K P",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-Q2_K_P.gguf",
            ),
            Quantization::new(
                "Q3_K_P",
                "Q3_K P",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-Q3_K_P.gguf",
            ),
            Quantization::new(
                "Q5_K_P",
                "Q5_K P",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-Q5_K_P.gguf",
            ),
            Quantization::new(
                "Q6_K_P",
                "Q6_K P",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-Q6_K_P.gguf",
            ),
            Quantization::new(
                "Q8_K_P",
                "Q8_K P",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-Q8_K_P.gguf",
            ),
        ],
        projector: Some("mmproj-Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-f16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 64, 0.9),
        languages: None,
    },
    Entry {
        id: "gemma4-12b-uncensored",
        name: "Gemma 4 12B Uncensored",
        repository: Repository::model(
            "HauhauCS/Gemma4-12B-QAT-Uncensored-HauhauCS-Balanced",
            "ae8045ac2bd216293ca49a3065da2c942dde4b68",
        ),
        quantizations: &[Quantization::new(
            "Q4_K_M",
            "Q4_K M",
            "Gemma4-12B-QAT-Uncensored-HauhauCS-Balanced-Q4_K_M.gguf",
        )],
        projector: Some("mmproj-Gemma4-12B-QAT-Uncensored-HauhauCS-Balanced-BF16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 64, 0.9),
        languages: None,
    },
    Entry {
        id: "gemma4-26b-a4b-uncensored",
        name: "Gemma 4 26B A4B Uncensored",
        repository: Repository::model(
            "HauhauCS/Gemma4-26B-A4B-QAT-Uncensored-HauhauCS-Balanced-MTP",
            "f9093662a2e7ae0503f637088bc96f77a1a70c83",
        ),
        quantizations: &[Quantization::new(
            "Q4_K_M",
            "Q4_K M",
            "Gemma4-26B-A4B-QAT-Uncensored-HauhauCS-Balanced-Q4_K_M.gguf",
        )],
        projector: Some("mmproj-Gemma4-26B-A4B-QAT-Uncensored-HauhauCS-Balanced-BF16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 64, 0.9),
        languages: None,
    },
    Entry {
        id: "gemma4-31b-uncensored",
        name: "Gemma 4 31B Uncensored",
        repository: Repository::model(
            "HauhauCS/Gemma4-31B-QAT-Uncensored-HauhauCS-Balanced-MTP",
            "9654466e82d83f5ebfe1518a369bc5900873abb1",
        ),
        quantizations: &[Quantization::new(
            "Q4_K_M",
            "Q4_K M",
            "Gemma4-31B-QAT-Uncensored-HauhauCS-Balanced-Q4_K_M.gguf",
        )],
        projector: Some("mmproj-Gemma4-31B-QAT-Uncensored-HauhauCS-Balanced-BF16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 64, 0.9),
        languages: None,
    },
    Entry {
        id: "qwen3.5-0.8b",
        name: "Qwen 3.5 0.8B",
        repository: Repository::model(
            "unsloth/Qwen3.5-0.8B-GGUF",
            "6ab461498e2023f6e3c1baea90a8f0fe38ab64d0",
        ),
        quantizations: &[
            Quantization::new("Q4_K_XL", "Q4_K XL", "Qwen3.5-0.8B-UD-Q4_K_XL.gguf"),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.5-0.8B-UD-IQ2_M.gguf"),
            Quantization::new("IQ2_XXS", "IQ2 XXS", "Qwen3.5-0.8B-UD-IQ2_XXS.gguf"),
            Quantization::new("IQ3_XXS", "IQ3 XXS", "Qwen3.5-0.8B-UD-IQ3_XXS.gguf"),
            Quantization::new("Q2_K_XL", "Q2_K XL", "Qwen3.5-0.8B-UD-Q2_K_XL.gguf"),
            Quantization::new("Q3_K_XL", "Q3_K XL", "Qwen3.5-0.8B-UD-Q3_K_XL.gguf"),
            Quantization::new("Q5_K_XL", "Q5_K XL", "Qwen3.5-0.8B-UD-Q5_K_XL.gguf"),
            Quantization::new("Q6_K_XL", "Q6_K XL", "Qwen3.5-0.8B-UD-Q6_K_XL.gguf"),
            Quantization::new("Q8_K_XL", "Q8_K XL", "Qwen3.5-0.8B-UD-Q8_K_XL.gguf"),
        ],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.9),
        languages: None,
    },
    Entry {
        id: "qwen3.5-2b",
        name: "Qwen 3.5 2B",
        repository: Repository::model(
            "unsloth/Qwen3.5-2B-GGUF",
            "f6d5376be1edb4d416d56da11e5397a961aca8ae",
        ),
        quantizations: &[
            Quantization::new("Q4_K_XL", "Q4_K XL", "Qwen3.5-2B-UD-Q4_K_XL.gguf"),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.5-2B-UD-IQ2_M.gguf"),
            Quantization::new("IQ2_XXS", "IQ2 XXS", "Qwen3.5-2B-UD-IQ2_XXS.gguf"),
            Quantization::new("IQ3_XXS", "IQ3 XXS", "Qwen3.5-2B-UD-IQ3_XXS.gguf"),
            Quantization::new("Q2_K_XL", "Q2_K XL", "Qwen3.5-2B-UD-Q2_K_XL.gguf"),
            Quantization::new("Q3_K_XL", "Q3_K XL", "Qwen3.5-2B-UD-Q3_K_XL.gguf"),
            Quantization::new("Q5_K_XL", "Q5_K XL", "Qwen3.5-2B-UD-Q5_K_XL.gguf"),
            Quantization::new("Q6_K_XL", "Q6_K XL", "Qwen3.5-2B-UD-Q6_K_XL.gguf"),
            Quantization::new("Q8_K_XL", "Q8_K XL", "Qwen3.5-2B-UD-Q8_K_XL.gguf"),
        ],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.9),
        languages: None,
    },
    Entry {
        id: "qwen3.5-4b",
        name: "Qwen 3.5 4B",
        repository: Repository::model(
            "unsloth/Qwen3.5-4B-GGUF",
            "e87f176479d0855a907a41277aca2f8ee7a09523",
        ),
        quantizations: &[
            Quantization::new("Q4_K_XL", "Q4_K XL", "Qwen3.5-4B-UD-Q4_K_XL.gguf"),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.5-4B-UD-IQ2_M.gguf"),
            Quantization::new("IQ2_XXS", "IQ2 XXS", "Qwen3.5-4B-UD-IQ2_XXS.gguf"),
            Quantization::new("IQ3_XXS", "IQ3 XXS", "Qwen3.5-4B-UD-IQ3_XXS.gguf"),
            Quantization::new("Q2_K_XL", "Q2_K XL", "Qwen3.5-4B-UD-Q2_K_XL.gguf"),
            Quantization::new("Q3_K_XL", "Q3_K XL", "Qwen3.5-4B-UD-Q3_K_XL.gguf"),
            Quantization::new("Q5_K_XL", "Q5_K XL", "Qwen3.5-4B-UD-Q5_K_XL.gguf"),
            Quantization::new("Q6_K_XL", "Q6_K XL", "Qwen3.5-4B-UD-Q6_K_XL.gguf"),
            Quantization::new("Q8_K_XL", "Q8_K XL", "Qwen3.5-4B-UD-Q8_K_XL.gguf"),
        ],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.9),
        languages: None,
    },
    Entry {
        id: "qwen3.5-9b",
        name: "Qwen 3.5 9B",
        repository: Repository::model(
            "unsloth/Qwen3.5-9B-GGUF",
            "3885219b6810b007914f3a7950a8d1b469d598a5",
        ),
        quantizations: &[
            Quantization::new("Q4_K_XL", "Q4_K XL", "Qwen3.5-9B-UD-Q4_K_XL.gguf"),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.5-9B-UD-IQ2_M.gguf"),
            Quantization::new("IQ2_XXS", "IQ2 XXS", "Qwen3.5-9B-UD-IQ2_XXS.gguf"),
            Quantization::new("IQ3_XXS", "IQ3 XXS", "Qwen3.5-9B-UD-IQ3_XXS.gguf"),
            Quantization::new("Q2_K_XL", "Q2_K XL", "Qwen3.5-9B-UD-Q2_K_XL.gguf"),
            Quantization::new("Q3_K_XL", "Q3_K XL", "Qwen3.5-9B-UD-Q3_K_XL.gguf"),
            Quantization::new("Q5_K_XL", "Q5_K XL", "Qwen3.5-9B-UD-Q5_K_XL.gguf"),
            Quantization::new("Q6_K_XL", "Q6_K XL", "Qwen3.5-9B-UD-Q6_K_XL.gguf"),
            Quantization::new("Q8_K_XL", "Q8_K XL", "Qwen3.5-9B-UD-Q8_K_XL.gguf"),
        ],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.9),
        languages: None,
    },
    Entry {
        id: "qwen3.5-27b",
        name: "Qwen 3.5 27B",
        repository: Repository::model(
            "unsloth/Qwen3.5-27B-GGUF",
            "3221f178a6b842d04f1fb42f1c413534adcc0a6a",
        ),
        quantizations: &[
            Quantization::new("Q4_K_XL", "Q4_K XL", "Qwen3.5-27B-UD-Q4_K_XL.gguf"),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.5-27B-UD-IQ2_M.gguf"),
            Quantization::new("IQ2_XXS", "IQ2 XXS", "Qwen3.5-27B-UD-IQ2_XXS.gguf"),
            Quantization::new("IQ3_XXS", "IQ3 XXS", "Qwen3.5-27B-UD-IQ3_XXS.gguf"),
            Quantization::new("Q2_K_XL", "Q2_K XL", "Qwen3.5-27B-UD-Q2_K_XL.gguf"),
            Quantization::new("Q3_K_XL", "Q3_K XL", "Qwen3.5-27B-UD-Q3_K_XL.gguf"),
            Quantization::new("Q5_K_XL", "Q5_K XL", "Qwen3.5-27B-UD-Q5_K_XL.gguf"),
            Quantization::new("Q6_K_XL", "Q6_K XL", "Qwen3.5-27B-UD-Q6_K_XL.gguf"),
            Quantization::new("Q8_K_XL", "Q8_K XL", "Qwen3.5-27B-UD-Q8_K_XL.gguf"),
        ],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.9),
        languages: None,
    },
    Entry {
        id: "qwen3.5-35b-a3b",
        name: "Qwen 3.5 35B A3B",
        repository: Repository::model(
            "unsloth/Qwen3.5-35B-A3B-GGUF",
            "bc014a17be43adabd7066b7a86075ff935c6a4e2",
        ),
        quantizations: &[
            Quantization::new("Q4_K_XL", "Q4_K XL", "Qwen3.5-35B-A3B-UD-Q4_K_XL.gguf"),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.5-35B-A3B-UD-IQ2_M.gguf"),
            Quantization::new("IQ2_XXS", "IQ2 XXS", "Qwen3.5-35B-A3B-UD-IQ2_XXS.gguf"),
            Quantization::new("IQ3_XXS", "IQ3 XXS", "Qwen3.5-35B-A3B-UD-IQ3_XXS.gguf"),
            Quantization::new("Q2_K_XL", "Q2_K XL", "Qwen3.5-35B-A3B-UD-Q2_K_XL.gguf"),
            Quantization::new("Q3_K_XL", "Q3_K XL", "Qwen3.5-35B-A3B-UD-Q3_K_XL.gguf"),
            Quantization::new("Q5_K_XL", "Q5_K XL", "Qwen3.5-35B-A3B-UD-Q5_K_XL.gguf"),
            Quantization::new("Q6_K_XL", "Q6_K XL", "Qwen3.5-35B-A3B-UD-Q6_K_XL.gguf"),
            Quantization::new("Q8_K_XL", "Q8_K XL", "Qwen3.5-35B-A3B-UD-Q8_K_XL.gguf"),
        ],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.9),
        languages: None,
    },
    Entry {
        id: "qwen3.6-27b",
        name: "Qwen 3.6 27B",
        repository: Repository::model(
            "unsloth/Qwen3.6-27B-GGUF",
            "82d411acf4a06cfb8d9b073a5211bf410bfc29bf",
        ),
        quantizations: &[
            Quantization::new("Q4_K_XL", "Q4_K XL", "Qwen3.6-27B-UD-Q4_K_XL.gguf"),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.6-27B-UD-IQ2_M.gguf"),
            Quantization::new("IQ2_XXS", "IQ2 XXS", "Qwen3.6-27B-UD-IQ2_XXS.gguf"),
            Quantization::new("IQ3_XXS", "IQ3 XXS", "Qwen3.6-27B-UD-IQ3_XXS.gguf"),
            Quantization::new("Q2_K_XL", "Q2_K XL", "Qwen3.6-27B-UD-Q2_K_XL.gguf"),
            Quantization::new("Q3_K_XL", "Q3_K XL", "Qwen3.6-27B-UD-Q3_K_XL.gguf"),
            Quantization::new("Q5_K_XL", "Q5_K XL", "Qwen3.6-27B-UD-Q5_K_XL.gguf"),
            Quantization::new("Q6_K_XL", "Q6_K XL", "Qwen3.6-27B-UD-Q6_K_XL.gguf"),
            Quantization::new("Q8_K_XL", "Q8_K XL", "Qwen3.6-27B-UD-Q8_K_XL.gguf"),
        ],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.8),
        languages: None,
    },
    Entry {
        id: "qwen3.6-35b-a3b",
        name: "Qwen 3.6 35B A3B",
        repository: Repository::model(
            "unsloth/Qwen3.6-35B-A3B-GGUF",
            "a483e9e6cbd595906af30beda3187c2663a1118c",
        ),
        quantizations: &[
            Quantization::new("Q4_K_XL", "Q4_K XL", "Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf"),
            Quantization::new("IQ1_M", "IQ1 M", "Qwen3.6-35B-A3B-UD-IQ1_M.gguf"),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.6-35B-A3B-UD-IQ2_M.gguf"),
            Quantization::new("IQ2_XXS", "IQ2 XXS", "Qwen3.6-35B-A3B-UD-IQ2_XXS.gguf"),
            Quantization::new("IQ3_S", "IQ3 S", "Qwen3.6-35B-A3B-UD-IQ3_S.gguf"),
            Quantization::new("IQ3_XXS", "IQ3 XXS", "Qwen3.6-35B-A3B-UD-IQ3_XXS.gguf"),
            Quantization::new("IQ4_NL", "IQ4 NL", "Qwen3.6-35B-A3B-UD-IQ4_NL.gguf"),
            Quantization::new("IQ4_XS", "IQ4 XS", "Qwen3.6-35B-A3B-UD-IQ4_XS.gguf"),
            Quantization::new("Q2_K_XL", "Q2_K XL", "Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf"),
            Quantization::new("Q3_K_M", "Q3_K M", "Qwen3.6-35B-A3B-UD-Q3_K_M.gguf"),
            Quantization::new("Q3_K_S", "Q3_K S", "Qwen3.6-35B-A3B-UD-Q3_K_S.gguf"),
            Quantization::new("Q3_K_XL", "Q3_K XL", "Qwen3.6-35B-A3B-UD-Q3_K_XL.gguf"),
            Quantization::new("Q4_K_M", "Q4_K M", "Qwen3.6-35B-A3B-UD-Q4_K_M.gguf"),
            Quantization::new("Q4_K_S", "Q4_K S", "Qwen3.6-35B-A3B-UD-Q4_K_S.gguf"),
            Quantization::new("Q5_K_M", "Q5_K M", "Qwen3.6-35B-A3B-UD-Q5_K_M.gguf"),
            Quantization::new("Q5_K_S", "Q5_K S", "Qwen3.6-35B-A3B-UD-Q5_K_S.gguf"),
            Quantization::new("Q5_K_XL", "Q5_K XL", "Qwen3.6-35B-A3B-UD-Q5_K_XL.gguf"),
            Quantization::new("Q6_K", "Q6_K", "Qwen3.6-35B-A3B-UD-Q6_K.gguf"),
            Quantization::new("Q6_K_XL", "Q6_K XL", "Qwen3.6-35B-A3B-UD-Q6_K_XL.gguf"),
            Quantization::new("Q8_K_XL", "Q8_K XL", "Qwen3.6-35B-A3B-UD-Q8_K_XL.gguf"),
        ],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.8),
        languages: None,
    },
    Entry {
        id: "qwen3.8-27b",
        name: "Qwen 3.8 27B",
        repository: Repository::model(
            "unsloth/Qwen3.8-27B-GGUF",
            "f975863083b62f54a5e6fac11671c750c2bbc59c",
        ),
        quantizations: &[
            Quantization::new("Q4_K_XL", "Q4_K XL", "Qwen3.8-27B-UD-Q4_K_XL.gguf"),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.8-27B-UD-IQ2_M.gguf"),
            Quantization::new("IQ2_XXS", "IQ2 XXS", "Qwen3.8-27B-UD-IQ2_XXS.gguf"),
            Quantization::new("IQ3_XXS", "IQ3 XXS", "Qwen3.8-27B-UD-IQ3_XXS.gguf"),
            Quantization::new("Q2_K_XL", "Q2_K XL", "Qwen3.8-27B-UD-Q2_K_XL.gguf"),
            Quantization::new("Q3_K_XL", "Q3_K XL", "Qwen3.8-27B-UD-Q3_K_XL.gguf"),
            Quantization::new("Q5_K_XL", "Q5_K XL", "Qwen3.8-27B-UD-Q5_K_XL.gguf"),
            Quantization::new("Q6_K_XL", "Q6_K XL", "Qwen3.8-27B-UD-Q6_K_XL.gguf"),
            Quantization::new("Q8_K_XL", "Q8_K XL", "Qwen3.8-27B-UD-Q8_K_XL.gguf"),
        ],
        projector: Some("mmproj-F16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.8),
        languages: None,
    },
    Entry {
        id: "qwen3.5-2b-uncensored",
        name: "Qwen 3.5 2B Uncensored",
        repository: Repository::model(
            "HauhauCS/Qwen3.5-2B-Uncensored-HauhauCS-Aggressive",
            "2bcf35c1ebf62c837c12c1aa90b578ff4717e831",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_M",
                "Q4_K M",
                "Qwen3.5-2B-Uncensored-HauhauCS-Aggressive-Q4_K_M.gguf",
            ),
            Quantization::new(
                "BF16",
                "BF16",
                "Qwen3.5-2B-Uncensored-HauhauCS-Aggressive-BF16.gguf",
            ),
            Quantization::new(
                "Q6_K",
                "Q6_K",
                "Qwen3.5-2B-Uncensored-HauhauCS-Aggressive-Q6_K.gguf",
            ),
            Quantization::new(
                "Q8_0",
                "Q8_0",
                "Qwen3.5-2B-Uncensored-HauhauCS-Aggressive-Q8_0.gguf",
            ),
        ],
        projector: Some("mmproj-Qwen3.5-2B-Uncensored-HauhauCS-Aggressive-f16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.9),
        languages: None,
    },
    Entry {
        id: "qwen3.5-4b-uncensored",
        name: "Qwen 3.5 4B Uncensored",
        repository: Repository::model(
            "HauhauCS/Qwen3.5-4B-Uncensored-HauhauCS-Aggressive",
            "c09cdbcdb1fefad6d335809d445621b5f5ba0c6e",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_M",
                "Q4_K M",
                "Qwen3.5-4B-Uncensored-HauhauCS-Aggressive-Q4_K_M.gguf",
            ),
            Quantization::new(
                "BF16",
                "BF16",
                "Qwen3.5-4B-Uncensored-HauhauCS-Aggressive-BF16.gguf",
            ),
            Quantization::new(
                "Q6_K",
                "Q6_K",
                "Qwen3.5-4B-Uncensored-HauhauCS-Aggressive-Q6_K.gguf",
            ),
            Quantization::new(
                "Q8_0",
                "Q8_0",
                "Qwen3.5-4B-Uncensored-HauhauCS-Aggressive-Q8_0.gguf",
            ),
        ],
        projector: Some("mmproj-Qwen3.5-4B-Uncensored-HauhauCS-Aggressive-BF16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.9),
        languages: None,
    },
    Entry {
        id: "qwen3.5-9b-uncensored",
        name: "Qwen 3.5 9B Uncensored",
        repository: Repository::model(
            "HauhauCS/Qwen3.5-9B-Uncensored-HauhauCS-Aggressive",
            "0a41c68809d375475f954be12ba7c40efa56c2a9",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_M",
                "Q4_K M",
                "Qwen3.5-9B-Uncensored-HauhauCS-Aggressive-Q4_K_M.gguf",
            ),
            Quantization::new(
                "BF16",
                "BF16",
                "Qwen3.5-9B-Uncensored-HauhauCS-Aggressive-BF16.gguf",
            ),
            Quantization::new(
                "Q6_K",
                "Q6_K",
                "Qwen3.5-9B-Uncensored-HauhauCS-Aggressive-Q6_K.gguf",
            ),
            Quantization::new(
                "Q8_0",
                "Q8_0",
                "Qwen3.5-9B-Uncensored-HauhauCS-Aggressive-Q8_0.gguf",
            ),
        ],
        projector: Some("mmproj-Qwen3.5-9B-Uncensored-HauhauCS-Aggressive-BF16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.9),
        languages: None,
    },
    Entry {
        id: "qwen3.6-27b-uncensored",
        name: "Qwen 3.6 27B Uncensored",
        repository: Repository::model(
            "HauhauCS/Qwen3.6-27B-Uncensored-HauhauCS-Balanced",
            "9a28c2fbd15f21fb8ed204074c6f58d5bc889941",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_P",
                "Q4_K P",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-Q4_K_P.gguf",
            ),
            Quantization::new(
                "IQ2_M",
                "IQ2 M",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-IQ2_M.gguf",
            ),
            Quantization::new(
                "IQ3_M",
                "IQ3 M",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-IQ3_M.gguf",
            ),
            Quantization::new(
                "IQ3_XS",
                "IQ3 XS",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-IQ3_XS.gguf",
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4 XS",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-IQ4_XS.gguf",
            ),
            Quantization::new(
                "Q2_K_P",
                "Q2_K P",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-Q2_K_P.gguf",
            ),
            Quantization::new(
                "Q3_K_P",
                "Q3_K P",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-Q3_K_P.gguf",
            ),
            Quantization::new(
                "Q5_K_P",
                "Q5_K P",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-Q5_K_P.gguf",
            ),
            Quantization::new(
                "Q6_K_P",
                "Q6_K P",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-Q6_K_P.gguf",
            ),
            Quantization::new(
                "Q8_K_P",
                "Q8_K P",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-Q8_K_P.gguf",
            ),
        ],
        projector: Some("mmproj-Qwen3.6-27B-Uncensored-HauhauCS-Balanced-f16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.8),
        languages: None,
    },
    Entry {
        id: "qwen3.6-35b-a3b-uncensored",
        name: "Qwen 3.6 35B A3B Uncensored",
        repository: Repository::model(
            "HauhauCS/Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive",
            "f12a584fecbeb5f20001130d8ecd66c9327ae685",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_P",
                "Q4_K P",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q4_K_P.gguf",
            ),
            Quantization::new(
                "IQ2_M",
                "IQ2 M",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ2_M.gguf",
            ),
            Quantization::new(
                "IQ3_M",
                "IQ3 M",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf",
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4 XS",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ4_XS.gguf",
            ),
            Quantization::new(
                "Q2_K_P",
                "Q2_K P",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q2_K_P.gguf",
            ),
            Quantization::new(
                "Q3_K_P",
                "Q3_K P",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q3_K_P.gguf",
            ),
            Quantization::new(
                "Q5_K_P",
                "Q5_K P",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q5_K_P.gguf",
            ),
            Quantization::new(
                "Q6_K_P",
                "Q6_K P",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q6_K_P.gguf",
            ),
            Quantization::new(
                "Q8_K_P",
                "Q8_K P",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q8_K_P.gguf",
            ),
        ],
        projector: Some("mmproj-Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-f16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.8),
        languages: None,
    },
    Entry {
        id: "qwen3.8-27b-uncensored",
        name: "Qwen 3.8 27B Uncensored",
        repository: Repository::model(
            "HauhauCS/Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-MTP-GGUF",
            "993a5971fda8f30dd1b7eb2654792ba4415c7460",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_P",
                "Q4_K P",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q4_K_P.gguf",
            ),
            Quantization::new(
                "IQ2_M",
                "IQ2 M",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-IQ2_M.gguf",
            ),
            Quantization::new(
                "IQ3_M",
                "IQ3 M",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf",
            ),
            Quantization::new(
                "IQ3_XS",
                "IQ3 XS",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-IQ3_XS.gguf",
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4 XS",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-IQ4_XS.gguf",
            ),
            Quantization::new(
                "Q2_K_P",
                "Q2_K P",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q2_K_P.gguf",
            ),
            Quantization::new(
                "Q3_K_P",
                "Q3_K P",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q3_K_P.gguf",
            ),
            Quantization::new(
                "Q5_K_P",
                "Q5_K P",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q5_K_P.gguf",
            ),
            Quantization::new(
                "Q6_K_P",
                "Q6_K P",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q6_K_P.gguf",
            ),
            Quantization::new(
                "Q8_K_P",
                "Q8_K P",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q8_K_P.gguf",
            ),
        ],
        projector: Some("mmproj-Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-BF16.gguf"),
        reasoning: true,
        sampling: sampling(0.2, 20, 0.8),
        languages: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_unique_and_complete() {
        for (index, entry) in MODELS.iter().enumerate() {
            assert!(!MODELS[..index].iter().any(|other| other.id == entry.id));
            assert!(!entry.quantizations.is_empty());
            for (index, quantization) in entry.quantizations.iter().enumerate() {
                assert!(!quantization.file.is_empty());
                assert!(
                    !entry.quantizations[..index]
                        .iter()
                        .any(|other| other.id == quantization.id)
                );
            }
        }
        assert!(
            entry(DEFAULT_MODEL)
                .unwrap()
                .quantization(Some(DEFAULT_QUANTIZATION))
                .is_ok()
        );
    }
}
