import { plainText } from '@/lib/document'
import { TARGET_LANGUAGES } from '@/lib/translation'
import type { Face, FontFace, FontFamily, Kernel, Text } from '@koharu/ipc/protocol'

/** The kernels by group, as the scene groups faces of one look, in its order, each group's first
 * naming it: Settings lists a group's kernels together. */
export const GROUPS: readonly (readonly Kernel[])[] = [
  ['antique'],
  ['mincho', 'fangsong', 'kaisho', 'textbook'],
  ['gothic'],
  ['rounded'],
  ['handwriting'],
  ['brush', 'gyosho', 'gyokai', 'sosho', 'reisho', 'tensho', 'kointai', 'weibei'],
  ['design', 'pop', 'decorative'],
]

/** Every kernel, by group. */
export const KERNELS: readonly Kernel[] = GROUPS.flat()

/** Faces by kernel, then by ISO 15924 script, as a project holds its styles and Preferences the
 * ones a new project starts with. */
export type Faces = Partial<{ [key in Kernel]: { [key in string]: Face } }>

/** The ISO 15924 script of `language`, its likely one as CLDR maximises it, as the renderer picks a
 * text's faces by; for an undetermined language, the one `content`'s kana or Hangul imply, or
 * Chinese for Han alone, else Latin. */
export function scriptOf(language: string, content = ''): string {
  const implied = /[\p{Script=Hiragana}\p{Script=Katakana}]/u.test(content)
    ? 'ja'
    : /\p{Script=Hangul}/u.test(content)
      ? 'ko'
      : /\p{Script=Han}/u.test(content)
        ? 'zh'
        : 'en'
  try {
    return new Intl.Locale(language === 'und' ? implied : language).maximize().script ?? 'Latn'
  } catch {
    return 'Latn'
  }
}

/** The scripts of the target languages, Latin first, each once. */
export const SCRIPTS: string[] = [
  ...new Set(['Latn', ...TARGET_LANGUAGES.map((language) => scriptOf(language))]),
]

/** `face`'s values, with `base`'s where it has none. */
function or(face: Face | undefined, base: Face | undefined): Face {
  return {
    family: face?.family ?? base?.family ?? null,
    weight: face?.weight ?? base?.weight ?? null,
    italic: face?.italic ?? base?.italic ?? null,
  }
}

/** The face `kernel` sets in `script` in `styles`. */
export function cellOf(
  kernel: Kernel,
  script: string,
  styles: Faces | undefined,
): Face | undefined {
  return styles?.[kernel]?.[script]
}

/** The face a text of `kernel` in `script` draws in where its own style sets nothing, as the
 * renderer resolves it from its project's `styles`: its kernel's, with what that leaves out from
 * antique's, and antique's with no kernel. */
export function faceIn(
  script: string,
  kernel: Kernel | null | undefined,
  styles: Faces | undefined,
): Face {
  const antique = cellOf('antique', script, styles)
  return or(kernel ? cellOf(kernel, script, styles) : undefined, antique)
}

/** The script `text` is drawn in. */
export function scriptOfText(text: Text): string {
  return scriptOf(text.language ?? 'und', plainText(text))
}

/** The face `text` draws in where its own style sets nothing, from its project's `styles`. */
export function faceOf(text: Text, styles: Faces | undefined): Face {
  return faceIn(scriptOfText(text), text.style?.kernel, styles)
}

/** The weight bold starts from, as the renderer counts it, and the one it sets. */
export const SEMIBOLD = 600
export const BOLD = 700

/** The heaviest weight `face` draws. */
export function heaviest(face: FontFace): number {
  return face.weight_range?.end ?? face.weight
}

/** The weights of `family`'s faces, and bold, which the renderer fakes for a family without it. */
export function usableFontWeights(family: FontFamily | undefined): number[] {
  const faces = family?.faces ?? []
  const weights = new Set(faces.length ? faces.map((face) => face.weight) : [400])
  if (!faces.some((face) => heaviest(face) >= SEMIBOLD)) weights.add(BOLD)
  for (const face of faces) {
    if (!face.weight_range) continue
    const { start, end } = face.weight_range
    weights.add(start)
    weights.add(end)
    for (let weight = 100; weight <= 900; weight += 100) {
      if (weight >= start && weight <= end) weights.add(weight)
    }
  }
  return [...weights].sort((left, right) => left - right)
}
