#!/usr/bin/env python3
"""Generate optional device-check goldens from pinned models and publisher readers.

Dependencies used for the recorded comparison: onnxruntime 1.31.1, Pillow 13.0.0,
numpy 2.5.4, tokenizers, jaconv. Native ONNX Runtime remains 1.24.2.
Pass the model directory containing the verified lama/aot/manga/baberu/hayai folders.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import shutil
import tempfile
import urllib.request

import jaconv
import numpy as np
import onnxruntime as ort
from PIL import Image

APP = Path(__file__).resolve().parents[1]


def session(path):
    options = ort.SessionOptions()
    options.intra_op_num_threads = 4
    return ort.InferenceSession(str(path), options, providers=["CPUExecutionProvider"])


def publisher(url, name, directory):
    path = directory / (name + ".py")
    urllib.request.urlretrieve(url, path)
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--models", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    models, out = args.models, args.out
    # Fail before generating goldens if any supplied checkpoint differs.
    for model in json.loads((APP / "Resources/vision-models.json").read_text()):
        if model["id"] not in ["lama", "aot", "manga", "baberu", "hayai"]:
            continue
        for file in model["files"]:
            path = models / model["id"] / file["path"]
            assert path.stat().st_size == file["bytes"], path
            with path.open("rb") as source:
                assert hashlib.file_digest(source, "sha256").hexdigest() == file["sha256"], path
    out.mkdir(parents=True, exist_ok=True)
    fixture = APP.parents[1] / "crates/koharu-ml/benches/fixtures/ocr/title.png"
    shutil.copyfile(fixture, out / "title.png")
    image = Image.open(fixture).convert("RGB")
    pixels = np.asarray(image.convert("L").convert("RGB").resize((224, 224), Image.Resampling.BILINEAR), np.float32).transpose(2, 0, 1)[None] / 127.5 - 1
    encoder, decoder = session(models / "manga/encoder.onnx"), session(models / "manga/decoder.onnx")
    hidden = encoder.run(None, {"pixel_values": pixels})[0]
    ids = [2]
    for _ in range(299):
        token = int(np.argmax(decoder.run(None, {"input_ids": np.array([ids], np.int64), "encoder_hidden_states": hidden})[0][0, -1]))
        if token == 3:
            break
        ids.append(token)
    else:
        raise RuntimeError("Manga OCR did not finish")
    vocabulary = (models / "manga/vocab.txt").read_text().splitlines()
    text = "".join(vocabulary[x] for x in ids[1:])
    text = re.sub(r"##|\[UNK\]|\[PAD\]|\[CLS\]|\[MASK\]", "", text)
    text = re.sub(r"[.・]{2,}", lambda m: "." * len(m[0]), "".join(text.split()).replace("…", "..."))
    results = {"manga": {"raw": jaconv.h2z(text, ascii=True, digit=True)}}
    del encoder, decoder
    with tempfile.TemporaryDirectory() as directory:
        directory = Path(directory)
        baberu = publisher("https://huggingface.co/genshiai-daichi/baberu-ocr/resolve/b9868433dbda3ed9ba1357d008599ff4ada873d5/onnx_infer.py", "baberu_reference", directory)
        reader = baberu.BaberuOnnxOCR.__new__(baberu.BaberuOnnxOCR)
        reader.vis = session(models / "baberu/encoder.onnx")
        reader.pre = session(models / "baberu/prefill.onnx")
        reader.stp = session(models / "baberu/step.onnx")
        reader.vocab = baberu.Vocab(models / "baberu/vocab.json")
        results["baberu"] = {"raw": reader(image)}
        del reader
        hayai = publisher("https://huggingface.co/JustANormalTinkerer/hayai-ocr-v2-onnx/resolve/685ae08ce48db204268854863722bcb292021e0d/inference_onnx.py", "hayai_reference", directory)
        tokenizer, bos, eos, pad = hayai.load_tokenizer(models / "hayai/tokenizer.json")
        text, _ = hayai.ocr(str(fixture), session(models / "hayai/encoder.onnx"), session(models / "hayai/decoder.onnx"), np.load(models / "hayai/position_base.npy"), tokenizer, bos, eos, pad)
        results["hayai"] = {"raw": text}
    (out / "ocr-text.json").write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    canvas = Image.new("RGB", (512, 512), "white")
    canvas.paste(image, (80, 160))
    canvas.save(out / "inpaint.png")
    rgb = np.asarray(canvas, np.float32).transpose(2, 0, 1)[None] / 255
    mask = np.zeros((1, 1, 512, 512), np.float32)
    mask[:, :, 156:217, 76:364] = 1
    for model, key in [("lama", "image"), ("aot", "img")]:
        pixels = (rgb * 2 - 1) * (1 - mask) if model == "aot" else rgb
        output = session(models / model / "model.onnx").run(None, {key: pixels, "mask": mask})[0]
        if model == "aot":
            output = (output + 1) / 2
        output = np.clip(np.rint(output[0].transpose(1, 2, 0) * 255), 0, 255).astype("uint8")
        output = np.where(mask[0, 0, :, :, None] > 0, output, np.asarray(canvas))
        Image.fromarray(output).save(out / (model + "-reference.png"))
    print(json.dumps(results, ensure_ascii=False))


if __name__ == "__main__":
    main()
