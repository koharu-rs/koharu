use std::collections::BTreeMap;

use koharu_scene::{Face, Kernel, Script};
use serde::{Deserialize, Serialize};
use specta::Type;
use strum::VariantArray as _;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Type)]
#[serde(default)]
pub struct RendererConfig {
    /// The styles a new project starts with, and a file from before projects
    /// held their own takes: each kernel's face in each script, by ISO 15924
    /// code. A project keeps its own from then on.
    #[specta(type = BTreeMap<Kernel, BTreeMap<String, Face>>)]
    pub styles: BTreeMap<Kernel, BTreeMap<Script, Face>>,
}

/// Each kernel's default face by script: for Latin, the bundled font
/// scanlation groups' font guides set its kind of line in; for Cyrillic and
/// Greek, which those fonts lack, the dialogue font's italic thoughts and
/// captions and bold italic emphasis; for Chinese and Japanese, a bundled face
/// of the kernel, as Chinese groups keep the original's. A group's first
/// kernel's face is each of its kernels' that names none of its own. What a
/// text's weight shows, as a heavy shout's does, comes from its source, not
/// from these.
fn faces() -> BTreeMap<Kernel, BTreeMap<Script, Face>> {
    let face = |family: Option<&str>, weight: Option<u16>, italic: Option<bool>| Face {
        family: family.map(str::to_owned),
        weight,
        italic,
    };
    let named = |family: &str| face(Some(family), None, None);
    let slanted = |family: &str| face(Some(family), None, Some(true));
    let italic = face(None, None, Some(true));
    let emphatic = face(None, Some(700), Some(true));
    // Narration in CC Victory Speech, a lowercase font apart from dialogue's, as
    // Quicksand Scans' dialogue and narration card and Vorbis's guide set it,
    // upright, as its case alone sets it apart from the capitals of dialogue
    // and a slant crowds a long narrow box; shouts in CreativeBlock BB, from
    // Quicksand Scans' shout card; thoughts in Anime Ace, italic, as Gemelli's
    // font guide sets them, which tells them from dialogue's like capitals;
    // handwriting in Ashcan BB, from Quicksand Scans' handwriting card; side
    // text in Komika Slick, as Gemelli's guide sets it, for pop. Brush and the
    // horror faces take the Blambot dialogue fonts Blambot tags Brush and
    // Horror, and design faces' special voices, Voice Activated BB, from
    // Quicksand Scans' card of other fonts.
    // https://quicksandscans.wordpress.com/resources/
    // https://www.insidescanlation.com/etc/Vorbis-Typesetting-Guide/fonts.htm
    // https://docs.google.com/presentation/d/1hOvjfXMLuvTO6-XofTf4T6asAOp2DOOiYkTvLLj-XEc/htmlpresent
    let latin = vec![
        (Kernel::Antique, named("CCWildWords")),
        (Kernel::Mincho, named("CCVictorySpeechLower")),
        (Kernel::Gothic, named("CreativeBlock BB")),
        (Kernel::Rounded, slanted("Anime Ace 3 BB")),
        (Kernel::Handwriting, named("Ashcan BB")),
        (Kernel::Brush, named("Ninjutsu BB")),
        (Kernel::Kointai, named("ManEater BB")),
        (Kernel::Design, named("Voice Activated BB")),
        (Kernel::Pop, named("Komika Slick")),
        (Kernel::Decorative, named("ManEater BB")),
    ];
    let cyrillic = vec![
        (Kernel::Antique, named("CCWildWords")),
        (Kernel::Mincho, italic.clone()),
        (Kernel::Rounded, italic),
        (Kernel::Brush, emphatic.clone()),
        (Kernel::Design, emphatic),
    ];
    let simplified = vec![
        (Kernel::Antique, named("Adobe 黑体 Std")),
        (Kernel::Mincho, named("Adobe 宋体 Std")),
        // A gothic of every weight, so a heavy one is a real bold.
        (Kernel::Gothic, named("思源黑体 CN")),
        (Kernel::Rounded, named("方正准圆_GBK")),
        (Kernel::Handwriting, named("国祥手写体")),
        (Kernel::Brush, named("方正行楷_GBK")),
        (Kernel::Reisho, named("方正隶变_GBK")),
        (Kernel::Design, named("华康POP1体W5")),
    ];
    let traditional = vec![
        (Kernel::Antique, named("微軟正黑體")),
        (Kernel::Mincho, named("Adobe 明體 Std")),
        (Kernel::Rounded, named("華康儷特圓")),
        (Kernel::Handwriting, named("華康鋼筆體W2")),
        (Kernel::Brush, named("華康超特楷體斜")),
        (Kernel::Reisho, named("華康漆隸體斜W5")),
        (Kernel::Design, named("華康POP1體W7")),
    ];
    // The catalog has no Japanese antique or rounded face, so antique takes
    // a gothic and rounded takes antique's.
    let japanese = vec![
        (Kernel::Antique, named("小塚ゴシック Pr6N")),
        (Kernel::Mincho, named("小塚明朝 Pr6N")),
        (Kernel::Handwriting, named("瀬戸フォント")),
        (Kernel::Brush, named("CRＣ＆Ｇ由紀葉太楷書体")),
        (Kernel::Design, named("FOT-スランプ Std")),
    ];
    let columns = [
        ("Latn", latin),
        ("Cyrl", cyrillic.clone()),
        ("Grek", cyrillic),
        ("Hans", simplified),
        ("Hant", traditional),
        ("Jpan", japanese),
    ];
    let mut faces = BTreeMap::<_, BTreeMap<_, _>>::new();
    for (script, column) in columns {
        let script: Script = script.parse().expect("an ISO 15924 code");
        let column = BTreeMap::from_iter(column);
        for &kernel in Kernel::VARIANTS {
            if let Some(face) = (column.get(&kernel)).or_else(|| column.get(&kernel.group())) {
                faces
                    .entry(kernel)
                    .or_default()
                    .insert(script, face.clone());
            }
        }
    }
    faces
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self { styles: faces() }
    }
}

impl koharu_config::Section for RendererConfig {
    const NAME: &'static str = "renderer";
}
