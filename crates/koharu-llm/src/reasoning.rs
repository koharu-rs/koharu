//! rig's `Reasoning` in a setting, as one word: `off`, or an effort level's
//! own, as `reasoning = "high"` reads. rig's serde writes an effort as
//! `{ effort = "high" }`, where the settings a release wrote spell the word
//! alone. Use it as `#[serde(with = "koharu_llm::reasoning")]` on an
//! `Option<Reasoning>`.

use rig_core::completion::{Effort, Reasoning};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn serialize<S: Serializer>(
    reasoning: &Option<Reasoning>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match reasoning {
        Some(Reasoning::Effort(effort)) => serializer.serialize_some(effort),
        // `off` is rig's own word for it, and a budget keeps rig's form.
        other => other.serialize(serializer),
    }
}

pub fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Reasoning>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Word {
        Effort(Effort),
        Reasoning(Reasoning),
    }
    Ok(
        Option::<Word>::deserialize(deserializer)?.map(|word| match word {
            Word::Effort(effort) => Reasoning::Effort(effort),
            Word::Reasoning(reasoning) => reasoning,
        }),
    )
}

#[cfg(test)]
mod tests {
    use rig_core::completion::{Effort, Reasoning};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Default, Deserialize, PartialEq, Serialize)]
    #[serde(default)]
    struct Setting {
        #[serde(with = "super")]
        reasoning: Option<Reasoning>,
    }

    #[test]
    fn a_setting_writes_off_or_an_effort_as_one_word_and_reads_them_back() {
        for (reasoning, written) in [
            (Some(Reasoning::Off), "reasoning = \"off\"\n"),
            (
                Some(Reasoning::Effort(Effort::High)),
                "reasoning = \"high\"\n",
            ),
            (None, ""),
        ] {
            let setting = Setting { reasoning };
            assert_eq!(toml::to_string(&setting).expect("written"), written);
            assert_eq!(toml::from_str::<Setting>(written).expect("read"), setting);
        }
    }
}
