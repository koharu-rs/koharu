//! The GGUF models Koharu offers, each pinned to a Hugging Face revision.
//! Ported from `koharu-translator`'s catalog.

use koharu_runtime::Repository;
use rig_core::catalog::ModelSpec;

use super::{
    Sampling,
    markup::{self, Markup},
};
use crate::TRANSLATION;

pub(crate) struct Entry {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) repository: Repository,
    /// The first is the default.
    pub(crate) quantizations: &'static [Quantization],
    pub(crate) projector: Option<Projector>,
    /// The tool-call and reasoning markup its chat template writes; a model
    /// without any reads its whole reply as its answer.
    pub(crate) markup: Option<&'static Markup>,
    /// A specialized kind, which its spec's `details.kind` holds, such as
    /// translation; `None` for a general model.
    pub(crate) kind: Option<&'static str>,
    pub(crate) sampling: Sampling,
    /// The languages it translates into, as BCP 47 tags; `None` for all.
    pub(crate) languages: Option<&'static [&'static str]>,
}

#[derive(Clone, Copy)]
pub(crate) struct Projector {
    pub(crate) file: &'static str,
    pub(crate) bytes: u64,
}

impl Projector {
    const fn new(file: &'static str, bytes: u64) -> Self {
        Self { file, bytes }
    }
}

pub(crate) struct Quantization {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) file: &'static str,
    pub(crate) bytes: u64,
}

impl Quantization {
    const fn new(id: &'static str, name: &'static str, file: &'static str, bytes: u64) -> Self {
        Self {
            id,
            name,
            file,
            bytes,
        }
    }
}

/// The settings every entry shares: min-p 0.05 and a repeat penalty of 1.2.
const fn sampling(temperature: f64, top_k: u32, top_p: f64) -> Sampling {
    Sampling {
        temperature: Some(temperature),
        max_tokens: None,
        top_p: Some(top_p),
        seed: None,
        top_k: Some(top_k),
        min_p: Some(0.05),
        repeat_penalty: Some(1.2),
        repeat_last_n: None,
        frequency_penalty: None,
        presence_penalty: None,
        enable_thinking: None,
        reasoning_effort: None,
    }
}

/// Greedy decoding with no penalties, as the translation models' cards
/// decode.
const GREEDY: Sampling = Sampling {
    temperature: Some(0.0),
    max_tokens: None,
    top_p: None,
    seed: None,
    top_k: None,
    min_p: None,
    repeat_penalty: None,
    repeat_last_n: None,
    frequency_penalty: None,
    presence_penalty: None,
    enable_thinking: None,
    reasoning_effort: None,
};

impl Entry {
    /// The model as rig describes it, but for the context it gets, which
    /// loading it says.
    pub(crate) fn spec(&self) -> ModelSpec {
        let mut spec = super::spec(self.id, self.markup, self.kind).with_display_name(self.name);
        spec.input.image = self.projector.is_some();
        spec
    }

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
        id: "lfm3-1.7b",
        name: "LFM3 1.7B",
        repository: Repository::model(
            "LiquidAI/LFM3-1.7B-GGUF",
            "cd0c74c575b5c416f3011b5b4c298077f26ca8c2",
        ),
        quantizations: &[
            Quantization::new("Q4_K_M", "Q4_K_M", "LFM3-1.7B-Q4_K_M.gguf", 1_099_627_808),
            Quantization::new("BF16", "BF16", "LFM3-1.7B-BF16.gguf", 3_466_626_336),
            Quantization::new("F16", "F16", "LFM3-1.7B-F16.gguf", 3_466_626_336),
            Quantization::new("NVFP4", "NVFP4", "LFM3-1.7B-NVFP4.gguf", 1_280_019_776),
            Quantization::new(
                "QAD-Q4_0",
                "QAD Q4_0",
                "LFM3-1.7B-QAD-Q4_0.gguf",
                1_049_615_680,
            ),
            Quantization::new("Q5_K_M", "Q5_K_M", "LFM3-1.7B-Q5_K_M.gguf", 1_258_663_200),
            Quantization::new("Q6_K", "Q6_K", "LFM3-1.7B-Q6_K.gguf", 1_427_638_304),
            Quantization::new("Q8_0", "Q8_0", "LFM3-1.7B-Q8_0.gguf", 1_846_238_496),
        ],
        projector: None,
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
        // The model card's settings, which name no top-p.
        sampling: Sampling {
            top_p: None,
            repeat_penalty: Some(1.1),
            ..sampling(0.1, 50, 0.9)
        },
        languages: Some(&[
            "en-US", "ar-SA", "zh-CN", "fr-FR", "de-DE", "it-IT", "ja-JP", "ko-KR", "pt-PT",
            "es-ES", "vi-VN", "th-TH", "id-ID", "hi-IN", "ru-RU", "pl-PL",
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
            Quantization::new(
                "Q4_K_XL",
                "Q4_K XL",
                "gemma-4-E2B-it-qat-UD-Q4_K_XL.gguf",
                2_620_370_976,
            ),
            Quantization::new(
                "Q2_K_XL",
                "Q2_K XL",
                "gemma-4-E2B-it-qat-UD-Q2_K_XL.gguf",
                2_186_186_784,
            ),
        ],
        projector: Some(Projector::new("mmproj-F16.gguf", 985_654_080)),
        markup: Some(&markup::GEMMA4),
        kind: None,
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
            Quantization::new(
                "Q4_K_XL",
                "Q4_K XL",
                "gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf",
                4_215_695_776,
            ),
            Quantization::new(
                "Q2_K_XL",
                "Q2_K XL",
                "gemma-4-E4B-it-qat-UD-Q2_K_XL.gguf",
                3_219_532_192,
            ),
        ],
        projector: Some(Projector::new("mmproj-F16.gguf", 990_372_672)),
        markup: Some(&markup::GEMMA4),
        kind: None,
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
            6_716_356_800,
        )],
        projector: Some(Projector::new("mmproj-F16.gguf", 175_115_840)),
        markup: Some(&markup::GEMMA4),
        kind: None,
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
            14_249_047_104,
        )],
        projector: Some(Projector::new("mmproj-F16.gguf", 1_193_058_784)),
        markup: Some(&markup::GEMMA4),
        kind: None,
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
            17_287_670_048,
        )],
        projector: Some(Projector::new("mmproj-F16.gguf", 1_198_957_024)),
        markup: Some(&markup::GEMMA4),
        kind: None,
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
                3_450_277_824,
            ),
            Quantization::new(
                "IQ3_M",
                "IQ3 M",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf",
                3_134_964_672,
            ),
            Quantization::new(
                "Q2_K_P",
                "Q2_K P",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-Q2_K_P.gguf",
                3_009_448_896,
            ),
            Quantization::new(
                "Q3_K_P",
                "Q3_K P",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-Q3_K_P.gguf",
                3_224_043_456,
            ),
            Quantization::new(
                "Q5_K_P",
                "Q5_K P",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-Q5_K_P.gguf",
                3_663_440_832,
            ),
            Quantization::new(
                "Q6_K_P",
                "Q6_K P",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-Q6_K_P.gguf",
                3_866_957_760,
            ),
            Quantization::new(
                "Q8_K_P",
                "Q8_K P",
                "Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-Q8_K_P.gguf",
                5_036_057_536,
            ),
        ],
        projector: Some(Projector::new(
            "mmproj-Gemma-4-E2B-Uncensored-HauhauCS-Aggressive-f16.gguf",
            985_570_240,
        )),
        markup: Some(&markup::GEMMA4_NO_TOOLS),
        kind: None,
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
                5_369_246_688,
            ),
            Quantization::new(
                "IQ3_M",
                "IQ3 M",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf",
                4_714_690_528,
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4 XS",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-IQ4_XS.gguf",
                5_070_950_368,
            ),
            Quantization::new(
                "Q2_K_P",
                "Q2_K P",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-Q2_K_P.gguf",
                4_431_882_208,
            ),
            Quantization::new(
                "Q3_K_P",
                "Q3_K P",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-Q3_K_P.gguf",
                4_884_505_568,
            ),
            Quantization::new(
                "Q5_K_P",
                "Q5_K P",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-Q5_K_P.gguf",
                5_812_940_768,
            ),
            Quantization::new(
                "Q6_K_P",
                "Q6_K P",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-Q6_K_P.gguf",
                6_249_794_528,
            ),
            Quantization::new(
                "Q8_K_P",
                "Q8_K P",
                "Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-Q8_K_P.gguf",
                8_133_226_464,
            ),
        ],
        projector: Some(Projector::new(
            "mmproj-Gemma-4-E4B-Uncensored-HauhauCS-Aggressive-f16.gguf",
            990_288_832,
        )),
        markup: Some(&markup::GEMMA4_NO_TOOLS),
        kind: None,
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
            7_381_381_760,
        )],
        projector: Some(Projector::new(
            "mmproj-Gemma4-12B-QAT-Uncensored-HauhauCS-Balanced-BF16.gguf",
            175_115_264,
        )),
        markup: Some(&markup::GEMMA4),
        kind: None,
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
            16_796_015_520,
        )],
        projector: Some(Projector::new(
            "mmproj-Gemma4-26B-A4B-QAT-Uncensored-HauhauCS-Balanced-BF16.gguf",
            1_194_827_776,
        )),
        markup: Some(&markup::GEMMA4),
        kind: None,
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
            18_687_062_176,
        )],
        projector: Some(Projector::new(
            "mmproj-Gemma4-31B-QAT-Uncensored-HauhauCS-Balanced-BF16.gguf",
            1_200_726_016,
        )),
        markup: Some(&markup::GEMMA4),
        kind: None,
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
            Quantization::new(
                "Q4_K_XL",
                "Q4_K XL",
                "Qwen3.5-0.8B-UD-Q4_K_XL.gguf",
                558_772_480,
            ),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.5-0.8B-UD-IQ2_M.gguf", 371_933_440),
            Quantization::new(
                "IQ2_XXS",
                "IQ2 XXS",
                "Qwen3.5-0.8B-UD-IQ2_XXS.gguf",
                338_227_456,
            ),
            Quantization::new(
                "IQ3_XXS",
                "IQ3 XXS",
                "Qwen3.5-0.8B-UD-IQ3_XXS.gguf",
                398_237_952,
            ),
            Quantization::new(
                "Q2_K_XL",
                "Q2_K XL",
                "Qwen3.5-0.8B-UD-Q2_K_XL.gguf",
                417_718_528,
            ),
            Quantization::new(
                "Q3_K_XL",
                "Q3_K XL",
                "Qwen3.5-0.8B-UD-Q3_K_XL.gguf",
                492_216_576,
            ),
            Quantization::new(
                "Q5_K_XL",
                "Q5_K XL",
                "Qwen3.5-0.8B-UD-Q5_K_XL.gguf",
                606_585_088,
            ),
            Quantization::new(
                "Q6_K_XL",
                "Q6_K XL",
                "Qwen3.5-0.8B-UD-Q6_K_XL.gguf",
                771_092_736,
            ),
            Quantization::new(
                "Q8_K_XL",
                "Q8_K XL",
                "Qwen3.5-0.8B-UD-Q8_K_XL.gguf",
                1_186_443_520,
            ),
        ],
        projector: Some(Projector::new("mmproj-F16.gguf", 204_987_232)),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
            Quantization::new(
                "Q4_K_XL",
                "Q4_K XL",
                "Qwen3.5-2B-UD-Q4_K_XL.gguf",
                1_339_752_704,
            ),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.5-2B-UD-IQ2_M.gguf", 859_857_152),
            Quantization::new(
                "IQ2_XXS",
                "IQ2 XXS",
                "Qwen3.5-2B-UD-IQ2_XXS.gguf",
                768_270_592,
            ),
            Quantization::new(
                "IQ3_XXS",
                "IQ3 XXS",
                "Qwen3.5-2B-UD-IQ3_XXS.gguf",
                931_823_872,
            ),
            Quantization::new(
                "Q2_K_XL",
                "Q2_K XL",
                "Qwen3.5-2B-UD-Q2_K_XL.gguf",
                966_533_376,
            ),
            Quantization::new(
                "Q3_K_XL",
                "Q3_K XL",
                "Qwen3.5-2B-UD-Q3_K_XL.gguf",
                1_159_274_752,
            ),
            Quantization::new(
                "Q5_K_XL",
                "Q5_K XL",
                "Qwen3.5-2B-UD-Q5_K_XL.gguf",
                1_466_687_744,
            ),
            Quantization::new(
                "Q6_K_XL",
                "Q6_K XL",
                "Qwen3.5-2B-UD-Q6_K_XL.gguf",
                1_864_483_072,
            ),
            Quantization::new(
                "Q8_K_XL",
                "Q8_K XL",
                "Qwen3.5-2B-UD-Q8_K_XL.gguf",
                2_834_940_160,
            ),
        ],
        projector: Some(Projector::new("mmproj-F16.gguf", 668_227_264)),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
            Quantization::new(
                "Q4_K_XL",
                "Q4_K XL",
                "Qwen3.5-4B-UD-Q4_K_XL.gguf",
                2_912_109_728,
            ),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.5-4B-UD-IQ2_M.gguf", 1_759_997_088),
            Quantization::new(
                "IQ2_XXS",
                "IQ2 XXS",
                "Qwen3.5-4B-UD-IQ2_XXS.gguf",
                1_520_217_248,
            ),
            Quantization::new(
                "IQ3_XXS",
                "IQ3 XXS",
                "Qwen3.5-4B-UD-IQ3_XXS.gguf",
                1_949_047_968,
            ),
            Quantization::new(
                "Q2_K_XL",
                "Q2_K XL",
                "Qwen3.5-4B-UD-Q2_K_XL.gguf",
                1_940_825_248,
            ),
            Quantization::new(
                "Q3_K_XL",
                "Q3_K XL",
                "Qwen3.5-4B-UD-Q3_K_XL.gguf",
                2_436_420_768,
            ),
            Quantization::new(
                "Q5_K_XL",
                "Q5_K XL",
                "Qwen3.5-4B-UD-Q5_K_XL.gguf",
                3_250_869_408,
            ),
            Quantization::new(
                "Q6_K_XL",
                "Q6_K XL",
                "Qwen3.5-4B-UD-Q6_K_XL.gguf",
                4_145_548_448,
            ),
            Quantization::new(
                "Q8_K_XL",
                "Q8_K XL",
                "Qwen3.5-4B-UD-Q8_K_XL.gguf",
                5_952_048_288,
            ),
        ],
        projector: Some(Projector::new("mmproj-F16.gguf", 672_423_616)),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
            Quantization::new(
                "Q4_K_XL",
                "Q4_K XL",
                "Qwen3.5-9B-UD-Q4_K_XL.gguf",
                5_966_095_584,
            ),
            Quantization::new("IQ2_M", "IQ2 M", "Qwen3.5-9B-UD-IQ2_M.gguf", 3_649_365_216),
            Quantization::new(
                "IQ2_XXS",
                "IQ2 XXS",
                "Qwen3.5-9B-UD-IQ2_XXS.gguf",
                3_190_613_216,
            ),
            Quantization::new(
                "IQ3_XXS",
                "IQ3 XXS",
                "Qwen3.5-9B-UD-IQ3_XXS.gguf",
                4_016_235_744,
            ),
            Quantization::new(
                "Q2_K_XL",
                "Q2_K XL",
                "Qwen3.5-9B-UD-Q2_K_XL.gguf",
                4_121_781_472,
            ),
            Quantization::new(
                "Q3_K_XL",
                "Q3_K XL",
                "Qwen3.5-9B-UD-Q3_K_XL.gguf",
                5_053_834_464,
            ),
            Quantization::new(
                "Q5_K_XL",
                "Q5_K XL",
                "Qwen3.5-9B-UD-Q5_K_XL.gguf",
                6_743_680_224,
            ),
            Quantization::new(
                "Q6_K_XL",
                "Q6_K XL",
                "Qwen3.5-9B-UD-Q6_K_XL.gguf",
                8_756_929_760,
            ),
            Quantization::new(
                "Q8_K_XL",
                "Q8_K XL",
                "Qwen3.5-9B-UD-Q8_K_XL.gguf",
                12_974_040_288,
            ),
        ],
        projector: Some(Projector::new("mmproj-F16.gguf", 918_166_080)),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
            Quantization::new(
                "Q4_K_XL",
                "Q4_K XL",
                "Qwen3.5-27B-UD-Q4_K_XL.gguf",
                17_621_125_024,
            ),
            Quantization::new(
                "IQ2_M",
                "IQ2 M",
                "Qwen3.5-27B-UD-IQ2_M.gguf",
                10_188_072_864,
            ),
            Quantization::new(
                "IQ2_XXS",
                "IQ2 XXS",
                "Qwen3.5-27B-UD-IQ2_XXS.gguf",
                8_573_593_504,
            ),
            Quantization::new(
                "IQ3_XXS",
                "IQ3 XXS",
                "Qwen3.5-27B-UD-IQ3_XXS.gguf",
                11_506_493_344,
            ),
            Quantization::new(
                "Q2_K_XL",
                "Q2_K XL",
                "Qwen3.5-27B-UD-Q2_K_XL.gguf",
                11_213_752_224,
            ),
            Quantization::new(
                "Q3_K_XL",
                "Q3_K XL",
                "Qwen3.5-27B-UD-Q3_K_XL.gguf",
                14_438_533_024,
            ),
            Quantization::new(
                "Q5_K_XL",
                "Q5_K XL",
                "Qwen3.5-27B-UD-Q5_K_XL.gguf",
                20_171_253_664,
            ),
            Quantization::new(
                "Q6_K_XL",
                "Q6_K XL",
                "Qwen3.5-27B-UD-Q6_K_XL.gguf",
                25_675_642_784,
            ),
            Quantization::new(
                "Q8_K_XL",
                "Q8_K XL",
                "Qwen3.5-27B-UD-Q8_K_XL.gguf",
                35_528_652_704,
            ),
        ],
        projector: Some(Projector::new("mmproj-F16.gguf", 927_607_040)),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
            Quantization::new(
                "Q4_K_XL",
                "Q4_K XL",
                "Qwen3.5-35B-A3B-UD-Q4_K_XL.gguf",
                22_241_950_336,
            ),
            Quantization::new(
                "IQ2_M",
                "IQ2 M",
                "Qwen3.5-35B-A3B-UD-IQ2_M.gguf",
                11_391_613_568,
            ),
            Quantization::new(
                "IQ2_XXS",
                "IQ2 XXS",
                "Qwen3.5-35B-A3B-UD-IQ2_XXS.gguf",
                10_656_955_008,
            ),
            Quantization::new(
                "IQ3_XXS",
                "IQ3 XXS",
                "Qwen3.5-35B-A3B-UD-IQ3_XXS.gguf",
                13_080_066_688,
            ),
            Quantization::new(
                "Q2_K_XL",
                "Q2_K XL",
                "Qwen3.5-35B-A3B-UD-Q2_K_XL.gguf",
                12_156_754_560,
            ),
            Quantization::new(
                "Q3_K_XL",
                "Q3_K XL",
                "Qwen3.5-35B-A3B-UD-Q3_K_XL.gguf",
                16_601_176_704,
            ),
            Quantization::new(
                "Q5_K_XL",
                "Q5_K XL",
                "Qwen3.5-35B-A3B-UD-Q5_K_XL.gguf",
                26_385_922_688,
            ),
            Quantization::new(
                "Q6_K_XL",
                "Q6_K XL",
                "Qwen3.5-35B-A3B-UD-Q6_K_XL.gguf",
                32_071_842_432,
            ),
            Quantization::new(
                "Q8_K_XL",
                "Q8_K XL",
                "Qwen3.5-35B-A3B-UD-Q8_K_XL.gguf",
                48_688_560_768,
            ),
        ],
        projector: Some(Projector::new("mmproj-F16.gguf", 899_283_648)),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
            Quantization::new(
                "Q4_K_XL",
                "Q4_K XL",
                "Qwen3.6-27B-UD-Q4_K_XL.gguf",
                17_612_564_704,
            ),
            Quantization::new(
                "IQ2_M",
                "IQ2 M",
                "Qwen3.6-27B-UD-IQ2_M.gguf",
                10_846_136_544,
            ),
            Quantization::new(
                "IQ2_XXS",
                "IQ2 XXS",
                "Qwen3.6-27B-UD-IQ2_XXS.gguf",
                9_388_779_744,
            ),
            Quantization::new(
                "IQ3_XXS",
                "IQ3 XXS",
                "Qwen3.6-27B-UD-IQ3_XXS.gguf",
                11_994_777_824,
            ),
            Quantization::new(
                "Q2_K_XL",
                "Q2_K XL",
                "Qwen3.6-27B-UD-Q2_K_XL.gguf",
                11_849_779_424,
            ),
            Quantization::new(
                "Q3_K_XL",
                "Q3_K XL",
                "Qwen3.6-27B-UD-Q3_K_XL.gguf",
                14_473_431_264,
            ),
            Quantization::new(
                "Q5_K_XL",
                "Q5_K XL",
                "Qwen3.6-27B-UD-Q5_K_XL.gguf",
                20_038_256_864,
            ),
            Quantization::new(
                "Q6_K_XL",
                "Q6_K XL",
                "Qwen3.6-27B-UD-Q6_K_XL.gguf",
                25_636_485_344,
            ),
            Quantization::new(
                "Q8_K_XL",
                "Q8_K XL",
                "Qwen3.6-27B-UD-Q8_K_XL.gguf",
                35_325_163_744,
            ),
        ],
        projector: Some(Projector::new("mmproj-F16.gguf", 927_607_360)),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
            Quantization::new(
                "Q4_K_XL",
                "Q4_K XL",
                "Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf",
                22_360_456_160,
            ),
            Quantization::new(
                "IQ1_M",
                "IQ1 M",
                "Qwen3.6-35B-A3B-UD-IQ1_M.gguf",
                10_047_749_088,
            ),
            Quantization::new(
                "IQ2_M",
                "IQ2 M",
                "Qwen3.6-35B-A3B-UD-IQ2_M.gguf",
                11_522_702_304,
            ),
            Quantization::new(
                "IQ2_XXS",
                "IQ2 XXS",
                "Qwen3.6-35B-A3B-UD-IQ2_XXS.gguf",
                10_756_586_464,
            ),
            Quantization::new(
                "IQ3_S",
                "IQ3 S",
                "Qwen3.6-35B-A3B-UD-IQ3_S.gguf",
                13_676_723_168,
            ),
            Quantization::new(
                "IQ3_XXS",
                "IQ3 XXS",
                "Qwen3.6-35B-A3B-UD-IQ3_XXS.gguf",
                13_211_155_424,
            ),
            Quantization::new(
                "IQ4_NL",
                "IQ4 NL",
                "Qwen3.6-35B-A3B-UD-IQ4_NL.gguf",
                18_040_888_288,
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4 XS",
                "Qwen3.6-35B-A3B-UD-IQ4_XS.gguf",
                17_730_509_792,
            ),
            Quantization::new(
                "Q2_K_XL",
                "Q2_K XL",
                "Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf",
                12_290_628_576,
            ),
            Quantization::new(
                "Q3_K_M",
                "Q3_K M",
                "Qwen3.6-35B-A3B-UD-Q3_K_M.gguf",
                16_600_710_112,
            ),
            Quantization::new(
                "Q3_K_S",
                "Q3_K S",
                "Qwen3.6-35B-A3B-UD-Q3_K_S.gguf",
                15_359_196_128,
            ),
            Quantization::new(
                "Q3_K_XL",
                "Q3_K XL",
                "Qwen3.6-35B-A3B-UD-Q3_K_XL.gguf",
                16_845_511_648,
            ),
            Quantization::new(
                "Q4_K_M",
                "Q4_K M",
                "Qwen3.6-35B-A3B-UD-Q4_K_M.gguf",
                22_134_528_992,
            ),
            Quantization::new(
                "Q4_K_S",
                "Q4_K S",
                "Qwen3.6-35B-A3B-UD-Q4_K_S.gguf",
                20_893_015_008,
            ),
            Quantization::new(
                "Q5_K_M",
                "Q5_K M",
                "Qwen3.6-35B-A3B-UD-Q5_K_M.gguf",
                26_456_194_016,
            ),
            Quantization::new(
                "Q5_K_S",
                "Q5_K S",
                "Qwen3.6-35B-A3B-UD-Q5_K_S.gguf",
                24_942_050_272,
            ),
            Quantization::new(
                "Q5_K_XL",
                "Q5_K XL",
                "Qwen3.6-35B-A3B-UD-Q5_K_XL.gguf",
                26_592_508_896,
            ),
            Quantization::new(
                "Q6_K",
                "Q6_K",
                "Qwen3.6-35B-A3B-UD-Q6_K.gguf",
                29_308_320_736,
            ),
            Quantization::new(
                "Q6_K_XL",
                "Q6_K XL",
                "Qwen3.6-35B-A3B-UD-Q6_K_XL.gguf",
                31_843_777_504,
            ),
            Quantization::new(
                "Q8_K_XL",
                "Q8_K XL",
                "Qwen3.6-35B-A3B-UD-Q8_K_XL.gguf",
                38_451_182_560,
            ),
        ],
        projector: Some(Projector::new("mmproj-F16.gguf", 899_283_680)),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
        sampling: sampling(0.2, 20, 0.8),
        languages: None,
    },
    Entry {
        id: "qwen3.8-27b",
        name: "Qwen 3.8 27B",
        repository: Repository::model(
            "unsloth/Qwen3.8-27B-GGUF",
            "4af51b3542f775b0a8e4b310bfb3f17e01c306c9",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_XL",
                "Q4_K XL",
                "Qwen3.8-27B-UD-Q4_K_XL.gguf",
                17_659_407_264,
            ),
            Quantization::new(
                "IQ2_XXS",
                "IQ2 XXS",
                "Qwen3.8-27B-UD-IQ2_XXS.gguf",
                7_206_043_648,
            ),
            Quantization::new(
                "IQ3_XXS",
                "IQ3 XXS",
                "Qwen3.8-27B-UD-IQ3_XXS.gguf",
                10_934_860_704,
            ),
            Quantization::new(
                "Q2_K_XL",
                "Q2_K XL",
                "Qwen3.8-27B-UD-Q2_K_XL.gguf",
                9_948_666_784,
            ),
            Quantization::new(
                "Q3_K_XL",
                "Q3_K XL",
                "Qwen3.8-27B-UD-Q3_K_XL.gguf",
                13_146_393_504,
            ),
            Quantization::new(
                "Q5_K_XL",
                "Q5_K XL",
                "Qwen3.8-27B-UD-Q5_K_XL.gguf",
                20_976_962_464,
            ),
            Quantization::new(
                "Q6_K_XL",
                "Q6_K XL",
                "Qwen3.8-27B-UD-Q6_K_XL.gguf",
                25_299_061_664,
            ),
            Quantization::new(
                "Q8_K_XL",
                "Q8_K XL",
                "Qwen3.8-27B-UD-Q8_K_XL.gguf",
                31_457_991_680,
            ),
        ],
        projector: Some(Projector::new("mmproj-F16.gguf", 927_607_488)),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
                1_270_808_032,
            ),
            Quantization::new(
                "BF16",
                "BF16",
                "Qwen3.5-2B-Uncensored-HauhauCS-Aggressive-BF16.gguf",
                3_775_708_640,
            ),
            Quantization::new(
                "Q6_K",
                "Q6_K",
                "Qwen3.5-2B-Uncensored-HauhauCS-Aggressive-Q6_K.gguf",
                1_556_390_368,
            ),
            Quantization::new(
                "Q8_0",
                "Q8_0",
                "Qwen3.5-2B-Uncensored-HauhauCS-Aggressive-Q8_0.gguf",
                2_012_012_000,
            ),
        ],
        projector: Some(Projector::new(
            "mmproj-Qwen3.5-2B-Uncensored-HauhauCS-Aggressive-f16.gguf",
            668_226_688,
        )),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
                2_707_513_696,
            ),
            Quantization::new(
                "BF16",
                "BF16",
                "Qwen3.5-4B-Uncensored-HauhauCS-Aggressive-BF16.gguf",
                8_424_393_056,
            ),
            Quantization::new(
                "Q6_K",
                "Q6_K",
                "Qwen3.5-4B-Uncensored-HauhauCS-Aggressive-Q6_K.gguf",
                3_464_055_136,
            ),
            Quantization::new(
                "Q8_0",
                "Q8_0",
                "Qwen3.5-4B-Uncensored-HauhauCS-Aggressive-Q8_0.gguf",
                4_482_402_656,
            ),
        ],
        projector: Some(Projector::new(
            "mmproj-Qwen3.5-4B-Uncensored-HauhauCS-Aggressive-BF16.gguf",
            675_568_768,
        )),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
                5_627_044_224,
            ),
            Quantization::new(
                "BF16",
                "BF16",
                "Qwen3.5-9B-Uncensored-HauhauCS-Aggressive-BF16.gguf",
                17_920_696_704,
            ),
            Quantization::new(
                "Q6_K",
                "Q6_K",
                "Qwen3.5-9B-Uncensored-HauhauCS-Aggressive-Q6_K.gguf",
                7_359_259_008,
            ),
            Quantization::new(
                "Q8_0",
                "Q8_0",
                "Qwen3.5-9B-Uncensored-HauhauCS-Aggressive-Q8_0.gguf",
                9_527_501_184,
            ),
        ],
        projector: Some(Projector::new(
            "mmproj-Qwen3.5-9B-Uncensored-HauhauCS-Aggressive-BF16.gguf",
            921_704_448,
        )),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
                17_536_279_712,
            ),
            Quantization::new(
                "IQ2_M",
                "IQ2 M",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-IQ2_M.gguf",
                10_004_592_032,
            ),
            Quantization::new(
                "IQ3_M",
                "IQ3 M",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-IQ3_M.gguf",
                12_580_873_632,
            ),
            Quantization::new(
                "IQ3_XS",
                "IQ3 XS",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-IQ3_XS.gguf",
                11_967_128_992,
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4 XS",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-IQ4_XS.gguf",
                15_082_505_632,
            ),
            Quantization::new(
                "Q2_K_P",
                "Q2_K P",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-Q2_K_P.gguf",
                11_516_069_792,
            ),
            Quantization::new(
                "Q3_K_P",
                "Q3_K P",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-Q3_K_P.gguf",
                14_301_709_472,
            ),
            Quantization::new(
                "Q5_K_P",
                "Q5_K P",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-Q5_K_P.gguf",
                20_812_952_992,
            ),
            Quantization::new(
                "Q6_K_P",
                "Q6_K P",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-Q6_K_P.gguf",
                23_154_960_032,
            ),
            Quantization::new(
                "Q8_K_P",
                "Q8_K P",
                "Qwen3.6-27B-Uncensored-HauhauCS-Balanced-Q8_K_P.gguf",
                31_963_749_792,
            ),
        ],
        projector: Some(Projector::new(
            "mmproj-Qwen3.6-27B-Uncensored-HauhauCS-Balanced-f16.gguf",
            927_606_944,
        )),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
                23_424_536_704,
            ),
            Quantization::new(
                "IQ2_M",
                "IQ2 M",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ2_M.gguf",
                11_659_235_456,
            ),
            Quantization::new(
                "IQ3_M",
                "IQ3 M",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf",
                15_440_519_296,
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4 XS",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ4_XS.gguf",
                18_728_777_856,
            ),
            Quantization::new(
                "Q2_K_P",
                "Q2_K P",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q2_K_P.gguf",
                14_981_265_536,
            ),
            Quantization::new(
                "Q3_K_P",
                "Q3_K P",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q3_K_P.gguf",
                19_023_337_600,
            ),
            Quantization::new(
                "Q5_K_P",
                "Q5_K P",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q5_K_P.gguf",
                28_027_394_176,
            ),
            Quantization::new(
                "Q6_K_P",
                "Q6_K P",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q6_K_P.gguf",
                30_649_317_504,
            ),
            Quantization::new(
                "Q8_K_P",
                "Q8_K P",
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q8_K_P.gguf",
                43_605_014_656,
            ),
        ],
        projector: Some(Projector::new(
            "mmproj-Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-f16.gguf",
            899_283_072,
        )),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
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
                17_923_393_664,
            ),
            Quantization::new(
                "IQ2_M",
                "IQ2 M",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-IQ2_M.gguf",
                10_319_906_944,
            ),
            Quantization::new(
                "IQ3_M",
                "IQ3 M",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf",
                12_789_303_424,
            ),
            Quantization::new(
                "IQ3_XS",
                "IQ3 XS",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-IQ3_XS.gguf",
                12_175_558_784,
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4 XS",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-IQ4_XS.gguf",
                15_705_860_224,
            ),
            Quantization::new(
                "Q2_K_P",
                "Q2_K P",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q2_K_P.gguf",
                10_676_422_784,
            ),
            Quantization::new(
                "Q3_K_P",
                "Q3_K P",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q3_K_P.gguf",
                13_441_058_944,
            ),
            Quantization::new(
                "Q5_K_P",
                "Q5_K P",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q5_K_P.gguf",
                20_218_177_664,
            ),
            Quantization::new(
                "Q6_K_P",
                "Q6_K P",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q6_K_P.gguf",
                25_924_151_424,
            ),
            Quantization::new(
                "Q8_K_P",
                "Q8_K P",
                "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q8_K_P.gguf",
                31_457_990_784,
            ),
        ],
        projector: Some(Projector::new(
            "mmproj-Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-BF16.gguf",
            931_146_624,
        )),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: None,
        sampling: sampling(0.2, 20, 0.8),
        languages: None,
    },
    Entry {
        id: "index-translate-2b",
        name: "Index-Translate 2B",
        repository: Repository::model(
            "IndexTeam/Index-Translate-2B-GGUF",
            "3a0eca6ce931d88d00614d427ab3ed8ff0743902",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_M",
                "Q4_K_M",
                "Index-Translate-2B.Q4_K_M.gguf",
                1_312_164_352,
            ),
            Quantization::new("Q2_K", "Q2_K", "Index-Translate-2B.Q2_K.gguf", 990_491_136),
            Quantization::new(
                "Q3_K_S",
                "Q3_K_S",
                "Index-Translate-2B.Q3_K_S.gguf",
                1_046_349_312,
            ),
            Quantization::new(
                "Q3_K_M",
                "Q3_K_M",
                "Index-Translate-2B.Q3_K_M.gguf",
                1_127_802_368,
            ),
            Quantization::new(
                "Q3_K_L",
                "Q3_K_L",
                "Index-Translate-2B.Q3_K_L.gguf",
                1_195_304_448,
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4_XS",
                "Index-Translate-2B.IQ4_XS.gguf",
                1_234_377_216,
            ),
            Quantization::new(
                "Q4_K_S",
                "Q4_K_S",
                "Index-Translate-2B.Q4_K_S.gguf",
                1_246_308_864,
            ),
            Quantization::new(
                "Q5_K_S",
                "Q5_K_S",
                "Index-Translate-2B.Q5_K_S.gguf",
                1_415_932_416,
            ),
            Quantization::new(
                "Q5_K_M",
                "Q5_K_M",
                "Index-Translate-2B.Q5_K_M.gguf",
                1_454_787_072,
            ),
            Quantization::new(
                "Q6_K",
                "Q6_K",
                "Index-Translate-2B.Q6_K.gguf",
                1_606_323_712,
            ),
            Quantization::new(
                "Q8_0",
                "Q8_0",
                "Index-Translate-2B.Q8_0.gguf",
                2_076_674_560,
            ),
            Quantization::new("F16", "F16", "Index-Translate-2B.f16.gguf", 3_897_387_520),
        ],
        // The Q8_0 projector, which the card recommends for local use.
        projector: Some(Projector::new(
            "Index-Translate-2B.mmproj-Q8_0.gguf",
            361_518_144,
        )),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: Some(TRANSLATION),
        sampling: GREEDY,
        languages: None,
    },
    Entry {
        id: "index-translate-9b",
        name: "Index-Translate 9B",
        repository: Repository::model(
            "IndexTeam/Index-Translate-9B-GGUF",
            "645939fbab839a656f1bc39975a872bc8ad749f8",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_M",
                "Q4_K_M",
                "Index-Translate-9B.Q4_K_M.gguf",
                5_780_090_304,
            ),
            Quantization::new(
                "Q2_K",
                "Q2_K",
                "Index-Translate-9B.Q2_K.gguf",
                3_914_968_512,
            ),
            Quantization::new(
                "Q3_K_S",
                "Q3_K_S",
                "Index-Translate-9B.Q3_K_S.gguf",
                4_364_021_184,
            ),
            Quantization::new(
                "Q3_K_M",
                "Q3_K_M",
                "Index-Translate-9B.Q3_K_M.gguf",
                4_737_609_152,
            ),
            Quantization::new(
                "Q3_K_L",
                "Q3_K_L",
                "Index-Translate-9B.Q3_K_L.gguf",
                5_048_511_936,
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4_XS",
                "Index-Translate-9B.IQ4_XS.gguf",
                5_357_874_624,
            ),
            Quantization::new(
                "Q4_K_S",
                "Q4_K_S",
                "Index-Translate-9B.Q4_K_S.gguf",
                5_488_553_408,
            ),
            Quantization::new(
                "Q5_K_S",
                "Q5_K_S",
                "Index-Translate-9B.Q5_K_S.gguf",
                6_472_641_984,
            ),
            Quantization::new(
                "Q5_K_M",
                "Q5_K_M",
                "Index-Translate-9B.Q5_K_M.gguf",
                6_642_544_064,
            ),
            Quantization::new(
                "Q6_K",
                "Q6_K",
                "Index-Translate-9B.Q6_K.gguf",
                7_558_901_184,
            ),
            Quantization::new(
                "Q8_0",
                "Q8_0",
                "Index-Translate-9B.Q8_0.gguf",
                9_786_060_224,
            ),
            Quantization::new("F16", "F16", "Index-Translate-9B.f16.gguf", 18_407_321_024),
        ],
        // The Q8_0 projector, which the card recommends for local use.
        projector: Some(Projector::new(
            "Index-Translate-9B.mmproj-Q8_0.gguf",
            620_690_272,
        )),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: Some(TRANSLATION),
        sampling: GREEDY,
        languages: None,
    },
    Entry {
        id: "index-translate-35b-a3b-preview",
        name: "Index-Translate 35B A3B Preview",
        repository: Repository::model(
            "IndexTeam/Index-Translate-35B-A3B-preview-GGUF",
            "3319ce338363a181c6c20c06b7379063d9baf484",
        ),
        quantizations: &[
            Quantization::new(
                "Q4_K_M",
                "Q4_K_M",
                "Index-Translate-35B-A3B-preview.Q4_K_M.gguf",
                21_713_462_400,
            ),
            Quantization::new(
                "Q2_K",
                "Q2_K",
                "Index-Translate-35B-A3B-preview.Q2_K.gguf",
                13_247_181_952,
            ),
            Quantization::new(
                "Q3_K_S",
                "Q3_K_S",
                "Index-Translate-35B-A3B-preview.Q3_K_S.gguf",
                15_547_034_752,
            ),
            Quantization::new(
                "Q3_K_M",
                "Q3_K_M",
                "Index-Translate-35B-A3B-preview.Q3_K_M.gguf",
                17_166_658_688,
            ),
            Quantization::new(
                "Q3_K_L",
                "Q3_K_L",
                "Index-Translate-35B-A3B-preview.Q3_K_L.gguf",
                18_552_089_728,
            ),
            Quantization::new(
                "IQ4_XS",
                "IQ4_XS",
                "Index-Translate-35B-A3B-preview.IQ4_XS.gguf",
                19_390_055_552,
            ),
            Quantization::new(
                "Q4_K_S",
                "Q4_K_S",
                "Index-Translate-35B-A3B-preview.Q4_K_S.gguf",
                20_366_861_440,
            ),
            Quantization::new(
                "Q5_K_S",
                "Q5_K_S",
                "Index-Translate-35B-A3B-preview.Q5_K_S.gguf",
                24_563_754_112,
            ),
            Quantization::new(
                "Q5_K_M",
                "Q5_K_M",
                "Index-Translate-35B-A3B-preview.Q5_K_M.gguf",
                25_347_531_904,
            ),
            Quantization::new(
                "Q6_K",
                "Q6_K",
                "Index-Translate-35B-A3B-preview.Q6_K.gguf",
                29_208_730_752,
            ),
            Quantization::new(
                "Q8_0",
                "Q8_0",
                "Index-Translate-35B-A3B-preview.Q8_0.gguf",
                37_802_148_992,
            ),
        ],
        // The Q8_0 projector, which the card recommends for local use.
        projector: Some(Projector::new(
            "Index-Translate-35B-A3B-preview.mmproj-Q8_0.gguf",
            610_655_072,
        )),
        markup: Some(&markup::FUNCTION_TAGS),
        kind: Some(TRANSLATION),
        sampling: GREEDY,
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
            if let Some(projector) = entry.projector {
                assert!(!projector.file.is_empty());
                assert!(
                    projector.bytes > 0,
                    "{} projector {} must have non-zero bytes",
                    entry.id,
                    projector.file
                );
            }
            for (index, quantization) in entry.quantizations.iter().enumerate() {
                assert!(!quantization.file.is_empty());
                assert!(
                    quantization.bytes > 0,
                    "{} {} must have non-zero bytes",
                    entry.id,
                    quantization.id
                );
                assert!(
                    !entry.quantizations[..index]
                        .iter()
                        .any(|other| other.id == quantization.id)
                );
            }
        }
    }
}
