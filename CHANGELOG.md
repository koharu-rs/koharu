## [0.82.1](https://github.com/koharu-rs/koharu/compare/0.82.0..0.82.1) - 2026-09-11

### 🐛 Bug Fixes

- *(ci)* Pin AppImage helper source URL - ([a7431c5](https://github.com/koharu-rs/koharu/commit/a7431c5a5e7517cb7413309e4e4da5679efa20b1))

### ⚙️ Miscellaneous Tasks

- *(app)* Default the processing scope to the selection ([#1088](https://github.com/koharu-rs/koharu/issues/1088)) - ([767e4d0](https://github.com/koharu-rs/koharu/commit/767e4d0441f986c18e55fc4e1af42f3dddfd860e))


## [0.82.0](https://github.com/koharu-rs/koharu/compare/0.81.10..0.82.0) - 2026-09-11

### ⛰️  Features

- *(runtime)* Update native runtimes and CUDA 13.3 - ([67d507b](https://github.com/koharu-rs/koharu/commit/67d507b4cf3959208e87201f62b03ba771d3f372))

### ⚙️ Miscellaneous Tasks

- *(scripts)* Add Kindle JP downloader - ([8ccfe9f](https://github.com/koharu-rs/koharu/commit/8ccfe9ffafd8037e06c8999213dd55a78a6f7b37))


## [0.81.9](https://github.com/koharu-rs/koharu/compare/0.81.8..0.81.9) - 2026-09-08

### 🐛 Bug Fixes

- *(runtime)* Update Torch to v2.13.0.6 - ([7a6884a](https://github.com/koharu-rs/koharu/commit/7a6884a67b3c0a4b1f48f69eb59dd2230292b0c7))

### 🚜 Refactor

- *(runtime)* Simplify accelerator discovery - ([087b40a](https://github.com/koharu-rs/koharu/commit/087b40aabc9d26c1f0195916963f1980ac7d6f09))

### ⚙️ Miscellaneous Tasks

- Align contribution template wording with OSS conventions - ([edfbf84](https://github.com/koharu-rs/koharu/commit/edfbf8428d33e726ba8530c6b9ca9610015b62a5))
- Shorten issue template filenames - ([7f046cb](https://github.com/koharu-rs/koharu/commit/7f046cb7f59d9ff18103696f7d80ed5b34c8c9d3))
- Simplify contribution templates and use issue types - ([e29860a](https://github.com/koharu-rs/koharu/commit/e29860a75f551f1e0d5d4384eda8dc98aba97d73))


## [0.81.8](https://github.com/koharu-rs/koharu/compare/0.81.7..0.81.8) - 2026-09-08

### 🐛 Bug Fixes

- *(app)* Disable devtools to prevent F12 crash ([#1042](https://github.com/koharu-rs/koharu/issues/1042)) ([#1044](https://github.com/koharu-rs/koharu/issues/1044)) - ([fa9d907](https://github.com/koharu-rs/koharu/commit/fa9d90727c8187c965951e0c3762a548e5497250))
- *(runtime)* Skip unsupported Linux ROCm targets - ([e460a9b](https://github.com/koharu-rs/koharu/commit/e460a9bfd3b02ea2bbdb6701f956bb0df3cbd0b8))

### 📚 Documentation

- Align documentation with implementation - ([4f8216a](https://github.com/koharu-rs/koharu/commit/4f8216aec1e948c53aef6e63b0dcb7ff83142c2e))
- Migrate site to Mintlify ([#1049](https://github.com/koharu-rs/koharu/issues/1049)) - ([15ab5a6](https://github.com/koharu-rs/koharu/commit/15ab5a651916c9a0801ca612fda851cf74079866))

### ⚙️ Miscellaneous Tasks

- *(editor)* Change default brush color to pure white ([#1053](https://github.com/koharu-rs/koharu/issues/1053)) - ([6bdac20](https://github.com/koharu-rs/koharu/commit/6bdac20428388918564294cf8e8b98dcecf74c6c))
- Update feature request template name and label - ([fb1071a](https://github.com/koharu-rs/koharu/commit/fb1071a8e701128738de9b29b17d45034fd3dd54))


## [0.81.7](https://github.com/koharu-rs/koharu/compare/0.81.6..0.81.7) - 2026-09-05

### 🐛 Bug Fixes

- *(app)* Remove ineffective hide-chrome-bubbles flag - ([5e317b6](https://github.com/koharu-rs/koharu/commit/5e317b6255faf2802b5a7e3a58a2719f0e0a86c5))

### ⚙️ Miscellaneous Tasks

- Remove summary GHA - ([3a0c6a3](https://github.com/koharu-rs/koharu/commit/3a0c6a366d5355dee7a19cc905f28d84afa6e4b8))


## [0.81.6](https://github.com/koharu-rs/koharu/compare/0.81.5..0.81.6) - 2026-09-04

### 🐛 Bug Fixes

- *(app)* Pass Linux CEF switches correctly - ([24bda3a](https://github.com/koharu-rs/koharu/commit/24bda3a5cc9d47ed45b0d118cbf0711c3b46effe))


## [0.81.5](https://github.com/koharu-rs/koharu/compare/0.81.4..0.81.5) - 2026-09-03

### 🐛 Bug Fixes

- *(updater)* Install Windows updates quietly - ([e2df1f6](https://github.com/koharu-rs/koharu/commit/e2df1f60ad3850b79d78b224dc698de6c2d82c8b))


## [0.81.4](https://github.com/koharu-rs/koharu/compare/0.81.3..0.81.4) - 2026-09-03

### 🐛 Bug Fixes

- *(translator)* Recover model responses the JSON repair layer rejects ([#1022](https://github.com/koharu-rs/koharu/issues/1022)) - ([e6d330c](https://github.com/koharu-rs/koharu/commit/e6d330c83a76ed3efa1f7ab15988f81544df8219))


## [0.81.3](https://github.com/koharu-rs/koharu/compare/0.81.2..0.81.3) - 2026-09-03

### 🐛 Bug Fixes

- *(ci)* Remove AppImage bundle target ([#1035](https://github.com/koharu-rs/koharu/issues/1035)) ([#1037](https://github.com/koharu-rs/koharu/issues/1037)) - ([859ae68](https://github.com/koharu-rs/koharu/commit/859ae68a6c012afe349bedffc616e38a929b8f19))


## [0.81.2](https://github.com/koharu-rs/koharu/compare/0.81.1..0.81.2) - 2026-09-03

### 🐛 Bug Fixes

- *(agent)* Recover from invalidated Codex refresh tokens ([#1034](https://github.com/koharu-rs/koharu/issues/1034)) - ([5f299cd](https://github.com/koharu-rs/koharu/commit/5f299cd88e2ecfd834823a6e62068f420a72f73a))


## [0.81.1](https://github.com/koharu-rs/koharu/compare/0.81.0..0.81.1) - 2026-09-02

### ⚙️ Miscellaneous Tasks

- Build Linux ARM64 application - ([4add6d8](https://github.com/koharu-rs/koharu/commit/4add6d8d7a60f7a7cafc70a9b756fae4a3b0dd6a))


## [0.81.0](https://github.com/koharu-rs/koharu/compare/0.80.0..0.81.0) - 2026-09-02

### ⛰️  Features

- *(runtime)* Support Linux ARM64 CUDA - ([892db6b](https://github.com/koharu-rs/koharu/commit/892db6bfe0d4e1f461793341a26607bfddbba59a))
- *(runtime)* Support all ROCm 10 targets - ([1938566](https://github.com/koharu-rs/koharu/commit/193856650c85352eff530fbfbab75ef83e6679d9))

### 🐛 Bug Fixes

- *(runtime)* Align ROCm targets with llama.cpp - ([eadb797](https://github.com/koharu-rs/koharu/commit/eadb7976450ded81849d88b79e1d0f279da6c928))
- *(runtime)* Drop unsupported gfx1250 - ([64173ab](https://github.com/koharu-rs/koharu/commit/64173abbc4fd4c089bf5c3d8a9e350c483e4cbef))
- *(runtime)* Use exact ROCm library paths - ([eb06347](https://github.com/koharu-rs/koharu/commit/eb063470e06a8c8fdc2e510774996915c869fbba))

### 🚜 Refactor

- *(runtime)* Inline ROCm device library check - ([9152946](https://github.com/koharu-rs/koharu/commit/9152946a89e96fa2cc9ec78c7385d190afa0c014))

### 🧪 Testing

- *(runtime)* Remove gfx1250 reference - ([c3e79e6](https://github.com/koharu-rs/koharu/commit/c3e79e689179921c70de04cb5ac9913afb3ce8a5))

### ⚙️ Miscellaneous Tasks

- Update sponsors ([#943](https://github.com/koharu-rs/koharu/issues/943)) - ([687af18](https://github.com/koharu-rs/koharu/commit/687af180abcd5ecb69994d997acfcef115ad560f))


## [0.80.0](https://github.com/koharu-rs/koharu/compare/0.79.5..0.80.0) - 2026-08-31

### ⛰️  Features

- *(llama)* Sync utilityai wrapper changes - ([5b4b9b8](https://github.com/koharu-rs/koharu/commit/5b4b9b8e862e06b7a1e9e4d2bef383fe2b2f63c5))

### 📚 Documentation

- Improve contribution guidelines - ([971fed9](https://github.com/koharu-rs/koharu/commit/971fed9c16bf712d5bb1ddf9e46abaf67a018dfb))


## [0.79.5](https://github.com/koharu-rs/koharu/compare/0.79.4..0.79.5) - 2026-08-31

### 🐛 Bug Fixes

- *(ci)* Separate runner image caches - ([5ba1d97](https://github.com/koharu-rs/koharu/commit/5ba1d974ed81f8e40a3afc5d7673b52b5f650f72))
- *(ml)* Pass ggml Metal backend name MTL0 to FLUX.2 ([#1006](https://github.com/koharu-rs/koharu/issues/1006)) - ([01ba67a](https://github.com/koharu-rs/koharu/commit/01ba67a6e0686d2d39b456eeeab4e13a0e08b652))
- *(runtime)* Update native dependencies - ([1c736f7](https://github.com/koharu-rs/koharu/commit/1c736f70ffbc19cc23c37f92bbc99beb6d354f37))
- *(runtime)* Update ROCm to 10.0 - ([0147de1](https://github.com/koharu-rs/koharu/commit/0147de13c2f09375b2888a18f0ad84f4e34f600e))
- *(ui)* Select CJK glyph variants from the interface language ([#999](https://github.com/koharu-rs/koharu/issues/999)) - ([d22757d](https://github.com/koharu-rs/koharu/commit/d22757d0f8f7db85f0ef9ccb3306824b32eab919))

### 📚 Documentation

- Clarify hardware acceleration requirements - ([ba43b2d](https://github.com/koharu-rs/koharu/commit/ba43b2dc23687eecc8c141e2ffe8a13573386b38))

### ⚙️ Miscellaneous Tasks

- Use latest Windows runners - ([13d52c0](https://github.com/koharu-rs/koharu/commit/13d52c0a3e72ba05fb7aa7bd2f350afe43c3076c))

### Runtime

- Limit ROCm devices to official ROCm 10 - ([ccfdf34](https://github.com/koharu-rs/koharu/commit/ccfdf340c930601e814bdc790dbb017382ee1282))
- Support every ROCm 10 device target - ([f282f22](https://github.com/koharu-rs/koharu/commit/f282f22528218092d93a4453beb4619adc77c16c))


## [0.79.4](https://github.com/koharu-rs/koharu/compare/0.79.3..0.79.4) - 2026-08-28

### 🐛 Bug Fixes

- *(runtime)* Update Torch to v2.13.0.3 - ([fd0ca01](https://github.com/koharu-rs/koharu/commit/fd0ca01e0ceba2d198fde4b981da7efa8c0d32ed))
- *(runtime)* Update Torch release - ([2f308fa](https://github.com/koharu-rs/koharu/commit/2f308fa2729f6fd34155ada2a1686851e7f1213b))

### 📚 Documentation

- Update readme - ([4932a9d](https://github.com/koharu-rs/koharu/commit/4932a9d02a925b014f8d7f8e5768bf6932c2f134))


## [0.79.3](https://github.com/koharu-rs/koharu/compare/0.79.2..0.79.3) - 2026-08-27

### 🐛 Bug Fixes

- *(runtime)* Use dedicated release repositories - ([86ff7f0](https://github.com/koharu-rs/koharu/commit/86ff7f0f8a2fd23d6d0d0658751a31bf9604bdc3))

### ⚙️ Miscellaneous Tasks

- Update repository links - ([bf516a0](https://github.com/koharu-rs/koharu/commit/bf516a01f2c8a42d2c559410ef7070c67c81e227))
- Update codeowners - ([52dfe1e](https://github.com/koharu-rs/koharu/commit/52dfe1e6984c631ebae63cabec71a5e4f12310ac))


## [0.79.2](https://github.com/koharu-rs/koharu/compare/0.79.1..0.79.2) - 2026-08-27

### 🐛 Bug Fixes

- *(runtime)* Use koharu-rs release repositories - ([559aab7](https://github.com/koharu-rs/koharu/commit/559aab7b1328e31191ef1356fdc57103329bc3b4))


## [0.79.1](https://github.com/koharu-rs/koharu/compare/0.79.0..0.79.1) - 2026-08-27

### 🐛 Bug Fixes

- *(build)* Pin Tauri CEF toolchain - ([a9ce05f](https://github.com/koharu-rs/koharu/commit/a9ce05f83465382ebe866ed8943510b90b50cef6))


## [0.79.0](https://github.com/koharu-rs/koharu/compare/0.78.2..0.79.0) - 2026-08-26

### ⛰️  Features

- Add Hayai OCR model ([#988](https://github.com/koharu-rs/koharu/issues/988)) - ([156c5fc](https://github.com/koharu-rs/koharu/commit/156c5fcf6222030521fc5963e75d826c17c7e546))

### 🐛 Bug Fixes

- *(runtime)* Restore macOS Torch load order - ([10c440a](https://github.com/koharu-rs/koharu/commit/10c440aa960a714647094ad331f5f8c772fc56ee))
- Inpaint preview mask transparency ([#987](https://github.com/koharu-rs/koharu/issues/987)) - ([023bf7a](https://github.com/koharu-rs/koharu/commit/023bf7a6496dbf13344da9ddf39649987efb4609))


## [0.78.2](https://github.com/koharu-rs/koharu/compare/0.78.1..0.78.2) - 2026-08-26

### 🐛 Bug Fixes

- *(ci)* Extend native build timeout - ([19a7999](https://github.com/koharu-rs/koharu/commit/19a7999e3ba7a18b0036092990cbcff23ab4ec40))
- *(ci)* Use CodeBuild project defaults - ([bbb2f42](https://github.com/koharu-rs/koharu/commit/bbb2f421be6e11a4bc67a1c9743583aa07592c4b))
- *(ci)* Route CodeBuild jobs by platform - ([8a7472d](https://github.com/koharu-rs/koharu/commit/8a7472d4c920e51372543ec99185fe58183ab313))
- *(ci)* Apply explicit CodeBuild overrides - ([d47413c](https://github.com/koharu-rs/koharu/commit/d47413c0c99afeef5b88f5b75136f1edd0ea6070))
- *(ci)* Use CodeBuild large runners - ([b3730fa](https://github.com/koharu-rs/koharu/commit/b3730fa1b04be3d62de3279edcdbde9df8328ccd))
- *(ci)* Use CodeBuild Ubuntu 7 image - ([44f9053](https://github.com/koharu-rs/koharu/commit/44f905360851dc9aa2e364017f052be43ee827e7))
- *(ci)* Support PyTorch 2.13 Torch builds - ([12da3d9](https://github.com/koharu-rs/koharu/commit/12da3d947eeb8ebaaa3333582847263c07234535))
- *(ci)* Persist Torch ROCm architectures - ([c434b7f](https://github.com/koharu-rs/koharu/commit/c434b7fa06e1560f3d91a9ea5e28a93ef414e214))
- *(ci)* Repair Torch accelerator builds - ([16551b7](https://github.com/koharu-rs/koharu/commit/16551b7fba2d4578796003d2b1345014bb8270ca))
- *(ci)* Use Linux CUDA package names - ([4a840e1](https://github.com/koharu-rs/koharu/commit/4a840e179719234cc462f809da0c1f2252e76410))
- *(ci)* Enable long paths for Torch checkout - ([575b243](https://github.com/koharu-rs/koharu/commit/575b243b635ca2686bfda7ef4715256024d92068))
- *(ci)* Include prerelease llama.cpp releases - ([ea545e3](https://github.com/koharu-rs/koharu/commit/ea545e31dbf99e6f1972537b736cc2713aded890))
- *(linux)* Prevent CEF mallinfo overflow - ([67cffb8](https://github.com/koharu-rs/koharu/commit/67cffb8fc7826ab16bf6cd987c3b680fe70fe44c))
- *(ml)* Disable MIOpen on Windows gfx10 - ([8818911](https://github.com/koharu-rs/koharu/commit/8818911f79b3909e4919b0acf2da987364c558b2))
- *(ml)* Align RF-DETR inference with upstream - ([e09feb8](https://github.com/koharu-rs/koharu/commit/e09feb87b3c95896b95feb4ce78848f6be9108e0))
- *(torch)* Restore stream serialization - ([9f73d2c](https://github.com/koharu-rs/koharu/commit/9f73d2c003d873ec00e34d0ae600d308f20053ff))
- *(torch)* Disambiguate module handle in C++20 - ([ba313b4](https://github.com/koharu-rs/koharu/commit/ba313b476ad668bd3e57f45781d9b530159f7324))
- Disable MIOpen on Windows - ([874ed98](https://github.com/koharu-rs/koharu/commit/874ed98c07bc34ef43d5a99e7e7232fa0c39fd2e))
- Resolve clippy warnings - ([bcefc52](https://github.com/koharu-rs/koharu/commit/bcefc5219fc74c9d8b1ec9f4c4cd80d23c0273fe))

### 🚜 Refactor

- *(runtime)* Simplify Torch package variants - ([922f0f4](https://github.com/koharu-rs/koharu/commit/922f0f4840580b43b5c474272d54e926f62016de))
- *(runtime)* Use bundled native releases - ([b5cfaba](https://github.com/koharu-rs/koharu/commit/b5cfaba989a19e4b4f519205dbd8c02a9c1077b9))
- *(runtime)* Require pinned Hugging Face revisions - ([dbed34d](https://github.com/koharu-rs/koharu/commit/dbed34d8b5de8b6bbad39ae42852b1a13e06ba3f))
- Refactor ci - ([5fdf2e7](https://github.com/koharu-rs/koharu/commit/5fdf2e74c0955dafc541cd385d3a4cd70980316d))

### 📚 Documentation

- Update screenshot - ([d66ffb9](https://github.com/koharu-rs/koharu/commit/d66ffb9edb4efd734a3bd0a08644d560ad2a513f))

### 🧪 Testing

- *(torch)* Remove runtime-dependent serialization tests - ([38fc352](https://github.com/koharu-rs/koharu/commit/38fc352fd01697dd2d938575e577b812bd18116b))
- *(torch)* Align serialization tests with upstream - ([bf93005](https://github.com/koharu-rs/koharu/commit/bf930058dbd3f160c610979e66fabae20faaaf5a))

### ⚙️ Miscellaneous Tasks

- *(runtime)* Restore GitHub-hosted runners - ([c1870af](https://github.com/koharu-rs/koharu/commit/c1870af0505411fb68a2513a852a232217456ff7))
- *(runtime)* Use CodeBuild runners - ([db2539f](https://github.com/koharu-rs/koharu/commit/db2539f03efa1272c6e4d2cedefbfe5c6199158b))
- *(runtime)* Update llama.cpp - ([d3ce865](https://github.com/koharu-rs/koharu/commit/d3ce865b39203be67a5ed7163d54f2c8fa0367a8))
- *(torch)* Use CodeBuild runners - ([88565ea](https://github.com/koharu-rs/koharu/commit/88565ea2f7a2076abaa870941d3df685dc4ed9db))
- Move runtime builds to dedicated repositories - ([a81c582](https://github.com/koharu-rs/koharu/commit/a81c5829ea99a45e04580ff97fd6affa81b2db34))
- Rename workflows - ([904546a](https://github.com/koharu-rs/koharu/commit/904546a5cc00c84e8bf17de6c3337e652ad8acd0))
- Use gha cache - ([36e21c3](https://github.com/koharu-rs/koharu/commit/36e21c3f394959703e29d743b8e60bfe44ea95a8))
- Stream compiler cache to GitHub Actions - ([ddac1e4](https://github.com/koharu-rs/koharu/commit/ddac1e469bdd9de570a64b12bd35090ac2993f00))
- Simplify Torch release packaging - ([eb2a66a](https://github.com/koharu-rs/koharu/commit/eb2a66aa31f4186a1eb028d19ed72438286a1100))
- Optimize native runtimes and package Torch shim - ([93cc5f6](https://github.com/koharu-rs/koharu/commit/93cc5f66dbd4bd0a0345552f45030e1d2c00b9d9))
- Build Torch runtime releases - ([c7daa73](https://github.com/koharu-rs/koharu/commit/c7daa7322fcba56e89803698129d40d327452679))


## [0.78.1](https://github.com/koharu-rs/koharu/compare/0.78.0..0.78.1) - 2026-08-22

### 🐛 Bug Fixes

- *(ml)* Disambiguate image buffer access - ([e4c976b](https://github.com/koharu-rs/koharu/commit/e4c976b96ec7c33089ea1a9dcc1f5184c53ab574))

### 🚜 Refactor

- *(runtime)* Simplify downloads - ([7834852](https://github.com/koharu-rs/koharu/commit/7834852e9258a43af626cbc2d626c46b69446ce2))


## [0.78.0](https://github.com/koharu-rs/koharu/compare/0.77.5..0.78.0) - 2026-08-22

### ⛰️  Features

- *(translator)* Add uncensored Qwen 3.8 model - ([e4c3d06](https://github.com/koharu-rs/koharu/commit/e4c3d069f3c9221575ed20e28a6e4693b39dec2a))

### 🐛 Bug Fixes

- *(runtime)* Improve accelerator detection - ([e7f0cbc](https://github.com/koharu-rs/koharu/commit/e7f0cbced69e7d621b06f32286d4105b70979409))


## [0.77.5](https://github.com/koharu-rs/koharu/compare/0.77.4..0.77.5) - 2026-08-21

### 🐛 Bug Fixes

- *(metrics)* Exclude telemetry from logs - ([299e883](https://github.com/koharu-rs/koharu/commit/299e883538402a092364457b7a383f0b209b79de))
- *(runtime)* Isolate system HIP discovery - ([c1fce14](https://github.com/koharu-rs/koharu/commit/c1fce14c4f914b309fdc0acef29f3ecee45e69ea))
- *(runtime)* Reject unsupported CUDA architectures - ([dc2c044](https://github.com/koharu-rs/koharu/commit/dc2c0442cc7d768e7b1ef37a75c463081d75f0c8))
- *(translator)* Map Gemini thinking controls - ([f53631b](https://github.com/koharu-rs/koharu/commit/f53631b402f6a7fc981638f00176f1d5517796b5))
- *(ui)* Preserve quick settings across navigation - ([5b38393](https://github.com/koharu-rs/koharu/commit/5b38393633be7b7cafbdf20de49c839c48632769))

### 🚜 Refactor

- Refine metrics - ([5f87445](https://github.com/koharu-rs/koharu/commit/5f87445e8739c17c627d403060d61bf58ba1067a))

### ⚙️ Miscellaneous Tasks

- Remove unused params - ([c7d635c](https://github.com/koharu-rs/koharu/commit/c7d635c59f5608088eb28e41a7c23e1ced15854c))
- Fix lint - ([68be117](https://github.com/koharu-rs/koharu/commit/68be11737b14e573928639e8e547612f08340428))


## [0.77.4](https://github.com/koharu-rs/koharu/compare/0.77.3..0.77.4) - 2026-08-20

### 📚 Documentation

- Describe agent resources - ([dff193d](https://github.com/koharu-rs/koharu/commit/dff193de9fdbd6e40b3ec9995358b40572cb8164))
- Make contribution guidance more welcoming - ([765d327](https://github.com/koharu-rs/koharu/commit/765d327170ae8330a54ef4755465467f956cef70))

### ⚙️ Miscellaneous Tasks

- *(runtime)* Update native runtimes - ([6df4ded](https://github.com/koharu-rs/koharu/commit/6df4ded508658019e3b969ac9dc863d8fdccc9e1))


## [0.77.3](https://github.com/koharu-rs/koharu/compare/0.77.2..0.77.3) - 2026-08-19

### 🐛 Bug Fixes

- *(translator)* Decode optional Gemini thinking capability - ([3da3391](https://github.com/koharu-rs/koharu/commit/3da339141b56b97323e001ce79df260cfd2ddc27))


## [0.77.2](https://github.com/koharu-rs/koharu/compare/0.77.1..0.77.2) - 2026-08-19

### 🐛 Bug Fixes

- *(runtime)* Load ROCm libraries globally - ([1dda084](https://github.com/koharu-rs/koharu/commit/1dda08440a0b6d2115025ee9a8def543375083ff))


## [0.77.1](https://github.com/koharu-rs/koharu/compare/0.77.0..0.77.1) - 2026-08-19

### 🐛 Bug Fixes

- *(runtime)* Load Linux cuFile for Torch - ([556485b](https://github.com/koharu-rs/koharu/commit/556485b18651e952e9adee92825b5c02d66ed31d))


## [0.77.0](https://github.com/koharu-rs/koharu/compare/0.76.0..0.77.0) - 2026-08-19

### ⛰️  Features

- Add native usage metrics - ([66e603d](https://github.com/koharu-rs/koharu/commit/66e603d4438a2f61aee41cf6bbe38c36e97f8219))

### 🐛 Bug Fixes

- *(ci)* Cover supported ROCm targets in HIP builds - ([b855982](https://github.com/koharu-rs/koharu/commit/b855982c3e2eba09b3fc285535828d972b11e77e))
- *(ci)* Target dependency releases at default branch - ([d2b04ad](https://github.com/koharu-rs/koharu/commit/d2b04ad4337229bcab4355610f936ef09bfc1417))
- *(pipeline)* Refine detection and inpainting reruns - ([acb7d99](https://github.com/koharu-rs/koharu/commit/acb7d99c9855db8c09224655c84d5d68e359c2e5))


## [0.76.0](https://github.com/koharu-rs/koharu/compare/0.75.16..0.76.0) - 2026-08-19

### ⛰️  Features

- *(translator)* Support local reasoning - ([87104fc](https://github.com/koharu-rs/koharu/commit/87104fcf4d419e0de0f9d0cca7c404d87a0b6a14))

### 🐛 Bug Fixes

- *(translator)* Respect model capabilities - ([f842172](https://github.com/koharu-rs/koharu/commit/f842172e636628da0a018dfa1c4811d4d90f5301))
- *(translator)* Align reasoning capabilities - ([5b6672c](https://github.com/koharu-rs/koharu/commit/5b6672ccefb94a1afefcaf45190cb633b5b65442))


## [0.75.16](https://github.com/koharu-rs/koharu/compare/0.75.15..0.75.16) - 2026-08-19

### 🐛 Bug Fixes

- *(ci)* Restore Windows 2022 app builds - ([bf7a6ea](https://github.com/koharu-rs/koharu/commit/bf7a6ea9036e7eb422bc298d13e9e1e181ba190a))
- *(ci)* Stabilize dependency builds - ([2486494](https://github.com/koharu-rs/koharu/commit/248649469d15213ef2d64146a8bd8320ebf7edd2))
- *(editor)* Replace text centered stroke with outward path ([#951](https://github.com/koharu-rs/koharu/issues/951)) - ([fbc2e4f](https://github.com/koharu-rs/koharu/commit/fbc2e4f11b6f0f7a815b00590a6bcaae574b2008))

### ⚙️ Miscellaneous Tasks

- Use ubuntu-latest runners - ([dda25ba](https://github.com/koharu-rs/koharu/commit/dda25baa1f70722c87de10ecc5216a0e742331a7))
- Use windows-latest runners - ([d4997ca](https://github.com/koharu-rs/koharu/commit/d4997cad5dd1855f4b11e63ff250ba852e09864e))


## [0.75.15](https://github.com/koharu-rs/koharu/compare/0.75.14..0.75.15) - 2026-08-19

### 🐛 Bug Fixes

- *(ci)* Avoid MSVC environment during app builds - ([7ab39f1](https://github.com/koharu-rs/koharu/commit/7ab39f14676ef0ff39166cfa1cc70a334af7dbbb))

### ⚙️ Miscellaneous Tasks

- Enable Linux AppImage bundling - ([6d4d303](https://github.com/koharu-rs/koharu/commit/6d4d303ea0cacbfba3ffa70280e2f915cda15ac6))


## [0.75.14](https://github.com/koharu-rs/koharu/compare/0.75.13..0.75.14) - 2026-08-19

### 🐛 Bug Fixes

- *(runtime)* Preserve ROCm wheel layout - ([b830b49](https://github.com/koharu-rs/koharu/commit/b830b4955cd7411070a5526dea53caf0754c9578))

### ⚙️ Miscellaneous Tasks

- Optimize release caches - ([9aba741](https://github.com/koharu-rs/koharu/commit/9aba7419dbc9358bb77951995b691cfffd24f49c))


## [0.75.13](https://github.com/koharu-rs/koharu/compare/0.75.12..0.75.13) - 2026-08-19

### 🐛 Bug Fixes

- *(runtime)* Share ROCm runtime on Linux - ([c6dab0c](https://github.com/koharu-rs/koharu/commit/c6dab0c076129551627744ac491b77f3bde4d66d))


## [0.75.12](https://github.com/koharu-rs/koharu/compare/0.75.11..0.75.12) - 2026-08-19

### 🐛 Bug Fixes

- *(ui)* Gate vision by model capability - ([80b03f1](https://github.com/koharu-rs/koharu/commit/80b03f16d7340fd0560720f18806f31425f44216))


## [0.75.11](https://github.com/koharu-rs/koharu/compare/0.75.10..0.75.11) - 2026-08-18

### 🐛 Bug Fixes

- *(runtime)* Isolate bundled ROCm libraries on Linux - ([6c21c9a](https://github.com/koharu-rs/koharu/commit/6c21c9add22e97ef75c7aa6459eb1c1c76ef1614))

### ⚙️ Miscellaneous Tasks

- Disable OpenMP for stable-diffusion.cpp - ([77de3a4](https://github.com/koharu-rs/koharu/commit/77de3a4d4d9f28fe290344e83c7cc3f9330bd313))


## [0.75.10](https://github.com/koharu-rs/koharu/compare/0.75.9..0.75.10) - 2026-08-18

### 🐛 Bug Fixes

- *(runtime)* Load CUDA Torch dependencies in order - ([0d20664](https://github.com/koharu-rs/koharu/commit/0d2066409ed827761fbe5fc782cded94d13825b1))


## [0.75.9](https://github.com/koharu-rs/koharu/compare/0.75.8..0.75.9) - 2026-08-18

### 🐛 Bug Fixes

- *(translator)* Use LM Studio vision capability - ([8ab7cde](https://github.com/koharu-rs/koharu/commit/8ab7cde12bfc8fc9bae2012c531a8442cd13426a))


## [0.75.8](https://github.com/koharu-rs/koharu/compare/0.75.7..0.75.8) - 2026-08-18

### 🐛 Bug Fixes

- *(runtime)* Isolate bundled DRM symbols on Linux - ([fb74b75](https://github.com/koharu-rs/koharu/commit/fb74b7527bf1ddc3bd832cde09f3e2cc01f3c319))

### ⚙️ Miscellaneous Tasks

- Cache on release - ([9772b7c](https://github.com/koharu-rs/koharu/commit/9772b7cde02f5fd3896834d41a0a218243971e46))


## [0.75.7](https://github.com/koharu-rs/koharu/compare/0.75.6..0.75.7) - 2026-08-18

### 🐛 Bug Fixes

- *(app)* Limit persistent log size - ([a4e8b4d](https://github.com/koharu-rs/koharu/commit/a4e8b4d029f2304eabbdc4a00059d67ee9cbd261))


## [0.75.6](https://github.com/koharu-rs/koharu/compare/0.75.5..0.75.6) - 2026-08-18

### 🐛 Bug Fixes

- *(ml)* Select precision by device capability - ([ce2e6fb](https://github.com/koharu-rs/koharu/commit/ce2e6fbe2a4a31f7355c5c892ee9c5f4a6083e54))
- *(runtime)* Retry interrupted download ranges - ([1143fe7](https://github.com/koharu-rs/koharu/commit/1143fe77dd1179e4e477c1776aff8d5f62382dbb))
- *(runtime)* Correct Linux cuDNN library manifest - ([84ef3ee](https://github.com/koharu-rs/koharu/commit/84ef3ee5ed6c40e6ba0a2e9f0b2e92d25c761969))


## [0.75.5](https://github.com/koharu-rs/koharu/compare/0.75.4..0.75.5) - 2026-08-18

### ⛰️  Features

- *(app)* Persist backend logs - ([3587243](https://github.com/koharu-rs/koharu/commit/3587243199c39335755ca28669dd171f412d8b6f))


## [0.75.4](https://github.com/koharu-rs/koharu/compare/0.75.3..0.75.4) - 2026-08-18

### 🐛 Bug Fixes

- *(runtime)* Load ROCm Torch dependencies in order - ([01bc970](https://github.com/koharu-rs/koharu/commit/01bc97057d89169a5251dec9236707bb087f25b8))


## [0.75.3](https://github.com/koharu-rs/koharu/compare/0.75.2..0.75.3) - 2026-08-18

### 🐛 Bug Fixes

- *(ui)* Preserve model toggle preferences - ([63764d6](https://github.com/koharu-rs/koharu/commit/63764d6c28647cd992200474be164fc5faad6d47))


## [0.75.2](https://github.com/koharu-rs/koharu/compare/0.75.1..0.75.2) - 2026-08-18

### 🐛 Bug Fixes

- *(app)* Restore frameless window resizing - ([d8bddc4](https://github.com/koharu-rs/koharu/commit/d8bddc426b48758c8feec39694438c8699956d26))


## [0.75.1](https://github.com/koharu-rs/koharu/compare/0.75.0..0.75.1) - 2026-08-18

### ⚡ Performance

- *(runtime)* Accelerate multipart downloads - ([22dab37](https://github.com/koharu-rs/koharu/commit/22dab373485245bbe1ce9ec2b35ca7a72ba56d45))


## [0.75.0](https://github.com/koharu-rs/koharu/compare/0.74.0..0.75.0) - 2026-08-18

### ⛰️  Features

- *(runtime)* Add Linux CUDA and ROCm artifacts - ([cf914e8](https://github.com/koharu-rs/koharu/commit/cf914e8b92aee5829e6a6a03edd08a893b0eda95))

### 🐛 Bug Fixes

- *(ci)* Use Linux CUDA package names - ([2a05c67](https://github.com/koharu-rs/koharu/commit/2a05c67c24736c7bc117961c27c954ca5676276f))

### 📚 Documentation

- Improve readme feature list - ([b015225](https://github.com/koharu-rs/koharu/commit/b0152251435d6fff9afe3c746a76014e1150169c))
- Improve readme - ([316795b](https://github.com/koharu-rs/koharu/commit/316795b93fe66f1abfea07af30cd98924b456a5d))
- Document hardware runtime support - ([2d38f14](https://github.com/koharu-rs/koharu/commit/2d38f14ef4c76ad00355f7fe3ccb2004f7525c05))
- Expand debug logging guide - ([cba2629](https://github.com/koharu-rs/koharu/commit/cba26293a26b9777a29769099c636e41d30f6454))

### ⚡ Performance

- *(runtime)* Maximize download throughput - ([b608a9f](https://github.com/koharu-rs/koharu/commit/b608a9f7f7311969fd7c5a31c0d1eb81c153c03b))

### ⚙️ Miscellaneous Tasks

- Automate native dependency releases - ([9cd3100](https://github.com/koharu-rs/koharu/commit/9cd3100515c17fbb0007bc456c5e47add6a49a55))


## [0.74.0](https://github.com/koharu-rs/koharu/compare/0.73.0..0.74.0) - 2026-08-18

### ⛰️  Features

- *(runtime)* Support ROCm Torch on Linux - ([7462036](https://github.com/koharu-rs/koharu/commit/7462036cbda998e4517b3925d3117d8f36549f3f))

### 🐛 Bug Fixes

- *(app)* Hide link status bubble - ([002ec11](https://github.com/koharu-rs/koharu/commit/002ec11bd6133c082cf5989ac57ddb4f0f2ef46a))


## [0.73.0](https://github.com/koharu-rs/koharu/compare/0.72.0..0.73.0) - 2026-08-17

### ⛰️  Features

- *(ocr)* Tighten PaddleOCR-VL sampling - ([653eb14](https://github.com/koharu-rs/koharu/commit/653eb14b02d23693a036a5e9f5391a97b572f9c1))
- *(translator)* Tune local model sampling - ([9962e14](https://github.com/koharu-rs/koharu/commit/9962e14c1a4ac77bfc102eb97275bdde37fc30f8))
- *(translator)* Strengthen translation prompt - ([efb28e3](https://github.com/koharu-rs/koharu/commit/efb28e3054344035ca1cf35e1d439b1a0b3f0160))
- Sort projects by recent activity - ([43d6930](https://github.com/koharu-rs/koharu/commit/43d69300482422ae5835baaf9bcf83a9e8b14c8a))
- Sort projects by recent use - ([369de57](https://github.com/koharu-rs/koharu/commit/369de5704aa8d7882f4d7dc1e0a60153ec6b5045))

### 🐛 Bug Fixes

- Refine provider and translation settings - ([138b2ff](https://github.com/koharu-rs/koharu/commit/138b2ff68ddc7af087845547fbd11a030799cb45))
- Improve provider credential controls - ([a09f25d](https://github.com/koharu-rs/koharu/commit/a09f25d382628b3c7f5343c87e02acf8cb5bf7e0))

### ⚡ Performance

- *(translator)* Optimize structured inference - ([e47620c](https://github.com/koharu-rs/koharu/commit/e47620c06479b6d3e422687dc048289fed8b1abf))


## [0.72.0](https://github.com/koharu-rs/koharu/compare/0.71.0..0.72.0) - 2026-08-17

### ⛰️  Features

- *(translator)* Discover provider model catalogs - ([412822d](https://github.com/koharu-rs/koharu/commit/412822d2c9d542fa39b963ad8e0d1f0a2820c7a4))
- *(translator)* Add Grok and MiniMax providers - ([800e32b](https://github.com/koharu-rs/koharu/commit/800e32bb4902d48919313b6ba4ebe05247b7e07a))
- *(translator)* Refresh remote model catalogs - ([08601a8](https://github.com/koharu-rs/koharu/commit/08601a8fbffbc7cb802e791d50483635d12f51e2))


## [0.71.0](https://github.com/koharu-rs/koharu/compare/0.70.6..0.71.0) - 2026-08-17

### ⛰️  Features

- Support archive and PDF page imports - ([79cc4d6](https://github.com/koharu-rs/koharu/commit/79cc4d6cf62ee96a071a1ea6d86b834a879755ce))

### 🐛 Bug Fixes

- *(renderer)* Apply language-aware line breaking - ([d4c075f](https://github.com/koharu-rs/koharu/commit/d4c075fe95bcf511e91a4a15eff6a1f433658dc5))


## [0.70.6](https://github.com/koharu-rs/koharu/compare/0.70.5..0.70.6) - 2026-08-17

### 🐛 Bug Fixes

- Restore app icons - ([faf9f42](https://github.com/koharu-rs/koharu/commit/faf9f42d83212dda2d1c65743c46d167658bf64a))


## [0.70.5](https://github.com/koharu-rs/koharu/compare/0.70.4..0.70.5) - 2026-08-17

### 🐛 Bug Fixes

- *(macos)* Titlebar style - ([19ec24e](https://github.com/koharu-rs/koharu/commit/19ec24ee76764f1c2b4e6e8433f4a8fbb6bc0eb7))

### ⚙️ Miscellaneous Tasks

- *(dev)* Fix macos development - ([2169d0d](https://github.com/koharu-rs/koharu/commit/2169d0d298e4f40003f9379990945a3049e68f2c))


## [0.70.4](https://github.com/koharu-rs/koharu/compare/0.70.3..0.70.4) - 2026-08-16


## [0.70.3](https://github.com/koharu-rs/koharu/compare/0.70.2..0.70.3) - 2026-08-16

### 🐛 Bug Fixes

- *(runtime)* Disable broken gfx1032 inpainting solver - ([8199a8b](https://github.com/koharu-rs/koharu/commit/8199a8b20e5cbfb4affba569475d66bda0eb4572))


## [0.70.2](https://github.com/koharu-rs/koharu/compare/0.70.1..0.70.2) - 2026-08-16

### 🐛 Bug Fixes

- *(runtime)* Disable broken gfx1032 Winograd solver - ([4d9f3ac](https://github.com/koharu-rs/koharu/commit/4d9f3acad1232ab6ba6c3832ab1e99848c5270d2))


## [0.70.1](https://github.com/koharu-rs/koharu/compare/0.70.0..0.70.1) - 2026-08-16

### 🐛 Bug Fixes

- *(editor)* Preserve automatic text direction - ([cd57c82](https://github.com/koharu-rs/koharu/commit/cd57c827ad0375744b7160f65044bd661d0bde5c))
- *(ui)* Avoid localized hydration mismatch - ([c108afb](https://github.com/koharu-rs/koharu/commit/c108afb119d3ddb68f9bc2e9a71ab0619853a48f))


## [0.70.0](https://github.com/koharu-rs/koharu/compare/0.69.1..0.70.0) - 2026-08-16

### ⛰️  Features

- *(translator)* Enforce keyed translation output - ([6269da2](https://github.com/koharu-rs/koharu/commit/6269da2c4f1d259e28abc9bb72fc76c9a299a49f))

### 🐛 Bug Fixes

- *(renderer)* Respect text direction defaults - ([098d697](https://github.com/koharu-rs/koharu/commit/098d697f982fb66300eb7ff900085d29270f5afa))
- Configure CEF remote debugging in debug builds - ([eaee34e](https://github.com/koharu-rs/koharu/commit/eaee34e3ad39732a93ea8550a3a3503d91ef9a99))

### 📚 Documentation

- Update development prerequisites - ([fc183d8](https://github.com/koharu-rs/koharu/commit/fc183d89546eaa670416c3010271d79b28442dff))


## [0.69.1](https://github.com/koharu-rs/koharu/compare/0.69.0..0.69.1) - 2026-08-16

### 🐛 Bug Fixes

- Compress PNG exports - ([c3d06fb](https://github.com/koharu-rs/koharu/commit/c3d06fbd65c2d7ff47120f8c9c4a00a996b87290))
- Preserve canvas zoom and vertical text - ([28968b4](https://github.com/koharu-rs/koharu/commit/28968b4414e23e7d882f69642ba1efdcdeb0f443))
- Brush freeze bug, Change brush size with alt + scroll wheel ([#937](https://github.com/koharu-rs/koharu/issues/937)) - ([0ebc038](https://github.com/koharu-rs/koharu/commit/0ebc038442c6cb92dd8943e06f632837f9b19863))


## [0.69.0](https://github.com/koharu-rs/koharu/compare/0.68.0..0.69.0) - 2026-08-15

### ⛰️  Features

- *(translator)* Add Qwen 3.8 27B model - ([c17d7a0](https://github.com/koharu-rs/koharu/commit/c17d7a075c866f30eba1464975f5b34ae4ca5141))

### 🐛 Bug Fixes

- *(ui)* Align updater header content - ([d80649d](https://github.com/koharu-rs/koharu/commit/d80649d0a803b2b06db2d966b631d8a4d4f6e383))


## [0.68.0](https://github.com/koharu-rs/koharu/compare/0.67.3..0.68.0) - 2026-08-15

### ⛰️  Features

- *(i18n)* Detect preferred language - ([a6defc2](https://github.com/koharu-rs/koharu/commit/a6defc2cb98c2a145a4f6ed7ae8e3b593c8f5c68))

### 🐛 Bug Fixes

- *(ui)* Keep translation controls usable while saving - ([a0df79a](https://github.com/koharu-rs/koharu/commit/a0df79a1b04a2d416bfb46ebe30d95db78246cc7))


## [0.67.3](https://github.com/koharu-rs/koharu/compare/0.67.2..0.67.3) - 2026-08-15

### 🐛 Bug Fixes

- *(desktop)* Enable Linux WebGPU Vulkan - ([6e19d68](https://github.com/koharu-rs/koharu/commit/6e19d6877feb17973f5b78d489ae633925b846f9))


## [0.67.2](https://github.com/koharu-rs/koharu/compare/0.67.1..0.67.2) - 2026-08-15

### 🚜 Refactor

- Use Tauri with CEF - ([a6c71bd](https://github.com/koharu-rs/koharu/commit/a6c71bd06673bbc428cb878987fa135968dc879f))


## [0.67.1](https://github.com/koharu-rs/koharu/compare/0.67.0..0.67.1) - 2026-08-15

### 🐛 Bug Fixes

- *(ui)* Preserve output picker state while saving - ([7fe1d29](https://github.com/koharu-rs/koharu/commit/7fe1d29d77c5c0308bba28f4eac1fb89dc544431))

### ⚙️ Miscellaneous Tasks

- Enable devtools - ([49883fc](https://github.com/koharu-rs/koharu/commit/49883fce9bbbf9d25ea830abb738bd4375fc5fd5))


## [0.67.0](https://github.com/koharu-rs/koharu/compare/0.66.10..0.67.0) - 2026-08-15

### ⛰️  Features

- Update diffusion and ggml FFI bindings - ([6959411](https://github.com/koharu-rs/koharu/commit/6959411c7ccabab694683bb95d155f92ef90ccd6))

### 🐛 Bug Fixes

- Correct updater dialog presentation - ([6a9a3d8](https://github.com/koharu-rs/koharu/commit/6a9a3d8f9ebc35a17e4250910379154f85d07724))
- Disable viewport scaling - ([4cfebec](https://github.com/koharu-rs/koharu/commit/4cfebec4bf76400575b1d7a8fe26a1a7eac4858b))

### 🚜 Refactor

- WebGPU/WASM-based canvas - ([6ebec47](https://github.com/koharu-rs/koharu/commit/6ebec470c29200e2120309ce92b59f460b8037cc))
- Restructure application components - ([b87b93b](https://github.com/koharu-rs/koharu/commit/b87b93b28886d9e4ab71980edf0eea0c8eca3741))

### 🧪 Testing

- *(koharu)* Mock canvas WASM module in Vitest - ([1d35a33](https://github.com/koharu-rs/koharu/commit/1d35a33522a1c722100c4a0331596352ac94b47e))

### ⚙️ Miscellaneous Tasks

- Update sponsors ([#927](https://github.com/koharu-rs/koharu/issues/927)) - ([106a99c](https://github.com/koharu-rs/koharu/commit/106a99cccc656168742b53f304c6abbd4cf28cf9))
- Bump llama.cpp and sd.cpp - ([c4fddad](https://github.com/koharu-rs/koharu/commit/c4fddad24c77e345353788fae5c795bc329108bc))
- Remove unused workspace dependencies - ([cac8f75](https://github.com/koharu-rs/koharu/commit/cac8f75718ad2585bd449c9e85d725deba777529))


## [0.66.10](https://github.com/koharu-rs/koharu/compare/0.66.9..0.66.10) - 2026-08-14

### 🐛 Bug Fixes

- *(desktop)* Restore macOS keyboard input - ([2f178c1](https://github.com/koharu-rs/koharu/commit/2f178c1a9f3964f583ef1ce000e538dc62f9dde0))
- *(renderer)* Support tiled large images and mask geometry - ([7ac6e09](https://github.com/koharu-rs/koharu/commit/7ac6e0912b406a22885bac503b53e3f44efb632d))
- *(windows)* Attach to the parent console on startup - ([a8f649f](https://github.com/koharu-rs/koharu/commit/a8f649f7977db6d60fe1df5e177f2be6f823c554))
- Recover from an out-of-range detection threshold ([#912](https://github.com/koharu-rs/koharu/issues/912)) - ([dceb848](https://github.com/koharu-rs/koharu/commit/dceb8485471a8acaaa564e0160b599cc35de8de3))


## [0.66.9](https://github.com/koharu-rs/koharu/compare/0.66.8..0.66.9) - 2026-08-13

### 🐛 Bug Fixes

- Render IME preedit without yellow background - ([e81c0a1](https://github.com/koharu-rs/koharu/commit/e81c0a16210152b711d1e33aa6545e3876a183ce))


## [0.66.8](https://github.com/koharu-rs/koharu/compare/0.66.7..0.66.8) - 2026-08-13

### 🐛 Bug Fixes

- IME on CEF window - ([7a0b96b](https://github.com/koharu-rs/koharu/commit/7a0b96bdf9a7ae98c91b132777d026e797143eaf))
- Libkoharu-torch.so path - ([1626c7f](https://github.com/koharu-rs/koharu/commit/1626c7f64ba83d0d7f6d57916a6ffb2b2c0d1429))


## [0.66.7](https://github.com/koharu-rs/koharu/compare/0.66.6..0.66.7) - 2026-08-13

### 🐛 Bug Fixes

- Relax revision validation - ([f5467d4](https://github.com/koharu-rs/koharu/commit/f5467d4ae4a2eebc9a80cc55ed283f46a9d7410d))
- Only configure store path for windows - ([f228507](https://github.com/koharu-rs/koharu/commit/f22850771edb9e7544d59678e6b985892e8b84ee))


## [0.66.6](https://github.com/koharu-rs/koharu/compare/0.66.5..0.66.6) - 2026-08-13

### 🐛 Bug Fixes

- Run CEF subprocesses before Tokio - ([b608c14](https://github.com/koharu-rs/koharu/commit/b608c14437f290b779028a8003ee38a9c8697198))

### ⚙️ Miscellaneous Tasks

- Prefer rust cache - ([93b3f50](https://github.com/koharu-rs/koharu/commit/93b3f50eafb497c0908e902ebd9eb6cb290cd89c))


## [0.66.5](https://github.com/koharu-rs/koharu/compare/0.66.4..0.66.5) - 2026-08-13

### 🐛 Bug Fixes

- Runtime discover - ([c6f2271](https://github.com/koharu-rs/koharu/commit/c6f2271b26d995e5c3b671c34f33201eaee88a1c))

### 🚜 Refactor

- Remove usage-based eviction - ([47dd0d2](https://github.com/koharu-rs/koharu/commit/47dd0d29ac44e30d17fbc3d98caef05da612207b))


## [0.66.4](https://github.com/koharu-rs/koharu/compare/0.66.3..0.66.4) - 2026-08-13

### 🐛 Bug Fixes

- Missing ROCm files - ([ff37a02](https://github.com/koharu-rs/koharu/commit/ff37a02002b3480d9f74630f393f464313fe97c1))


## [0.66.3](https://github.com/koharu-rs/koharu/compare/0.66.2..0.66.3) - 2026-08-13

### 🐛 Bug Fixes

- Put store in resource dir - ([9590631](https://github.com/koharu-rs/koharu/commit/959063121cf15330d41c8acd03f2ffab60a6a2ed))


## [0.66.2](https://github.com/koharu-rs/koharu/compare/0.66.1..0.66.2) - 2026-08-13

### 🐛 Bug Fixes

- Bundle CEF correctly - ([10d2ae8](https://github.com/koharu-rs/koharu/commit/10d2ae8fd88fa32bd9818471e03f902b000e9989))


## [0.66.1](https://github.com/koharu-rs/koharu/compare/0.66.0..0.66.1) - 2026-08-13

### 🐛 Bug Fixes

- Update dialog overflow - ([66ce7c6](https://github.com/koharu-rs/koharu/commit/66ce7c635ef4d8fbcfbe445c7fc929746063fa3a))

### ⚙️ Miscellaneous Tasks

- Update sponsors ([#903](https://github.com/koharu-rs/koharu/issues/903)) - ([fe8075e](https://github.com/koharu-rs/koharu/commit/fe8075e9b67e9dca7275758149173a8b25f0e640))


## [0.66.0](https://github.com/koharu-rs/koharu/compare/0.65.3..0.66.0) - 2026-08-13

### ⛰️  Features

- CEF OSR - ([9379006](https://github.com/koharu-rs/koharu/commit/937900683eea385a89e0e6e7fc0065cc757e1fa2))
- Use CEF runtime - ([547106f](https://github.com/koharu-rs/koharu/commit/547106f17e8183046fb22572b532f6755eef02f7))


## [0.65.3](https://github.com/koharu-rs/koharu/compare/0.65.2..0.65.3) - 2026-08-13

### 🐛 Bug Fixes

- Restore Windows packaged app startup and icon - ([99dd895](https://github.com/koharu-rs/koharu/commit/99dd895aee94c18919062b215c48c132efe3de93))


## [0.65.2](https://github.com/koharu-rs/koharu/compare/0.65.1..0.65.2) - 2026-08-12


## [0.65.1](https://github.com/koharu-rs/koharu/compare/0.65.0..0.65.1) - 2026-08-12


## [0.65.0](https://github.com/koharu-rs/koharu/compare/0.64.2..0.65.0) - 2026-08-12

### ⛰️  Features

- Updater & improve canvas - ([0b7b5f2](https://github.com/koharu-rs/koharu/commit/0b7b5f295f5e89f7180af0783566f8613623c2d8))

### 🐛 Bug Fixes

- Support multiple GPUs - ([f0acda1](https://github.com/koharu-rs/koharu/commit/f0acda102f0f165828cbc79f2b97740d4e8ca7bb))

### 🚜 Refactor

- Use CEF to replace Tauri - ([920faac](https://github.com/koharu-rs/koharu/commit/920faac6c5cea2209d68ee31588bc5cba3f4f9ee))


## [0.64.2](https://github.com/koharu-rs/koharu/compare/0.64.1..0.64.2) - 2026-08-12

### 🐛 Bug Fixes

- Windows path bug - ([981703c](https://github.com/koharu-rs/koharu/commit/981703c7ad7afffa1871bae80309b3f2b35a5047))
- Support hip 6.x - ([e7e9378](https://github.com/koharu-rs/koharu/commit/e7e93783cbb6e254d21731cad3de96c70dfeb213))


## [0.64.1](https://github.com/koharu-rs/koharu/compare/0.64.0..0.64.1) - 2026-08-12

### 🐛 Bug Fixes

- Only enable bf16 when compute capability >= 80 - ([3b9ca08](https://github.com/koharu-rs/koharu/commit/3b9ca08e3027cb77ae3bec08c74fabf387b4e9ca))


## [0.64.0](https://github.com/koharu-rs/koharu/compare/0.63.5..0.64.0) - 2026-08-12


## [0.63.5](https://github.com/koharu-rs/koharu/compare/0.63.4..0.63.5) - 2026-08-12

### 🐛 Bug Fixes

- Device selection - ([e7a8d07](https://github.com/koharu-rs/koharu/commit/e7a8d075adf86be40dd258c68404561db7e33a1e))


## [0.63.4](https://github.com/koharu-rs/koharu/compare/0.63.3..0.63.4) - 2026-08-11


## [0.63.3](https://github.com/koharu-rs/koharu/compare/0.63.2..0.63.3) - 2026-08-11

### ⚙️ Miscellaneous Tasks

- *(ci)* Skip appimage - ([9d3d913](https://github.com/koharu-rs/koharu/commit/9d3d9131d9ce6cdea96e3fc09e7ed0bed99feeb1))


## [0.63.2](https://github.com/koharu-rs/koharu/compare/0.63.1..0.63.2) - 2026-08-11


## [0.63.1](https://github.com/koharu-rs/koharu/compare/0.63.0..0.63.1) - 2026-08-11


## [0.63.0](https://github.com/koharu-rs/koharu/compare/0.61.2..0.63.0) - 2026-08-11

### ⛰️  Features

- *(llm)* Add Atlas Cloud provider ([#881](https://github.com/koharu-rs/koharu/issues/881)) - ([0fdacd4](https://github.com/koharu-rs/koharu/commit/0fdacd4f1446e2092e11c199951a11b7dc5e0114))
- *(translator)* Add Atlas Cloud provider - ([69d87bf](https://github.com/koharu-rs/koharu/commit/69d87bf4552fb366f382e91ad46291c3f6723115))
- Canvas Scrolling Tweaks ([#897](https://github.com/koharu-rs/koharu/issues/897)) - ([71d0416](https://github.com/koharu-rs/koharu/commit/71d041655be60550ead29012ced683824dfc38a4))
- Delete Page Confirmation ([#809](https://github.com/koharu-rs/koharu/issues/809)) - ([2107843](https://github.com/koharu-rs/koharu/commit/2107843f0c7e2458de5a329980c78575401babb5))
- Added a "delete selected textblocks" button and keyboard shortcut ([#808](https://github.com/koharu-rs/koharu/issues/808)) - ([d62bec2](https://github.com/koharu-rs/koharu/commit/d62bec2a0ea5585f650bccf84bde3971d3b44715))

### 🐛 Bug Fixes

- *(ci)* Update sponsors through pull requests [skip ci] - ([bf78ff0](https://github.com/koharu-rs/koharu/commit/bf78ff0a3b112ab52671810feaee43fb5fc4439b))
- *(sponsors)* Handle anonymous Patreon members [skip ci] - ([d9ef27e](https://github.com/koharu-rs/koharu/commit/d9ef27ed07dab8cbdb8ff6f3cf82972c8b4fee0a))
- Various Delete Function Fixes ([#801](https://github.com/koharu-rs/koharu/issues/801)) - ([e1fced9](https://github.com/koharu-rs/koharu/commit/e1fced9151b9af358025c10f5729b33bdc2d9ba9))
- Allow deleting font size value completely. ([#851](https://github.com/koharu-rs/koharu/issues/851)) - ([f4ce039](https://github.com/koharu-rs/koharu/commit/f4ce03999ed1ae2faaec938dd52c2f41a87d03d9))
- Fix bug where progress indicator doesn't progress ([#845](https://github.com/koharu-rs/koharu/issues/845)) - ([ea1e9f9](https://github.com/koharu-rs/koharu/commit/ea1e9f9fe6fd5de2c1fa814b81fdc7ab4a7aecd5))
- Frontend dist path - ([2dd9a65](https://github.com/koharu-rs/koharu/commit/2dd9a6540d75d0541f630719c893e386a8d80a77))

### 🚜 Refactor

- *(renderer)* Remove google fonts & bundle new fonts - ([90f6793](https://github.com/koharu-rs/koharu/commit/90f6793758df7180d196a499e910b6b77cb54dbd))
- Rebuild scene graph - ([76c7bd0](https://github.com/koharu-rs/koharu/commit/76c7bd0a256864f11a97f92f06663d78810a9240))
- Integrate canvas, storage, renderer, and desktop - ([fddfa9c](https://github.com/koharu-rs/koharu/commit/fddfa9c747d793dba2b3b7a0bbd8b1b45d9eacba))
- Rebuild Koharu application and ML architecture - ([d53d3d7](https://github.com/koharu-rs/koharu/commit/d53d3d702805ba523be74c9daef2fd1fdb317112))
- Split secrets into koharu-secrets - ([430ff6f](https://github.com/koharu-rs/koharu/commit/430ff6f6274ce6c1648c1204721a592dc2b816f1))
- Move crates to crates/ - ([81a3cd1](https://github.com/koharu-rs/koharu/commit/81a3cd135c69c7202325e9c79875a1d28bc44ea4))

### 📚 Documentation

- Small edit to README - ([876dfe9](https://github.com/koharu-rs/koharu/commit/876dfe981948397f9434ed0fb5953c0a60283786))
- Add homebrew - ([4bf2d3e](https://github.com/koharu-rs/koharu/commit/4bf2d3eb0efb77e9bc01001757dc1b2f933d1dc8))

### ⚡ Performance

- Optimize pipeline, rendering, and inpainting - ([eb05f4d](https://github.com/koharu-rs/koharu/commit/eb05f4d275d75296f985ad0a1bdaa3c054d8f9db))

### ⚙️ Miscellaneous Tasks

- *(ci)* Enable GHA - ([a278086](https://github.com/koharu-rs/koharu/commit/a278086acc0de7f95dd0fb3b5f8c07cddf9e70cc))
- *(ci)* Remove docker - ([b224395](https://github.com/koharu-rs/koharu/commit/b224395ff9c1d8bda91c82c6ba54c2a154f002b6))
- *(dev)* Switched to Microsoft recommended way of finding MSVC ([#849](https://github.com/koharu-rs/koharu/issues/849)) - ([00966be](https://github.com/koharu-rs/koharu/commit/00966beef1ba0f069d16b2d62a0a45c388bb922b))
- *(docs)* Update README.md [skip ci] - ([f61c65b](https://github.com/koharu-rs/koharu/commit/f61c65b7104ab4bce9bdf5ff2f692b8667b72988))
- *(ui)* Cleaned up the styling of the text style buttons ([#854](https://github.com/koharu-rs/koharu/issues/854)) - ([6291d50](https://github.com/koharu-rs/koharu/commit/6291d507f4d2c527898368517342d66466eb26b5))
- Update sponsors ([#898](https://github.com/koharu-rs/koharu/issues/898)) - ([d016e46](https://github.com/koharu-rs/koharu/commit/d016e46ba1b69579e065c69e7cbc71a8d6d66f8c))
- Update sponsors ([#896](https://github.com/koharu-rs/koharu/issues/896)) - ([fb31702](https://github.com/koharu-rs/koharu/commit/fb3170202599dcff40ec30bea1e402bcefccc634))
- Update sponsors ([#889](https://github.com/koharu-rs/koharu/issues/889)) - ([9a162fc](https://github.com/koharu-rs/koharu/commit/9a162fcdf067cdd57f8672a841c1463e1d1bc616))
- Update sponsors ([#888](https://github.com/koharu-rs/koharu/issues/888)) - ([7940e03](https://github.com/koharu-rs/koharu/commit/7940e035fcfb349f6c2ce007c8a3ce80189df693))
- Remove mockup - ([59aa81e](https://github.com/koharu-rs/koharu/commit/59aa81ecec73e58fba65f2fef836ac26b5b196e7))
- Update sponsors ([#882](https://github.com/koharu-rs/koharu/issues/882)) - ([dbe3843](https://github.com/koharu-rs/koharu/commit/dbe3843843e5951eca7f04ae679fde03209ece21))
- Add SponsorKit graph automation - ([bb52e2f](https://github.com/koharu-rs/koharu/commit/bb52e2f3ae1d6de9f8cefa1798bd6eaaebe97a95))
- Use wildcard - ([3462e51](https://github.com/koharu-rs/koharu/commit/3462e51e5eb287129fae9b5cd9d97f166c7b860b))
- Stop publishing bare exe on windows - ([a8d3bdc](https://github.com/koharu-rs/koharu/commit/a8d3bdc905a3f2132fac35ea48667ab67c91d669))
- Configure dependabot - ([f54ad1f](https://github.com/koharu-rs/koharu/commit/f54ad1f4b061be2ab322bccc539ac5d49f7f1cf7))

### Build

- Package native runtimes across supported platforms - ([7b79680](https://github.com/koharu-rs/koharu/commit/7b796801fbecdf934f887b1230982ea1179c82f8))


## [0.61.2](https://github.com/koharu-rs/koharu/compare/0.61.1..0.61.2) - 2026-06-16

### 🐛 Bug Fixes

- Upgrade ZLUDA to v6-preview.77 - ([2c34daa](https://github.com/koharu-rs/koharu/commit/2c34daa5117e5b6f530170d365865a01c9b4839b))

### 📚 Documentation

- RUST_LOG=debug - ([90d061c](https://github.com/koharu-rs/koharu/commit/90d061cabc4e63b907cca6fa2fed5a6b9d266119))

### ⚙️ Miscellaneous Tasks

- Update gemini models ([#790](https://github.com/koharu-rs/koharu/issues/790)) - ([2f3d648](https://github.com/koharu-rs/koharu/commit/2f3d64834fd016968640582b7af53c42900c1ae3))


## [0.61.1](https://github.com/koharu-rs/koharu/compare/0.61.0..0.61.1) - 2026-06-11

### 🐛 Bug Fixes

- Pin windows-2022 ([#772](https://github.com/koharu-rs/koharu/issues/772)) - ([8d00453](https://github.com/koharu-rs/koharu/commit/8d004530b4467fa11a3dddc3a58662d5179ebb26))


## [0.61.0](https://github.com/koharu-rs/koharu/compare/0.60.0..0.61.0) - 2026-06-10

### ⛰️  Features

- Page Deletion ([#727](https://github.com/koharu-rs/koharu/issues/727)) - ([639fda7](https://github.com/koharu-rs/koharu/commit/639fda7513562096fe763eee74635013575ce0f4))
- Custom Processing Pipeline ([#744](https://github.com/koharu-rs/koharu/issues/744)) - ([19e1400](https://github.com/koharu-rs/koharu/commit/19e1400331ee5c310427cceb4a6a4c7f9a94a4de))
- Project Deletion ([#753](https://github.com/koharu-rs/koharu/issues/753)) - ([ab82b6f](https://github.com/koharu-rs/koharu/commit/ab82b6f185b5a646efd1ed84803c6f756792707c))

### 📚 Documentation

- Add paddleocr-vl-1.6 and gemma4-12b-it - ([3995dce](https://github.com/koharu-rs/koharu/commit/3995dcea33bf4dff721542730dc28d6424f9cd92))

### ⚙️ Miscellaneous Tasks

- *(dev)* Do not check scripts - ([b29480c](https://github.com/koharu-rs/koharu/commit/b29480cb0fd0267857a66c9a5293b64421eddb1e))
- Add ARM64 build support ([#757](https://github.com/koharu-rs/koharu/issues/757)) - ([2822d18](https://github.com/koharu-rs/koharu/commit/2822d18bc547a298692cc8ad67c7e446394ae5e7))
- Hu-HU label translation ([#762](https://github.com/koharu-rs/koharu/issues/762)) - ([e83ff7b](https://github.com/koharu-rs/koharu/commit/e83ff7b4f53432357afaa90c18dd8204ebc83539))


## [0.60.0](https://github.com/koharu-rs/koharu/compare/0.59.2..0.60.0) - 2026-06-09

### ⛰️  Features

- Add gemma4-12b-it - ([d65d730](https://github.com/koharu-rs/koharu/commit/d65d73045bd6ea5f0652cbf8c5fadf4e6d938737))
- PaddleOCR-VL-1.6 - ([e583a2d](https://github.com/koharu-rs/koharu/commit/e583a2d4199050043e12aebbd15cf196becb694e))
- Add Hungarian language ([#759](https://github.com/koharu-rs/koharu/issues/759)) - ([e86080d](https://github.com/koharu-rs/koharu/commit/e86080daa142c9f48cbe438e29da5d7e721dbdf0))

### ⚙️ Miscellaneous Tasks

- *(dev)* Remove unused script - ([ccecce6](https://github.com/koharu-rs/koharu/commit/ccecce6054314e55e9ec2058b7761084899b3255))
- *(tests)* Remove unused warning - ([4ec5a15](https://github.com/koharu-rs/koharu/commit/4ec5a15b2030a7767340a4a60535537452cbc1eb))


## [0.59.2](https://github.com/koharu-rs/koharu/compare/0.59.1..0.59.2) - 2026-05-23

### 🐛 Bug Fixes

- Orval mock type - ([7f0e58c](https://github.com/koharu-rs/koharu/commit/7f0e58c2a6c6fa995c02285e8971c73368d48e8e))
- Remove Aggressive sceneEpoch Triggering ([#715](https://github.com/koharu-rs/koharu/issues/715)) - ([9a6b98e](https://github.com/koharu-rs/koharu/commit/9a6b98e2bf3e61d3137e9df957c388bba05fbe66))
- Text Layers Rasterizing on PSD Exports ([#710](https://github.com/koharu-rs/koharu/issues/710)) - ([6fb2411](https://github.com/koharu-rs/koharu/commit/6fb2411b714f5316307513ec38c866135dfc6b02))
- Cuda vulkan thread isolation ([#702](https://github.com/koharu-rs/koharu/issues/702)) - ([4b528af](https://github.com/koharu-rs/koharu/commit/4b528af44e013b46db872456cadff79ec8a6d2af))

### ⚙️ Miscellaneous Tasks

- Update README with icon image ([#703](https://github.com/koharu-rs/koharu/issues/703)) - ([19a9b8f](https://github.com/koharu-rs/koharu/commit/19a9b8fdd369cc49b1434243f04b15171e840f50))


## [0.59.1](https://github.com/koharu-rs/koharu/compare/0.59.0..0.59.1) - 2026-05-12

### 🐛 Bug Fixes

- *(llm)* Honor --cpu by zeroing n_gpu_layers ([#309](https://github.com/koharu-rs/koharu/issues/309)) ([#692](https://github.com/koharu-rs/koharu/issues/692)) - ([2a12c2c](https://github.com/koharu-rs/koharu/commit/2a12c2ca1580d330b4848939489dfcdcee42f372))


## [0.59.0](https://github.com/koharu-rs/koharu/compare/0.58.0..0.59.0) - 2026-05-11

### ⛰️  Features

- Font favorite + weights/styles implementation ([#670](https://github.com/koharu-rs/koharu/issues/670)) - ([5ca5677](https://github.com/koharu-rs/koharu/commit/5ca5677a195955a89ccf8cc93e4c336f19a22705))
- Reading Order Dropdown + LTR Reading Order ([#660](https://github.com/koharu-rs/koharu/issues/660)) - ([88625e2](https://github.com/koharu-rs/koharu/commit/88625e28ac0dd7a8be9173f5a44b8ad084453e02))


## [0.58.0](https://github.com/koharu-rs/koharu/compare/0.57.0..0.58.0) - 2026-05-01

### 🐛 Bug Fixes

- Per-block translate - ([0d4f899](https://github.com/koharu-rs/koharu/commit/0d4f899aed0f29df9a82043a08607997d4a99632))
- PSD text placement & editable - ([854209e](https://github.com/koharu-rs/koharu/commit/854209e1ab010fe147e33041615f33b551d708a7))
- Render on textless page - ([b3e9dea](https://github.com/koharu-rs/koharu/commit/b3e9dea3299915b21ba76003e194fb7bd92d8715))
- Black color not apply - ([4ffaa1d](https://github.com/koharu-rs/koharu/commit/4ffaa1dfdc4240570530227f64a7ecfcaa134ed9))
- Manual textbox modification should lock box - ([c43f3a2](https://github.com/koharu-rs/koharu/commit/c43f3a242a37d6c39be877e62e8e8df3d7a54df8))
- I18n languages display names - ([56e0784](https://github.com/koharu-rs/koharu/commit/56e078405483192753a5f025406d4f7bf2120199))


## [0.57.0](https://github.com/koharu-rs/koharu/compare/0.56.0..0.57.0) - 2026-05-01

### ⛰️  Features

- Chinese segmentation jieba - ([a23ebf1](https://github.com/koharu-rs/koharu/commit/a23ebf17515839bbedfdd8495eae9f65c78e114d))
- Anime-text model - ([b1ed7e6](https://github.com/koharu-rs/koharu/commit/b1ed7e6113742f738d0f843c20f673fbd364cc9d))


## [0.56.0](https://github.com/koharu-rs/koharu/compare/0.55.0..0.56.0) - 2026-05-01

### ⛰️  Features

- Better text rendering - ([973be25](https://github.com/koharu-rs/koharu/commit/973be2580d97903bac328714f0c9306b32c2761b))

### ⚙️ Miscellaneous Tasks

- Cleanup unused deps - ([6f82531](https://github.com/koharu-rs/koharu/commit/6f825319a04ad719521be040a3cc357cdb100f6e))

### Fix

- Centering Inconsistencies on Text Render ([#630](https://github.com/koharu-rs/koharu/issues/630)) - ([800c909](https://github.com/koharu-rs/koharu/commit/800c9090688056a6354bedfb497b4069ddbf63bf))


## [0.55.0](https://github.com/koharu-rs/koharu/compare/0.54.0..0.55.0) - 2026-04-28

### ⛰️  Features

- Better text rendering - ([83d79db](https://github.com/koharu-rs/koharu/commit/83d79dbac1dc8c0ae8e34b659f516a97a1be3bbe))


## [0.54.0](https://github.com/koharu-rs/koharu/compare/0.53.0..0.54.0) - 2026-04-28

### ⛰️  Features

- Add more cloud models - ([9c119a1](https://github.com/koharu-rs/koharu/commit/9c119a1e45c9036941da00b1089da8924e074a61))

### 🐛 Bug Fixes

- *(editor)* Delete key removes all selected text blocks ([#559](https://github.com/koharu-rs/koharu/issues/559)) ([#599](https://github.com/koharu-rs/koharu/issues/599)) - ([91cce6f](https://github.com/koharu-rs/koharu/commit/91cce6f72b11e081f300e3251e8d6c31f18cf20f))
- Revert inpainting logic for AOT/Lama ([#609](https://github.com/koharu-rs/koharu/issues/609)) - ([8e92960](https://github.com/koharu-rs/koharu/commit/8e92960faf404cb2e3482eaeb5402813d4a7ca9b))

### Fix

- Brush and Mask Layer not syncing on undo/redo ([#557](https://github.com/koharu-rs/koharu/issues/557)) - ([3cc8ac9](https://github.com/koharu-rs/koharu/commit/3cc8ac9506375c18b1f73457202df08f131baa32))


## [0.53.0](https://github.com/koharu-rs/koharu/compare/0.52.0..0.53.0) - 2026-04-26

### ⛰️  Features

- Qwen3.6 - ([3f0ec29](https://github.com/koharu-rs/koharu/commit/3f0ec29405678579f6e066d542a542cdc1e6ac8f))

### 🐛 Bug Fixes

- Filename overflow - ([c431e82](https://github.com/koharu-rs/koharu/commit/c431e823194e048ee5dee947644906f6b17f54b0))


## [0.52.0](https://github.com/koharu-rs/koharu/compare/0.51.0..0.52.0) - 2026-04-26

### 🐛 Bug Fixes

- *(llm)* Update Gemma 4 model IDs with hyphens ([#581](https://github.com/koharu-rs/koharu/issues/581)) - ([9f3eb17](https://github.com/koharu-rs/koharu/commit/9f3eb17ce8ade1e5f409d8ef526b64c33218ba9a))
- ZLUDA compatibility issues ([#582](https://github.com/koharu-rs/koharu/issues/582)) - ([e9fa6cd](https://github.com/koharu-rs/koharu/commit/e9fa6cdb4daf3acb76ae58b4e42409e268ee8ed6))


## [0.51.0](https://github.com/koharu-rs/koharu/compare/0.50.2..0.51.0) - 2026-04-26

### ⛰️  Features

- Gemma models to gemini models ([#578](https://github.com/koharu-rs/koharu/issues/578)) - ([fa7b9b7](https://github.com/koharu-rs/koharu/commit/fa7b9b7603d142cffc8f0e540f7eb26b6791fb92))

### 🐛 Bug Fixes

- Prefer 4000 port - ([a214dcf](https://github.com/koharu-rs/koharu/commit/a214dcfa89208aa7e3dc4bcb64286daf21c8dfd8))


## [0.50.2](https://github.com/koharu-rs/koharu/compare/0.50.1..0.50.2) - 2026-04-26

### ⚡ Performance

- Prefer bf16 - ([7f6f616](https://github.com/koharu-rs/koharu/commit/7f6f616fdb76c415cab97ca9122e85a887be09e9))


## [0.50.1](https://github.com/koharu-rs/koharu/compare/0.50.0..0.50.1) - 2026-04-26

### 🐛 Bug Fixes

- Include cudnn in zluda - ([33c0aff](https://github.com/koharu-rs/koharu/commit/33c0affbda03e8d9e79a18e23cc05f2d2dc48fe8))
- Do not uppercase translation - ([1afef77](https://github.com/koharu-rs/koharu/commit/1afef7742589212f49608f6f3574169172af25e1))

### 📚 Documentation

- Winget - ([8f9f0eb](https://github.com/koharu-rs/koharu/commit/8f9f0eb1ca3a3ebf8e75c825ec8d3ac1073486c4))

### ⚙️ Miscellaneous Tasks

- Update ko_KR/translation.json ([#573](https://github.com/koharu-rs/koharu/issues/573)) - ([60a2be4](https://github.com/koharu-rs/koharu/commit/60a2be40843d8463bfa9746d83d92de8ec506158))


## [0.50.0](https://github.com/koharu-rs/koharu/compare/0.49.0..0.50.0) - 2026-04-25

### ⛰️  Features

- Collapsible Navigator ([#571](https://github.com/koharu-rs/koharu/issues/571)) - ([8c925f0](https://github.com/koharu-rs/koharu/commit/8c925f0cd5c860e318d5615dfb07206e00a8718d))

### 🐛 Bug Fixes

- *(ui)* Open updater release-note links externally ([#565](https://github.com/koharu-rs/koharu/issues/565)) - ([5f309a7](https://github.com/koharu-rs/koharu/commit/5f309a79364b350762e082dbdbe694bfedb47fb3))
- [**breaking**] Require CUDA compute cap 8.0 - ([4a71a64](https://github.com/koharu-rs/koharu/commit/4a71a64a401bdda55591c00acfcc4f3b034d1bcd))
- Implement universal locale-aware uppercase conversion for all no… ([#572](https://github.com/koharu-rs/koharu/issues/572)) - ([973e2d4](https://github.com/koharu-rs/koharu/commit/973e2d465d90c900b3d4cfff35cab4553a8105b1))
- Implement universal locale-aware uppercase conversion using ICU ([#567](https://github.com/koharu-rs/koharu/issues/567)) - ([ca4cf81](https://github.com/koharu-rs/koharu/commit/ca4cf81724d0ae61722fa9b5afbe7ad00c743848))

### 📚 Documentation

- Add codex - ([8b90145](https://github.com/koharu-rs/koharu/commit/8b90145260cb1a2ba16ace48db38e6f8ece45425))

### ⚡ Performance

- FLUX.2 improvements - ([a66436f](https://github.com/koharu-rs/koharu/commit/a66436f75668389742f25e89a3349e6841659789))

### ⚙️ Miscellaneous Tasks

- Add Winget Releaser workflow ([#570](https://github.com/koharu-rs/koharu/issues/570)) - ([e4529ed](https://github.com/koharu-rs/koharu/commit/e4529edba9aad4f2a16029eb23591edf33dad1a0))


## [0.49.0](https://github.com/koharu-rs/koharu/compare/0.48.0..0.49.0) - 2026-04-25

### ⛰️  Features

- Codex integration - ([9d8af42](https://github.com/koharu-rs/koharu/commit/9d8af427a563432957c9f1acb70189829966a943))

### 🐛 Bug Fixes

- Only apply speech bubble mask logic to flux.2 - ([5e29e13](https://github.com/koharu-rs/koharu/commit/5e29e136b0a31161e79edf19fba649d07a04d69e))

### 📚 Documentation

- Add FLUX.2 Klein 4B - ([9d33bb6](https://github.com/koharu-rs/koharu/commit/9d33bb6a73d76ca564baf65b08b9fb33dc061ea1))


## [0.48.0](https://github.com/koharu-rs/koharu/compare/0.47.8..0.48.0) - 2026-04-24

### ⛰️  Features

- Refactor Brush Tool Settings to Sub-ToolRail panel ([#537](https://github.com/koharu-rs/koharu/issues/537)) - ([d30ca44](https://github.com/koharu-rs/koharu/commit/d30ca44d8fe91a4c7f98538e7174e22374b90046))
- Flux.2 Klein inpainting - ([ac81ff7](https://github.com/koharu-rs/koharu/commit/ac81ff7850b2168572252048b8d000de0e07d6e2))
- Koharu-ai crate - ([87acca5](https://github.com/koharu-rs/koharu/commit/87acca53a93b71e8237af37ad138821cc487f4b2))

### 🚜 Refactor

- Move keyring to koharu-runtime - ([9821bf7](https://github.com/koharu-rs/koharu/commit/9821bf7cec33a5915507490ab5b50b22d615e941))

### ⚡ Performance

- Use small-decoder for flux-2-klein-4b - ([dfe33a3](https://github.com/koharu-rs/koharu/commit/dfe33a367c0f9e277d00727425a68772de218b16))

### ⚙️ Miscellaneous Tasks

- *(dev)* Fix args parsing - ([e331a6e](https://github.com/koharu-rs/koharu/commit/e331a6e0b1883f0386ac81f767b7957678a4bc0b))


## [0.47.8](https://github.com/koharu-rs/koharu/compare/0.47.7..0.47.8) - 2026-04-23

### 🐛 Bug Fixes

- Show auto when not manual override for font size - ([e2f1d2b](https://github.com/koharu-rs/koharu/commit/e2f1d2b9462f2cab3b57e80888cb8e45cb03f6c6))

### 📚 Documentation

- Update to match current implementation - ([0f2a31b](https://github.com/koharu-rs/koharu/commit/0f2a31bc7ec2a390513181031249d1027a0c5114))
- Improve readme - ([6095446](https://github.com/koharu-rs/koharu/commit/6095446104dd9715fa0f625dbc0fa2d4537a4a4b))

### ⚙️ Miscellaneous Tasks

- Rust 1.95.0 clippy fix - ([a1ba983](https://github.com/koharu-rs/koharu/commit/a1ba983ee5f23d94d4d746f99159fbed7c67dc62))


## [0.47.7](https://github.com/koharu-rs/koharu/compare/0.47.6..0.47.7) - 2026-04-22

### 🐛 Bug Fixes

- Repeat guard for paddleocr-vl - ([103bb2c](https://github.com/koharu-rs/koharu/commit/103bb2c5252886ef12d97a68b80f2f73e9f8b0bf))

### ⚙️ Miscellaneous Tasks

- Adjust MAX_NEW_TOKENS to 256 - ([4cff3ea](https://github.com/koharu-rs/koharu/commit/4cff3eaf9d0ed0cdda17529f2acff60a3c7bd178))


## [0.47.6](https://github.com/koharu-rs/koharu/compare/0.47.5..0.47.6) - 2026-04-22

### 🐛 Bug Fixes

- Add repetition_penalty to PaddleOcrVl - ([ae80ce9](https://github.com/koharu-rs/koharu/commit/ae80ce9aa54197b8095d3d32a964e704d9ef25c3))

### ⚙️ Miscellaneous Tasks

- Make clippy happy - ([c38ac38](https://github.com/koharu-rs/koharu/commit/c38ac38062bf2b0f0c1257ae11e1eeec9b4c2994))


## [0.47.5](https://github.com/koharu-rs/koharu/compare/0.47.4..0.47.5) - 2026-04-22

### 🐛 Bug Fixes

- Keyring on Linux - ([6ee6aed](https://github.com/koharu-rs/koharu/commit/6ee6aed139d5a83a4c77b40e500ec13111da7d61))

### ⚙️ Miscellaneous Tasks

- Make clippy happy - ([0472ea9](https://github.com/koharu-rs/koharu/commit/0472ea9fe8256c6c20653ec9c3e72b9f57fa8ef1))


## [0.47.4](https://github.com/koharu-rs/koharu/compare/0.47.3..0.47.4) - 2026-04-21

### 🐛 Bug Fixes

- Panic when fail to bootstrap - ([5c01ade](https://github.com/koharu-rs/koharu/commit/5c01ade0804664e9e5180620bdccc689f64b696c))


## [0.47.3](https://github.com/koharu-rs/koharu/compare/0.47.2..0.47.3) - 2026-04-21

### 📚 Documentation

- Contributing - ([c6efe47](https://github.com/koharu-rs/koharu/commit/c6efe474dd6d4ddbbeb968c3710873c762914b6a))

### ⚙️ Miscellaneous Tasks

- Panic handler - ([facc424](https://github.com/koharu-rs/koharu/commit/facc424dfdc3c3f82a287ff95413ce0dac78b3b1))
- Remove unnecessary dependencies ([#527](https://github.com/koharu-rs/koharu/issues/527)) - ([3f8d464](https://github.com/koharu-rs/koharu/commit/3f8d464f5e746ce1ea4ca81ed01b8ca39fddb968))


## [0.47.2](https://github.com/koharu-rs/koharu/compare/0.47.1..0.47.2) - 2026-04-20

### ⚡ Performance

- Improve inpainting - ([6f1494a](https://github.com/koharu-rs/koharu/commit/6f1494af238616cac6cfdcdd9941eb702fe8b404))


## [0.47.1](https://github.com/koharu-rs/koharu/compare/0.47.0..0.47.1) - 2026-04-20

### 🐛 Bug Fixes

- Inpainting OOM & pipeline error handling - ([aeda77d](https://github.com/koharu-rs/koharu/commit/aeda77d085ba22679e5361a3368d982ab87661f6))


## [0.47.0](https://github.com/koharu-rs/koharu/compare/0.46.0..0.47.0) - 2026-04-20

### ⛰️  Features

- Keybind ui tweaks, redo/undo configuration ([#517](https://github.com/koharu-rs/koharu/issues/517)) - ([c13a2ee](https://github.com/koharu-rs/koharu/commit/c13a2ee3bfbe1b6909f254a3a3f1016de03e8a30))

### 🐛 Bug Fixes

- *(i18n)* Show language names in their native locale ([#518](https://github.com/koharu-rs/koharu/issues/518)) - ([211f658](https://github.com/koharu-rs/koharu/commit/211f65826b2eaac94cb257afb6e51a92b5d52e66))
- Brush tool where it doesn't render every stroke after the first - ([c72979c](https://github.com/koharu-rs/koharu/commit/c72979cb97fe489ba2e11f2ef71e8438bf981caf))
- Download stuck at 100% sometimes - ([9ef226a](https://github.com/koharu-rs/koharu/commit/9ef226ac76b4f17545adac30fd8334382f8bf300))


## [0.46.0](https://github.com/koharu-rs/koharu/compare/0.45.3..0.46.0) - 2026-04-20

### ⛰️  Features

- Add Belarusian translation option ([#512](https://github.com/koharu-rs/koharu/issues/512)) - ([3b0c9a3](https://github.com/koharu-rs/koharu/commit/3b0c9a31a865a7fb366767f89fe00f060d9cf116))

### 🐛 Bug Fixes

- Render panel throw an error when no page selected - ([ea455b7](https://github.com/koharu-rs/koharu/commit/ea455b746337827f4f1f2667de36ab9f65903598))
- Improve llm dropdown list - ([fff194d](https://github.com/koharu-rs/koharu/commit/fff194df134acd43f8d9573d82fc2688d9946906))
- Bold/italic cause postcard unable reopen - ([c62b050](https://github.com/koharu-rs/koharu/commit/c62b050d802a188f8a974acb8b9f37bd637b7139))
- Global color override per-block color - ([5156d51](https://github.com/koharu-rs/koharu/commit/5156d51f9d2855535bd9250d801f6c6193bd69b7))
- Auto render not pass default font - ([df61113](https://github.com/koharu-rs/koharu/commit/df611134105ea91b905b354d6ee6fe6c59c7ede8))
- Pipeline stop when no text to render - ([25a5de9](https://github.com/koharu-rs/koharu/commit/25a5de95e303a0271fc20a3ab5d92095a380a41b))
- AssetNotFound("index.html") - ([7c711d5](https://github.com/koharu-rs/koharu/commit/7c711d5b6a0605a54ce619d63b686bd8f211ccf2))

### ⚙️ Miscellaneous Tasks

- *(sentry)* Enable session tracking - ([e677200](https://github.com/koharu-rs/koharu/commit/e677200a3c2ccd53982b35e76e21c70158bcbc5d))
- Configure sentry sample rate to 0.1 - ([a2bba3a](https://github.com/koharu-rs/koharu/commit/a2bba3aeaa12cb668356ca2035937aef7dc6331b))


## [0.45.3](https://github.com/koharu-rs/koharu/compare/0.45.2..0.45.3) - 2026-04-19

### 🐛 Bug Fixes

- Normalizes invalid pipeline engine names back to the stage defaults - ([890fb2b](https://github.com/koharu-rs/koharu/commit/890fb2b3b934c58bdafa05aec1d3d66007aaff20))
- Text overlapping - ([39a872c](https://github.com/koharu-rs/koharu/commit/39a872cb0399f604d618c7f7b17f322b16e2c1f3))


## [0.45.2](https://github.com/koharu-rs/koharu/compare/0.45.1..0.45.2) - 2026-04-19

### 🐛 Bug Fixes

- Add bootstrap step - ([e70fc65](https://github.com/koharu-rs/koharu/commit/e70fc656f31c95d93d221282b2f1cfa486045c19))


## [0.45.1](https://github.com/koharu-rs/koharu/compare/0.45.0..0.45.1) - 2026-04-19

### 🐛 Bug Fixes

- Broken export - ([08d32e5](https://github.com/koharu-rs/koharu/commit/08d32e511c6543269c389ea1ae95c651933abded))


## [0.45.0](https://github.com/koharu-rs/koharu/compare/0.44.6..0.45.0) - 2026-04-19

### ⛰️  Features

- *(app)* CLI pipeline binary for full-image regression testing - ([8413df6](https://github.com/koharu-rs/koharu/commit/8413df69578b00e186230f0e5b73bcf36478ba75))
- *(app)* Pipeline engine system with inventory-registered engines - ([05dffa1](https://github.com/koharu-rs/koharu/commit/05dffa180dcd8c8908e641f315b88eb5dad19271))
- *(ui)* Shadcn primitive additions + polish + locale updates - ([7ba57cf](https://github.com/koharu-rs/koharu/commit/7ba57cf72afcda4fbd3f1c4f1c5043811cfcdb48))
- Improve text rendering ux - ([f8d096c](https://github.com/koharu-rs/koharu/commit/f8d096c42fb9a9cc8d4b4f10edd4753525d734d8))
- Multi-selection on ui - ([ef295f0](https://github.com/koharu-rs/koharu/commit/ef295f0b7b3f8a433122d75cce11152bb08ecef5))

### 🐛 Bug Fixes

- Arabic and mixed text rendering ([#500](https://github.com/koharu-rs/koharu/issues/500)) - ([cb1ce23](https://github.com/koharu-rs/koharu/commit/cb1ce234fdab0bfcbe01ed4db381532bfbcc7619))
- Make clippy happy - ([c4ef119](https://github.com/koharu-rs/koharu/commit/c4ef1198c55a24d5f0cb092856f4a72d86291b66))

### 🚜 Refactor

- *(app)* Replace monolithic engine/io with session + history + blobs - ([8eb4a95](https://github.com/koharu-rs/koharu/commit/8eb4a9579fd47873a9404c8286e25813ed076cb9))
- *(core)* Rewrite data model as Scene + Op + content-addressed blobs - ([0cf9ac6](https://github.com/koharu-rs/koharu/commit/0cf9ac6ec3993b61660a69a2cdf54bd3697dfe54))
- *(ml)* Shared TextRegion/TextDirection types for pipeline engines - ([3e4b068](https://github.com/koharu-rs/koharu/commit/3e4b068e83dbc12cc7cc4fb8253abf448db7d45c))
- *(psd)* Rewrite export around new Scene/Page/Node model - ([24155b7](https://github.com/koharu-rs/koharu/commit/24155b744aa44a547055a870a6a8b8a9907b5aa8))
- *(renderer)* Bubble-aware latin layout with BubbleIndex lookup - ([6e68231](https://github.com/koharu-rs/koharu/commit/6e682313f9f86b6ee82a2c8f68beedbfa62e5c85))
- *(rpc)* Axum router with typed routes, SSE events, binary I/O - ([092fee3](https://github.com/koharu-rs/koharu/commit/092fee3a770ded30a5b21be03e7b103318ff0ade))
- *(runtime)* Tauri shell + downloads queue for the v2 backend - ([436885a](https://github.com/koharu-rs/koharu/commit/436885ac584848c9a196049f2df8a5c70bb79cff))
- *(ui)* Menu, settings, welcome, navigator + error/loading chrome - ([7731a38](https://github.com/koharu-rs/koharu/commit/7731a3890c1e4534ac4b77652f6f45aad9f976a1))
- *(ui)* Rewrite editor canvas + right-panel components - ([97f6cb3](https://github.com/koharu-rs/koharu/commit/97f6cb36f4192de4096f048fee1e6d7022b7316b))
- *(ui)* Rewrite hooks around the new scene snapshot + selection store - ([86effa9](https://github.com/koharu-rs/koharu/commit/86effa9aa246b4687723d23bcaa3d1eaef9af425))
- *(ui)* Backend-as-truth state layer (ops / io / stores / events) - ([e7f70a8](https://github.com/koharu-rs/koharu/commit/e7f70a83e526b56002e8190c439832b7d8fb4f42))
- *(ui)* Regenerate orval client against the v2 axum router - ([d2c7ebc](https://github.com/koharu-rs/koharu/commit/d2c7ebc56c0282cf7b5475602e82d4a6302af535))
- Remove cudnn - ([720c8aa](https://github.com/koharu-rs/koharu/commit/720c8aab137759d77d78bb254d760061089e264e))
- Remove e2e - ([fbd739b](https://github.com/koharu-rs/koharu/commit/fbd739b435787596c496da95a65bf596b4ba44d0))
- Remove bubbleDetector - ([c6b28a0](https://github.com/koharu-rs/koharu/commit/c6b28a024a68b481ada57db57cddbe416de6d591))

### 📚 Documentation

- Update demo images - ([5bc33d4](https://github.com/koharu-rs/koharu/commit/5bc33d4d50cffe0e838568c2601c1b2450246b85))
- Update demo images - ([f703f2f](https://github.com/koharu-rs/koharu/commit/f703f2f5fd3ae31c241bb8b724880c8be90005ce))

### 🧪 Testing

- Ignore provider_secret_set_and_clear - ([00b327a](https://github.com/koharu-rs/koharu/commit/00b327af6ff611b1f63861cfd624ffced2bdbf79))
- Fix unit test - ([4ef5c03](https://github.com/koharu-rs/koharu/commit/4ef5c03cbd560cdcec9b918ecd5a8b77cdbaca46))
- Rust integration tests + Vitest UI tests - ([14a73e8](https://github.com/koharu-rs/koharu/commit/14a73e8fb99fc4a0eab13b8f113bf5c52b078699))

### ⚙️ Miscellaneous Tasks

- *(fix)* Add missing client - ([48009ab](https://github.com/koharu-rs/koharu/commit/48009ab2221aabc858a6a85a8561efc862942539))
- Add i18n keys - ([ed3ec95](https://github.com/koharu-rs/koharu/commit/ed3ec95334bf43fa0b22d6b2885d1df4747c8f0c))
- Run Rust integration tests and Vitest UI tests - ([fbf208d](https://github.com/koharu-rs/koharu/commit/fbf208db2da40984096acf0b406d18f8a83dbd5e))
- Workspace config + next.js + tauri build glue for v2 - ([4c1f5c5](https://github.com/koharu-rs/koharu/commit/4c1f5c560baaed6e094d3e7926ee7c772fc12f61))


## [0.44.6](https://github.com/koharu-rs/koharu/compare/0.44.5..0.44.6) - 2026-04-16

### ⛰️  Features

- Add Caiyun translation provider ([#478](https://github.com/koharu-rs/koharu/issues/478)) - ([f7fca6d](https://github.com/koharu-rs/koharu/commit/f7fca6dfbe93125a800db7bb178d48e50c6e4207))

### 🐛 Bug Fixes

- RTL Support and Text Alignment Overhaul ([#475](https://github.com/koharu-rs/koharu/issues/475)) - ([5411daf](https://github.com/koharu-rs/koharu/commit/5411daf27b4e5600fd4ab4de314c3763cbd69b96))
- Default font not being applied in processing ([#456](https://github.com/koharu-rs/koharu/issues/456)) - ([d0d1a56](https://github.com/koharu-rs/koharu/commit/d0d1a5688ba6d7ca70799cc86bc3865993f7b529))
- Resolve black ([#000000](https://github.com/koharu-rs/koharu/issues/000000)) text color not applying in render controls ([#482](https://github.com/koharu-rs/koharu/issues/482)) - ([7748937](https://github.com/koharu-rs/koharu/commit/77489374ba96d92754ee4ceb430f1c8e5e68e872))
- Incorrect sorting by image name ([#455](https://github.com/koharu-rs/koharu/issues/455)) - ([3bdf850](https://github.com/koharu-rs/koharu/commit/3bdf8506c67e15d51940573fbc42824fe926640a))

### 📚 Documentation

- Add Portuguese (pt-BR) docs and assets ([#476](https://github.com/koharu-rs/koharu/issues/476)) - ([15dba78](https://github.com/koharu-rs/koharu/commit/15dba783725b5d50fe2d917a4f5df1e598a6a516))

### ⚙️ Miscellaneous Tasks

- Build pt-BR docs and fix Portuguese wording ([#481](https://github.com/koharu-rs/koharu/issues/481)) - ([5edb16e](https://github.com/koharu-rs/koharu/commit/5edb16e1d91ad2fc466e0f69ce22cd0c96aefe87))
- Replace prettier with oxfmt and apply formatting ([#472](https://github.com/koharu-rs/koharu/issues/472)) - ([19be246](https://github.com/koharu-rs/koharu/commit/19be246108f8c479ebde8f21c706bc83cd859ab7))


## [0.44.5](https://github.com/koharu-rs/koharu/compare/0.44.4..0.44.5) - 2026-04-15

### ⛰️  Features

- Portable mode ([#415](https://github.com/koharu-rs/koharu/issues/415)) - ([332f55b](https://github.com/koharu-rs/koharu/commit/332f55b78d7a941c52c2e424b9ac1f4b58a52c76))

### 📚 Documentation

- Runtime & model downloads - ([0a913e9](https://github.com/koharu-rs/koharu/commit/0a913e99ed32e3a585e0511d66b2aaee852811bf))


## [0.44.4](https://github.com/koharu-rs/koharu/compare/0.44.3..0.44.4) - 2026-04-15

### 🐛 Bug Fixes

- Resolve incorrect ZLUDA DLLs usage ([#442](https://github.com/koharu-rs/koharu/issues/442)) ([#458](https://github.com/koharu-rs/koharu/issues/458)) - ([63bf6c1](https://github.com/koharu-rs/koharu/commit/63bf6c1c6c5844ac4bfc2e01a24546a870abd1b0))


## [0.44.3](https://github.com/koharu-rs/koharu/compare/0.44.2..0.44.3) - 2026-04-15

### 🐛 Bug Fixes

- Load correct ZLUDA DLLs - ([b3385d0](https://github.com/koharu-rs/koharu/commit/b3385d031fc8c15e02d36bf53aae37b0a8b92bc6))

### 📚 Documentation

- Improve README formatting and add resource links - ([1c8ad74](https://github.com/koharu-rs/koharu/commit/1c8ad742592b0d078c719acf4136f1d70c51ef5d))


## [0.44.2](https://github.com/koharu-rs/koharu/compare/0.44.1..0.44.2) - 2026-04-14

### 🐛 Bug Fixes

- Upgrade zluda to v6-preview.64 - ([23d97e5](https://github.com/koharu-rs/koharu/commit/23d97e552fb9d460ceb0d772c7823518efb36eef))


## [0.44.1](https://github.com/koharu-rs/koharu/compare/0.44.0..0.44.1) - 2026-04-14

### 🐛 Bug Fixes

- Sentry setup - ([7fa180b](https://github.com/koharu-rs/koharu/commit/7fa180b2ae343e625097602a8710afd97c394170))

### ⚙️ Miscellaneous Tasks

- Add oxc linting to CI and fix warnings ([#435](https://github.com/koharu-rs/koharu/issues/435)) - ([fbe8535](https://github.com/koharu-rs/koharu/commit/fbe853584b3279b8a94b18a5ab3b1d5c89019d9e))
- Add confirmation message to Portuguese translations ([#430](https://github.com/koharu-rs/koharu/issues/430)) - ([07963ad](https://github.com/koharu-rs/koharu/commit/07963ad23145378efe21ecebf39d045be683804b))
- Fix welcome workflow - ([94ae12b](https://github.com/koharu-rs/koharu/commit/94ae12b9ac46f002bbfca1a09ea6a0ea397fc3a9))
- Add OXLint for linting the UI code ([#432](https://github.com/koharu-rs/koharu/issues/432)) - ([9912409](https://github.com/koharu-rs/koharu/commit/9912409f080469b05eba02413f13897396531d70))
- Add Docker deployment ([#401](https://github.com/koharu-rs/koharu/issues/401)) - ([ff9f95c](https://github.com/koharu-rs/koharu/commit/ff9f95caae675a6f1b3b7734fecf974b22fd7ba2))
- Add missing SENTRY_AUTH_TOKEN - ([82e8c46](https://github.com/koharu-rs/koharu/commit/82e8c461162847d10771894de49e172cdce24702))


## [0.44.0](https://github.com/koharu-rs/koharu/compare/0.43.2..0.44.0) - 2026-04-13

### ⛰️  Features

- *(mt)* DeepL/Google as koharu-llm providers ([#357](https://github.com/koharu-rs/koharu/issues/357)) - ([5bf565f](https://github.com/koharu-rs/koharu/commit/5bf565f98a222f40cdbea4705d76e9cd3d462a31))
- *(ui)* Add model search to llm popover ([#409](https://github.com/koharu-rs/koharu/issues/409)) - ([654583f](https://github.com/koharu-rs/koharu/commit/654583f77b3479df1e4465df5ecfe5b1537407ea))
- Add keybind configuration in settings ([#411](https://github.com/koharu-rs/koharu/issues/411)) - ([bb471fd](https://github.com/koharu-rs/koharu/commit/bb471fdb87c5dc2b295f579949e768c21376f7cc))
- Tool keybind and Brush Outline on canvas implementation ([#396](https://github.com/koharu-rs/koharu/issues/396)) - ([05ccbc8](https://github.com/koharu-rs/koharu/commit/05ccbc8b8c2abdb6aa44320ae78a47f487bff227))
- Updater - ([a61bb75](https://github.com/koharu-rs/koharu/commit/a61bb756f4d373530971cf3fa68627b46d4390bc))
- Sentry integration - ([1e3462a](https://github.com/koharu-rs/koharu/commit/1e3462aaeadefa9122ab42d0bc59b147557daf8a))
- Remove segmentation mask detections outside of detected text blocks ([#378](https://github.com/koharu-rs/koharu/issues/378)) - ([b51c007](https://github.com/koharu-rs/koharu/commit/b51c0077d3b39ac3aee74f4e1c0f24120f4ec116))
- Add AMD GPU support via ZLUDA ([#362](https://github.com/koharu-rs/koharu/issues/362)) - ([759985b](https://github.com/koharu-rs/koharu/commit/759985b74bd240c8d92fcc60d9e843a9b89b398e))
- Implementing --host cli parameter ([#328](https://github.com/koharu-rs/koharu/issues/328)) - ([8238a7c](https://github.com/koharu-rs/koharu/commit/8238a7cd19223ea7e1614811aff785474ef75011))

### 🐛 Bug Fixes

- *(renderer)* Add thai font fallback ([#370](https://github.com/koharu-rs/koharu/issues/370)) - ([d0a14f2](https://github.com/koharu-rs/koharu/commit/d0a14f29641cdb3176ecdfd2771ad4c8102fc97a))
- No current document selected ([#394](https://github.com/koharu-rs/koharu/issues/394)) - ([c86578e](https://github.com/koharu-rs/koharu/commit/c86578e26abec22c9c6b28c2f88a673d18beaed0))
- Linking and updater - ([8d70da2](https://github.com/koharu-rs/koharu/commit/8d70da286e36665888599184d5a434e991c9174d))
- Add `get_document_id_for_index` mcp tool ([#377](https://github.com/koharu-rs/koharu/issues/377)) - ([d264853](https://github.com/koharu-rs/koharu/commit/d264853bc525ef25b6ea8c4ba08a3a67a8ca52f0))
- Allow Segmentation Mask to be viewed outside of Eraser and Repair Brush Tools ([#358](https://github.com/koharu-rs/koharu/issues/358)) - ([41c50d4](https://github.com/koharu-rs/koharu/commit/41c50d4e525226f5da77fc3850b92b99609f394a))

### 🚜 Refactor

- *(llm)* Deel & google translate ([#361](https://github.com/koharu-rs/koharu/issues/361)) - ([09c43a0](https://github.com/koharu-rs/koharu/commit/09c43a0ab58c432fff92f60003f483259429d435))

### 📚 Documentation

- Correct download button label ([#418](https://github.com/koharu-rs/koharu/issues/418)) - ([557662a](https://github.com/koharu-rs/koharu/commit/557662ac431f737107d5101d3a592d58964143a7))
- Update text rendering to include RTL - ([63fd98c](https://github.com/koharu-rs/koharu/commit/63fd98c2ef74465631deec212e09ff16792c0794))
- Update data privacy note - ([a78fdd6](https://github.com/koharu-rs/koharu/commit/a78fdd676c2fb36fcc39ac1a727b9697d8aef86a))
- Update developement section - ([a3c01a2](https://github.com/koharu-rs/koharu/commit/a3c01a2e7ab84b89d1bd0502f25251cda4f60299))
- Remove cudnn - ([caf9cec](https://github.com/koharu-rs/koharu/commit/caf9cecb263da333f294c361331d7249e556c017))
- Include zluda - ([b72338f](https://github.com/koharu-rs/koharu/commit/b72338f64e315187d5ab9b6aaa385ebe2ebba8da))
- Add technical term links - ([b886699](https://github.com/koharu-rs/koharu/commit/b8866999b55abb2e992e4be15b56b36af5955276))
- Mention text rendering - ([422c653](https://github.com/koharu-rs/koharu/commit/422c6536568088bbbb65bc6d8dd67e1fc569f122))
- Improve readme - ([2300986](https://github.com/koharu-rs/koharu/commit/2300986e100f0e03aa432bbe2c19be48499cbcc2))

### ⚙️ Miscellaneous Tasks

- *(ci)* Ignore scripts and generated - ([2bb81ae](https://github.com/koharu-rs/koharu/commit/2bb81aeba59b9192061cd3203b1d40dcb099ddd7))
- Enhance PR workflow with issue support and messages - ([e286f58](https://github.com/koharu-rs/koharu/commit/e286f5858abd88ccfdc22236981ac5def525bfcf))
- Simplify name - ([472e15c](https://github.com/koharu-rs/koharu/commit/472e15c995696364ba50d3898b2d63193ecad51a))
- Enhance workflow - ([686d710](https://github.com/koharu-rs/koharu/commit/686d7103c7ed0e703141e5713f08149d29c188ce))
- Add workflow to summarize new issues with AI - ([ce9d4c2](https://github.com/koharu-rs/koharu/commit/ce9d4c289ed46b20f108f8cee1bafa182a21dc0c))
- Enable Sentry only for production builds ([#397](https://github.com/koharu-rs/koharu/issues/397)) - ([bc38c49](https://github.com/koharu-rs/koharu/commit/bc38c49206f2256c69bd52831915da568f5fbb65))
- Update CODEOWNERS - ([b6972d2](https://github.com/koharu-rs/koharu/commit/b6972d2becf534f784e9b6565c5bf61fa5a66d95))


## [0.43.2](https://github.com/koharu-rs/koharu/compare/0.43.1..0.43.2) - 2026-04-06

### ⛰️  Features

- Change sort_manga_read_order algorithm for better ordering ([#349](https://github.com/koharu-rs/koharu/issues/349)) - ([3b31ecc](https://github.com/koharu-rs/koharu/commit/3b31eccee898a89eb2879928b25371a48c2d4851))

### 🐛 Bug Fixes

- *(ui)* Keep process-all semantics for single-image batches - ([5f99fbf](https://github.com/koharu-rs/koharu/commit/5f99fbf071b89987c12c8ee36a0c520a44c10fa6))
- Store app-level global font - ([a2eeb1c](https://github.com/koharu-rs/koharu/commit/a2eeb1c6628001d85ec7b2b75805c1f46b78e5ca))
- Cl path error on dev script, ([#348](https://github.com/koharu-rs/koharu/issues/348)) - ([d0885a8](https://github.com/koharu-rs/koharu/commit/d0885a8fbd36a60924feba55004e6a5085dedd4d))
- Preserve font inheritance when editing defaults - ([215a5b6](https://github.com/koharu-rs/koharu/commit/215a5b6e74c70519f1d145b32605f51232503e86))
- Persist font selection to document default when block is selected - ([434d27f](https://github.com/koharu-rs/koharu/commit/434d27fb691574f00b0f79f559ffa899d69e3fb9))
- Apply selected global font - ([142fb63](https://github.com/koharu-rs/koharu/commit/142fb6360749de4a2e15d02661af344cbb42b6b1))
- Batch procssing pipeline - ([537f088](https://github.com/koharu-rs/koharu/commit/537f088bba316f7f4006cd7d099c7a0e69328bd0))
- Enhance pipeline processing - ([d8e3bce](https://github.com/koharu-rs/koharu/commit/d8e3bced01de2e560309a41c9c54a02d95aa58d5))


## [0.43.1](https://github.com/koharu-rs/koharu/compare/0.43.0..0.43.1) - 2026-04-05

### 🐛 Bug Fixes

- Qwen3.5 template - ([eecfafe](https://github.com/koharu-rs/koharu/commit/eecfafe6a4f0aa1d87002e57a9f939c675dbcf63))

### 📚 Documentation

- List more LLMs - ([37162b9](https://github.com/koharu-rs/koharu/commit/37162b9565bde61940562a998b3699310e0d789f))

### ⚙️ Miscellaneous Tasks

- Fix tests - ([d530cbe](https://github.com/koharu-rs/koharu/commit/d530cbe1d530388c22d8b980edcc83db79e3ac63))


## [0.43.0](https://github.com/koharu-rs/koharu/compare/0.42.4..0.43.0) - 2026-04-05

### ⛰️  Features

- Custom system prompt - ([d3e6b06](https://github.com/koharu-rs/koharu/commit/d3e6b0650c24552367e073fe9e446bd8c7acdea2))
- Add uncensored LLMs - ([7814d1e](https://github.com/koharu-rs/koharu/commit/7814d1efad9d66a801521ef693d3136553f38be4))
- Add gemma4 and qwen3.5 models - ([e6262e4](https://github.com/koharu-rs/koharu/commit/e6262e49e92ce7b7cac853df9d31835faa3b9a98))
- Replce lfm2-350m-enjp-mt with lfm2.5-1.2b-instruct - ([4d2e5ef](https://github.com/koharu-rs/koharu/commit/4d2e5ef42905fa4aa0c8f7294f3fc58c1e002272))
- Update llama.cpp to b8665 - ([cd86637](https://github.com/koharu-rs/koharu/commit/cd866371a074cfb374cd5f6a5327e98f6c2ccb82))
- Google fonts - ([177767f](https://github.com/koharu-rs/koharu/commit/177767f1132208a2c909f5446f5fcfa01dec6ef9))

### 🐛 Bug Fixes

- *(ui)* Invalidate stale font style - ([d12cec0](https://github.com/koharu-rs/koharu/commit/d12cec0c299d6e4295691c55f39598d173bf9ead))
- Scroll on canvas - ([bfc0aef](https://github.com/koharu-rs/koharu/commit/bfc0aefa9f20118608b183df070e72ebb39810d7))

### ⚙️ Miscellaneous Tasks

- *(ui)* Rename section name - ([d6bf317](https://github.com/koharu-rs/koharu/commit/d6bf31729d8acea7f8c2691bee639b89a9e2f5ce))
- Format code - ([04b9034](https://github.com/koharu-rs/koharu/commit/04b90343d5c564729f41eca1bb2ef9b1bcb2c2c1))
- Format code - ([65bb821](https://github.com/koharu-rs/koharu/commit/65bb821e1a79aafca46a288ac1b51bccc43ef90c))


## [0.42.4](https://github.com/koharu-rs/koharu/compare/0.42.3..0.42.4) - 2026-04-05

### 🐛 Bug Fixes

- Http timeouts - ([676495f](https://github.com/koharu-rs/koharu/commit/676495f96179907b162a57f6453808e7f2c97ab7))


## [0.42.3](https://github.com/koharu-rs/koharu/compare/0.42.2..0.42.3) - 2026-04-05

### 🐛 Bug Fixes

- Predict stroke color - ([53617b6](https://github.com/koharu-rs/koharu/commit/53617b62c5d20f11011974f932c4d1a433a02199))
- Pin libz-sys to 1.1.25 - ([cae9542](https://github.com/koharu-rs/koharu/commit/cae9542a3aaf49a0b3c9e0da917b4e70e7308102))

### 📚 Documentation

- Rephrase - ([7d27635](https://github.com/koharu-rs/koharu/commit/7d27635b6aa75950c691922ea9d6c81f77dcfcc8))

### 🧪 Testing

- Remove outdated - ([31f13dc](https://github.com/koharu-rs/koharu/commit/31f13dcf0ac1d37aa4eb76e6289c0f71a2781372))

### ⚙️ Miscellaneous Tasks

- Remove ts-rs env - ([ef5cb84](https://github.com/koharu-rs/koharu/commit/ef5cb84812aa687d4070a84e4fd75d1b48a46aab))
- Disable github changelog - ([3847dcd](https://github.com/koharu-rs/koharu/commit/3847dcdde45a699b119a827ef9f016693b2447cf))


## [0.42.2](https://github.com/koharu-rs/koharu/compare/0.42.1..0.42.2) - 2026-04-04

### 🐛 Bug Fixes

- Cuda startup failure - ([766662b](https://github.com/koharu-rs/koharu/commit/766662beaa293e54e98bdf6c35c5447830a25525))

### 📚 Documentation

- Reflect new models - ([873236b](https://github.com/koharu-rs/koharu/commit/873236b7a0225475e7c9a43c349dfa2300163142))


## [0.42.1](https://github.com/koharu-rs/koharu/compare/0.42.0..0.42.1) - 2026-04-04

### 🐛 Bug Fixes

- Tauri remote url - ([770a477](https://github.com/koharu-rs/koharu/commit/770a477213d15bcc81cb6cc59fdb3c2b93482216))


## [0.42.0](https://github.com/koharu-rs/koharu/compare/0.41.4..0.42.0) - 2026-04-04

### ⛰️  Features

- *(ui)* Settings dialog, i18n, and initialization screen - ([290904d](https://github.com/koharu-rs/koharu/commit/290904db966c5fe293d972353a17803c5cad8015))
- Sort manga reading order - ([18f2b42](https://github.com/koharu-rs/koharu/commit/18f2b42ebaa50fb23655bc116fae11ff40999f2c))
- Blob API, bubble/AOT-inpainting models, and rendering redesign - ([004a94a](https://github.com/koharu-rs/koharu/commit/004a94a005c3c3a0ffaa14dd411af578ab2237be))
- Custom tracing layer and log suppression - ([aa91dbd](https://github.com/koharu-rs/koharu/commit/aa91dbd7e36268b09224c810ce6b23fd34652eab))
- Pluggable engine system with DAG pipeline - ([c84cfad](https://github.com/koharu-rs/koharu/commit/c84cfad0751e43bf980179d856a9146885bbb971))
- Add Portuguese (pt-BR) translations ([#317](https://github.com/koharu-rs/koharu/issues/317)) - ([ed6e4ac](https://github.com/koharu-rs/koharu/commit/ed6e4ac157687af3d6810bcbfd4d1f2e2f3420fe))
- Disabling keyring using --no-keyring ([#308](https://github.com/koharu-rs/koharu/issues/308)) - ([f470146](https://github.com/koharu-rs/koharu/commit/f470146e1b017f225e9ef797df942ed5c5a27f81))
- Add Korean (ko-KR) locale and translate ([#316](https://github.com/koharu-rs/koharu/issues/316)) - ([1d1b3df](https://github.com/koharu-rs/koharu/commit/1d1b3df09d49874584aa8e8cb42f0d2796c918b1))
- Add Turkish UI localization ([#315](https://github.com/koharu-rs/koharu/issues/315)) - ([4a99909](https://github.com/koharu-rs/koharu/commit/4a99909118c103dde880176c7edb4324a53d0b98))

### 🚜 Refactor

- *(ui)* Centralize error handling with XState - ([e29b31e](https://github.com/koharu-rs/koharu/commit/e29b31ebdf6bcdc760f60df889cccfbfe8681b95))
- Simplify Tauri setup and dev server - ([f301793](https://github.com/koharu-rs/koharu/commit/f3017931daee53999bbf878038e6260d6599b7a5))
- Rewrite backend storage, config, and runtime - ([ed48490](https://github.com/koharu-rs/koharu/commit/ed48490d715a13039fabf839573029adc119f2fe))
- OpenAPI codegen with utoipa and Orval - ([1502ac0](https://github.com/koharu-rs/koharu/commit/1502ac04d2b53b85a9aaae6067a3c5917a3d6d8f))

### ⚡ Performance

- Improve ctd - ([ae85e1a](https://github.com/koharu-rs/koharu/commit/ae85e1ac57b12e04c69b005af943a2faeee9c44b))

### ⚙️ Miscellaneous Tasks

- Remove macos warnings - ([58e5391](https://github.com/koharu-rs/koharu/commit/58e539104d4c60621f549f31fec2b5f944e96199))


## [0.41.4](https://github.com/koharu-rs/koharu/compare/0.41.3..0.41.4) - 2026-03-30

### ⚙️ Miscellaneous Tasks

- Use explicit tauri cli package in release - ([64cadbc](https://github.com/koharu-rs/koharu/commit/64cadbc4fcdcb16879df4591de07676ee08032e9))


## [0.41.3](https://github.com/koharu-rs/koharu/compare/0.41.2..0.41.3) - 2026-03-30

### ⚙️ Miscellaneous Tasks

- Use npx tauri for release builds - ([ea2a80b](https://github.com/koharu-rs/koharu/commit/ea2a80b688f4aad148be55ae78d6650c995a6b69))


## [0.41.2](https://github.com/koharu-rs/koharu/compare/0.41.1..0.41.2) - 2026-03-30

### ⚙️ Miscellaneous Tasks

- Download trusted-signing-cli from release - ([b686601](https://github.com/koharu-rs/koharu/commit/b6866015cbe8a695cb4f6415e35720afec31b644))


## [0.41.1](https://github.com/koharu-rs/koharu/compare/0.41.0..0.41.1) - 2026-03-30

### 🐛 Bug Fixes

- *(macos)* Allow downloaded llama runtimes to load - ([bb3e7d3](https://github.com/koharu-rs/koharu/commit/bb3e7d37680f1c5b2aa66078f19cb910eb3eeae3))


## [0.41.0](https://github.com/koharu-rs/koharu/compare/0.40.1..0.41.0) - 2026-03-30

### ⛰️  Features

- *(ui)* LLM multi-preset configuration ([#282](https://github.com/koharu-rs/koharu/issues/282)) - ([bcd9df1](https://github.com/koharu-rs/koharu/commit/bcd9df15a58bda1e19f9cb0c6742294dc8e7d2d0))
- Bootstrap window - ([288fcc7](https://github.com/koharu-rs/koharu/commit/288fcc7022c31bf8e79539357dbc04033fdc78f2))
- Llama.cpp-based paddleocr-vl-1.5 - ([5c4b7c1](https://github.com/koharu-rs/koharu/commit/5c4b7c1cabae636f470a9d5a406a6c639a1aeb31))
- Integrate llama.cpp - ([30db042](https://github.com/koharu-rs/koharu/commit/30db0424b70f21695aedfacb9a71bb3f1b2e794a))
- Llama.cpp crate - ([0465ff7](https://github.com/koharu-rs/koharu/commit/0465ff7eae8d4e47fdb3855ea523e170196290be))
- Add open folder import, CUDA 13.2 build fixes, local LLM settin… ([#271](https://github.com/koharu-rs/koharu/issues/271)) - ([83615e7](https://github.com/koharu-rs/koharu/commit/83615e7bbf52d68f38faf931c00b579cf58ccfb8))

### 🐛 Bug Fixes

- *(linux)* Load native libraryes by using explicit path to resolve missing libggml errors on Unix systems ([#295](https://github.com/koharu-rs/koharu/issues/295)) - ([5b0a3c1](https://github.com/koharu-rs/koharu/commit/5b0a3c1a7d32c033b83173fe4448cee470d0ac42))
- Pipeline crash when no text blocks detected ([#291](https://github.com/koharu-rs/koharu/issues/291)) - ([82454e0](https://github.com/koharu-rs/koharu/commit/82454e03722358d1d50c4c9309c999d40dfb482d))
- Prevent koharu-llm from recompiling on every build ([#289](https://github.com/koharu-rs/koharu/issues/289)) - ([1ccf02c](https://github.com/koharu-rs/koharu/commit/1ccf02c767a00753f39db5f4e38ca06fb65934ee))
- Avoid full document cloning in API endpoints to speed up image switching ([#279](https://github.com/koharu-rs/koharu/issues/279)) - ([46d47f8](https://github.com/koharu-rs/koharu/commit/46d47f86f0f2f7e01216da6e37945001dc29b40c))
- Nvcc build failure on linux - ([8808b12](https://github.com/koharu-rs/koharu/commit/8808b12492dc73e257819f0cb6245f12a88e7ead))
- Revert cudaforge - ([1e0bac8](https://github.com/koharu-rs/koharu/commit/1e0bac816e45a0883f9656d1c4f167d349cfb4a4))
- Pin candle commit - ([0eae785](https://github.com/koharu-rs/koharu/commit/0eae7857b789e3d54b8d501cbb8f24c820b0bf6c))
- Cast log level to i32 for previous level storage ([#273](https://github.com/koharu-rs/koharu/issues/273)) - ([b22ade6](https://github.com/koharu-rs/koharu/commit/b22ade66e45fb87f961b99fd1519ee07b286227f))
- Shared llama.cpp backend - ([f8db7d9](https://github.com/koharu-rs/koharu/commit/f8db7d9053979eb0dcbf506bb13b278e32b31505))
- Ui issue and automatic code formatting :v ([#272](https://github.com/koharu-rs/koharu/issues/272)) - ([08e4170](https://github.com/koharu-rs/koharu/commit/08e4170f7fc622c98c3ba477897ae4b1666003e1))
- Align paddleocr-vl impl with transformers - ([879c8da](https://github.com/koharu-rs/koharu/commit/879c8da4b622159746608e7ffa08ae236d461467))
- Remove normalize_ocr_text - ([ca64093](https://github.com/koharu-rs/koharu/commit/ca640939e6baf7322dafb49a67334cf0bbed5d43))

### 🚜 Refactor

- Merge crates - ([98f90f7](https://github.com/koharu-rs/koharu/commit/98f90f7412bec282d65f1b840a425200e74e82f8))
- Koharu-runtime - ([e78b2d2](https://github.com/koharu-rs/koharu/commit/e78b2d24060501c9a4f09246f7b639006f3c6348))

### 📚 Documentation

- Fix broken link - ([b1a4565](https://github.com/koharu-rs/koharu/commit/b1a45659c2065018d3471fedb067a9f42152f051))
- Fix readme img link - ([2e66e8d](https://github.com/koharu-rs/koharu/commit/2e66e8d4ee018ac5cea970ca911de61d418fa41a))
- Ja-JP translation - ([daa4b10](https://github.com/koharu-rs/koharu/commit/daa4b10bf064519ee6a842b3132fddb37cccb0a2))
- Zh-CN translations - ([90e6a29](https://github.com/koharu-rs/koharu/commit/90e6a29cadbf583b30f4b331928744b2a4fd34c4))
- Text rendering - ([3780b1c](https://github.com/koharu-rs/koharu/commit/3780b1c3e4a1197c4783e11aad94ec018c21ed54))
- Open graph - ([c013790](https://github.com/koharu-rs/koharu/commit/c013790178ff10c47ecb5b775791201413d3f5e6))
- More content - ([13e07b2](https://github.com/koharu-rs/koharu/commit/13e07b21fb0a486b25c9910372e88b74d0933abd))
- Add llama.cpp - ([d607a67](https://github.com/koharu-rs/koharu/commit/d607a6742c2c850561e9e8f118f95841583ab1f7))
- Homepage - ([ebd16c8](https://github.com/koharu-rs/koharu/commit/ebd16c8ff51e5ad9c3fdb099a06c9361f9608150))
- Add site - ([b8f34a0](https://github.com/koharu-rs/koharu/commit/b8f34a043fca3a556400f38e06a3f30777977e7a))
- Update models section - ([f9987ca](https://github.com/koharu-rs/koharu/commit/f9987ca1d157939ccafc174d0c5849909e4fbd21))

### ⚡ Performance

- Fix paddleocr-vl - ([c367543](https://github.com/koharu-rs/koharu/commit/c367543b11ef391b81eb16ef3e684557d9a8f987))

### ⚙️ Miscellaneous Tasks

- *(cargo)* Update deps - ([d73b2ad](https://github.com/koharu-rs/koharu/commit/d73b2ad8082493f3b316b1c46fb1ae81f1a3f617))
- *(dev)* Fix hydration warning - ([042cdb6](https://github.com/koharu-rs/koharu/commit/042cdb6cd892abf13b1b11a5b2413dc374d867c5))
- *(i18n)* Add missing translations - ([ea899a7](https://github.com/koharu-rs/koharu/commit/ea899a7e445a5c16ca6f43b73e190569afde993f))
- Make clippy happy - ([55575a7](https://github.com/koharu-rs/koharu/commit/55575a7f6f82af25d935fa1c8897eeb0810bb114))
- Rename integration tests to unit tests - ([623a55f](https://github.com/koharu-rs/koharu/commit/623a55fe7cd21117047a088fde607d65865eda19))
- Disable ml models integration tests - ([2df4fca](https://github.com/koharu-rs/koharu/commit/2df4fca2a95e34f3c3559e2b0183462652cea3fe))
- Cache bun packages and Next.js build output ([#290](https://github.com/koharu-rs/koharu/issues/290)) - ([b17ead9](https://github.com/koharu-rs/koharu/commit/b17ead9cea8243fd0ae8e8a44717fbaecf8786d5))
- Make clippy happy - ([8af2862](https://github.com/koharu-rs/koharu/commit/8af2862cc17eba9607fdb1894cd84b5fe77a6494))
- Vendor paddleocr-vl - ([8003e9b](https://github.com/koharu-rs/koharu/commit/8003e9bf8a41d327b28e334db989c28b4d69060a))


## [0.40.1](https://github.com/koharu-rs/koharu/compare/0.40.0..0.40.1) - 2026-03-21

### ⚙️ Miscellaneous Tasks

- Add github token - ([1269cdd](https://github.com/koharu-rs/koharu/commit/1269cdde0137428eee3b880165e6d1f5819d17cb))


## [0.40.0](https://github.com/koharu-rs/koharu/compare/0.39.0..0.40.0) - 2026-03-21

### ⛰️  Features

- Wire new models - ([ef54307](https://github.com/koharu-rs/koharu/commit/ef54307643633da38cb2f1ec0c4677907eba06e6))
- Manga-text-segmentation-2025 model - ([2e7209e](https://github.com/koharu-rs/koharu/commit/2e7209eb1fe9480131473a0eecec2e30356507b7))
- Paddleocr-vl-1.5 model - ([1b0f45e](https://github.com/koharu-rs/koharu/commit/1b0f45e76ff45d0b0d1f84647798994e08f72f92))
- Pp-doclayout-v3 model - ([3724003](https://github.com/koharu-rs/koharu/commit/3724003fd031b192a21e8568418bc2c571b8c066))

### 🐛 Bug Fixes

- Refine mask - ([4e4a7ca](https://github.com/koharu-rs/koharu/commit/4e4a7ca98911fdace1845756515f0dc255f9ee59))
- Use f32 for paddleocr-vl - ([0534100](https://github.com/koharu-rs/koharu/commit/05341008fe912ecae8d7d1d51e457705a9c0c601))

### ⚡ Performance

- Speedup detection & ocr - ([2e5eb9c](https://github.com/koharu-rs/koharu/commit/2e5eb9c85386b4b6190078b07364c53c5e3f8ba0))

### ⚙️ Miscellaneous Tasks

- Add refine manga109 script - ([42ddd88](https://github.com/koharu-rs/koharu/commit/42ddd88d4164905511d2a4333d2ee3e0af0d312c))


## [0.39.0](https://github.com/koharu-rs/koharu/compare/0.38.1..0.39.0) - 2026-03-19

### ⛰️  Features

- Add bg-BG lang - ([327120a](https://github.com/koharu-rs/koharu/commit/327120a279f838e3ee22a8614ea96c131c2a8b0a))
- Support more languages - ([9b84b51](https://github.com/koharu-rs/koharu/commit/9b84b5145fd8b01d492c7b1fb0dfc501ba0182a2))
- Export to psd - ([89b9ed2](https://github.com/koharu-rs/koharu/commit/89b9ed2158a37ef95b02300e0903684889b9da83))
- Add delete functionality to text blocks ([#245](https://github.com/koharu-rs/koharu/issues/245)) - ([d28a260](https://github.com/koharu-rs/koharu/commit/d28a2609c8f01fe4943ae20d41174451d1bf0b0c))

### 🐛 Bug Fixes

- Global font not applied - ([17382cf](https://github.com/koharu-rs/koharu/commit/17382cf2394a1d42145bac049d9f874605e3950d))
- Llm list not loaded - ([3446a09](https://github.com/koharu-rs/koharu/commit/3446a0936df098ee2ebcfc2ab23b9a15c50eec90))
- Text box ordering regression - ([c451d84](https://github.com/koharu-rs/koharu/commit/c451d84fe720dee08127db5a08ad2a8c1df8816e))
- Style & ui build - ([a7246b1](https://github.com/koharu-rs/koharu/commit/a7246b160d01916f4da9b63708030aed6c7762de))

### ⚙️ Miscellaneous Tasks

- I18n consistent - ([09e4f25](https://github.com/koharu-rs/koharu/commit/09e4f25d1d56bbc3a9073ef6230e4cf97549d0c1))
- Speed up linux - ([5d71b64](https://github.com/koharu-rs/koharu/commit/5d71b648987fb3b20315a71167a7e5cc729ee5d3))
- Speed up cuda install - ([c59b3d9](https://github.com/koharu-rs/koharu/commit/c59b3d9604020af26e9cae7bee52e310c096a18b))
- Add release notes - ([d5e3101](https://github.com/koharu-rs/koharu/commit/d5e3101ac84036fdc90fac725ce5848751cf2de8))


## [0.38.1](https://github.com/koharu-rs/koharu/compare/0.38.0..0.38.1) - 2026-03-17

### 🐛 Bug Fixes

- Fallback to cpu if nvidia driver doesn't support cuda 13.1 - ([f3a8f74](https://github.com/koharu-rs/koharu/commit/f3a8f743605185cb6d7c4c297fbde681078f7a3f))
- Compare_blocks_for_reading_order panics - ([103b93e](https://github.com/koharu-rs/koharu/commit/103b93e4d3e65623b4d17e1aef5120eab5a843a6))
- Headless on linux - ([1878064](https://github.com/koharu-rs/koharu/commit/18780641a326a504b6991a76030cc8116d9d8eff))

### 🚜 Refactor

- Simplify string serialization - ([dae1647](https://github.com/koharu-rs/koharu/commit/dae164746989af050d197531915551ef3273c3c0))

### ⚙️ Miscellaneous Tasks

- Update deps - ([0eb93c9](https://github.com/koharu-rs/koharu/commit/0eb93c9ef02d6a9fa10b3114a5af2c48e19dbea1))


## [0.38.0](https://github.com/koharu-rs/koharu/compare/0.37.0..0.38.0) - 2026-03-16

### ⛰️  Features

- OpenAi Compatible - ([9570a45](https://github.com/koharu-rs/koharu/commit/9570a4590f7dae26852186dbf49e2656c42ec622))
- Add Deepseek provider ([#235](https://github.com/koharu-rs/koharu/issues/235)) - ([69d9421](https://github.com/koharu-rs/koharu/commit/69d94211ce2a1e5ea3d4074a8ba6b1d0177fd73e))
- Implement API provider support with OpenAI, Gemini, and Claude integration - ([c6a47ce](https://github.com/koharu-rs/koharu/commit/c6a47cee2b7faa3dbd61d44bac4c2c3970fcc7b1))

### 🐛 Bug Fixes

- Settings page scrollarea - ([19d0f4f](https://github.com/koharu-rs/koharu/commit/19d0f4fb981cd6b957ebd104a1be5f6d16f2ffb3))
- Remove unnecessary check for Tauri in Providers component - ([7c08c2a](https://github.com/koharu-rs/koharu/commit/7c08c2af8d37bd720fa61f4774cb3fc2179d881f))
- Integrations test - ([44b4162](https://github.com/koharu-rs/koharu/commit/44b416216da6e09de18beafc55fef992d5dcf4dd))
- Use keyring credential deletion API - ([c78a021](https://github.com/koharu-rs/koharu/commit/c78a021d12b7ac7014e6560ad227d805f230cefe))

### 🚜 Refactor

- Switch ws to http - ([32ef47c](https://github.com/koharu-rs/koharu/commit/32ef47c6c97a2a5a0c411f2369137b3348e817d1))
- Avoid race condition - ([23f7d9f](https://github.com/koharu-rs/koharu/commit/23f7d9fb9b4e9fcbab16ce317660cb91576ce15f))
- Merge koharu-api into koharu-types - ([b499c50](https://github.com/koharu-rs/koharu/commit/b499c50f333fdeb8bfa92843ff01141080dffdaf))

### 📚 Documentation

- Add openai-compatible - ([550014a](https://github.com/koharu-rs/koharu/commit/550014a80e91b8f02d7de62658f67cefc32af789))
- Fix broken links - ([23b2063](https://github.com/koharu-rs/koharu/commit/23b2063fa6430d9f80d4b8079f59e1c1f091877e))
- Add ja & zh README - ([926dc02](https://github.com/koharu-rs/koharu/commit/926dc02e18617d7c210949128bd3a5b39cb1aba2))
- Fix typo - ([7ccbc9b](https://github.com/koharu-rs/koharu/commit/7ccbc9bbcc355fc7bb233fb4f0d0cc56b9ace244))
- Remove ja and zh-CN readme - ([b39a150](https://github.com/koharu-rs/koharu/commit/b39a1507ec25f4fe0b6d572a4ad70f61738c0b65))
- Fix typo - ([e481ae5](https://github.com/koharu-rs/koharu/commit/e481ae50e88337fc7fa948f1d8c40b4e520ec63d))

### ⚙️ Miscellaneous Tasks

- Update deps - ([9cdec47](https://github.com/koharu-rs/koharu/commit/9cdec475fb901fbefce0e62dc9ec05de454d2f6c))
- Format ui - ([cf466b6](https://github.com/koharu-rs/koharu/commit/cf466b6d00be814d27170d70a4bf99475de715bc))
- Rename LICENSE - ([fd0ad72](https://github.com/koharu-rs/koharu/commit/fd0ad7278121cfa85cb9b57b9778cdc48bb85a1d))
- Remove APACHE license - ([375c880](https://github.com/koharu-rs/koharu/commit/375c8800ead93cfbfa6ae7a854b34020d83439a1))
- Add cargo fmt - ([58fc0e8](https://github.com/koharu-rs/koharu/commit/58fc0e84a4ecb24d8f2382c54e5258af2fafe68d))


## [0.37.0](https://github.com/koharu-rs/koharu/compare/0.36.1..0.37.0) - 2026-03-11

### ⛰️  Features

- Add bulk export - ([1398b03](https://github.com/koharu-rs/koharu/commit/1398b0355cc9822adbd751e8db3beef4679e6eec))
- Enhance color picker - ([baed0b0](https://github.com/koharu-rs/koharu/commit/baed0b00aafbf756f895dfdb20a7ebf1621a6232))
- Block-aware inpainting - ([b292ea4](https://github.com/koharu-rs/koharu/commit/b292ea40635770701dbe14b3c63475826615709c))
- Add mit48px OCR backend - ([fbab2ef](https://github.com/koharu-rs/koharu/commit/fbab2efdd0e276b916465ab7d01ee7629cf17e2e))
- Overhaul comic text detector pipeline - ([bdcb933](https://github.com/koharu-rs/koharu/commit/bdcb933035c107aefa5432841cedb15c73f73d3d))

### 🐛 Bug Fixes

- *(renderer)* Manual size, do not auto-expand. - ([e9a26f1](https://github.com/koharu-rs/koharu/commit/e9a26f1b1c3076b830f0752a02a48f5111859312))
- *(renderer)* Handle new line break - ([bb18ba9](https://github.com/koharu-rs/koharu/commit/bb18ba9748377599c5348f640e48ebb09b7ec86a))
- *(ui)* Do not automatically display Inpainted Layer when creating new text box or adjusting text box size - ([c235dd1](https://github.com/koharu-rs/koharu/commit/c235dd18ee0f7e0691b6a1bb932ab134bd858fef))
- Normalize ocr text - ([17fa5f6](https://github.com/koharu-rs/koharu/commit/17fa5f62c729086b855d8849b22a697d44099527))
- Improve hunyuan result - ([f4962ab](https://github.com/koharu-rs/koharu/commit/f4962ab3a978bcde81f7ba0151e63602d69d5947))
- Korean text (Hangul) incorrectly rendered vertically in tall text blocks - ([0d55c6c](https://github.com/koharu-rs/koharu/commit/0d55c6cb160be6a1a856769524a6f0fb9ef262e2))
- Do not automatically apply Repair Brush in boxes at the "Detect" step - ([556442a](https://github.com/koharu-rs/koharu/commit/556442a70a64d6f4fde6983c50cd4a0edcf280e5))
- Adjust LLM generate options - ([704d858](https://github.com/koharu-rs/koharu/commit/704d858ea0e67d9f41db0089f55403a9330640d6))

### 🧪 Testing

- Remove unless test - ([6c38a75](https://github.com/koharu-rs/koharu/commit/6c38a75be5f10af3bcd7d264f79d642d6d385103))

### ⚙️ Miscellaneous Tasks

- *(dev)* Fix cudnn path - ([19d67d8](https://github.com/koharu-rs/koharu/commit/19d67d83ec3e1efe30a4585423315d911e2787d2))
- Upload portable exe - ([8f5eb50](https://github.com/koharu-rs/koharu/commit/8f5eb503f4cfeb404075520c1141e4144166600a))


## [0.36.1](https://github.com/koharu-rs/koharu/compare/0.36.0..0.36.1) - 2026-03-08

### 🐛 Bug Fixes

- Unable to resize text box on latin text box - ([c3eb3af](https://github.com/koharu-rs/koharu/commit/c3eb3afa01de3557379fef860d22d4cec777699b))


## [0.36.0](https://github.com/koharu-rs/koharu/compare/0.35.0..0.36.0) - 2026-03-08

### ⛰️  Features

- *(ui)* Move font settings to tabs - ([5de019c](https://github.com/koharu-rs/koharu/commit/5de019c677d8de0a1559daa98a6444e8b0cea5b9))
- Structured prompt - ([6882aae](https://github.com/koharu-rs/koharu/commit/6882aae16e53c11b3d70961482d214200a3f94e8))
- Text align options - ([1f2ee01](https://github.com/koharu-rs/koharu/commit/1f2ee0162e0c2b19fa812780e812f007e10ad50c))
- Improve english text rendering & layout - ([62c2556](https://github.com/koharu-rs/koharu/commit/62c255602f4bb63fd83cfc6be9bcff6a562a975d))
- Punctuation normalization in vertical mode - ([62fc37a](https://github.com/koharu-rs/koharu/commit/62fc37ab574597241d6b666ed270861fda01cbc2))
- Centering full-width punctuation in vertical layout - ([3b87b49](https://github.com/koharu-rs/koharu/commit/3b87b492709fd36197405327dd934db293cadcf9))
- Add italic, bold, border - ([24894d4](https://github.com/koharu-rs/koharu/commit/24894d48094f24e6b600b0d3677662f08543a606))

### 🐛 Bug Fixes

- Global font family & disable image transition - ([42ca4e8](https://github.com/koharu-rs/koharu/commit/42ca4e8695077414270dda95854f891d1456404d))
- Fix: smooth text editing
Fixes #197 - ([8de0019](https://github.com/koharu-rs/koharu/commit/8de00194dd20a1a742099926de8145739ddc18ec))

### 📚 Documentation

- New screenshots - ([1a980ec](https://github.com/koharu-rs/koharu/commit/1a980ecce805e508852f9cff566739632b5568bb))
- Add zh-CN readme - ([77f01ab](https://github.com/koharu-rs/koharu/commit/77f01abada35a17f29711df9e436b12827b71832))
- Add note for cuda driver - ([cfdacfb](https://github.com/koharu-rs/koharu/commit/cfdacfbb073420dc16fca11a10d712a3774c57bb))

### ⚙️ Miscellaneous Tasks

- Strip quotes from translated text - ([0501c54](https://github.com/koharu-rs/koharu/commit/0501c546772cf332a7525aff53e8a515b46d044e))
- Make clippy happy - ([8464b17](https://github.com/koharu-rs/koharu/commit/8464b1775fbe710491065681af85ed33e391f6f4))


## [0.35.0](https://github.com/koharu-rs/koharu/compare/0.34.0..0.35.0) - 2026-03-07

### ⛰️  Features

- Implement error handling and UI for RPC errors - ([64e762c](https://github.com/koharu-rs/koharu/commit/64e762c9d0787c9782036462c455c13e280d1ee7))
- Add loading indicators for pipeline step buttons - ([b50aac1](https://github.com/koharu-rs/koharu/commit/b50aac1748a60e1364879136e3442382c5e0ce5a))
- Add 'Add Documents' functionality - ([8083949](https://github.com/koharu-rs/koharu/commit/80839494c94d8f643737b6bd633bdb9205781081))
- Add Spanish translations and update language options ([#198](https://github.com/koharu-rs/koharu/issues/198)) - ([a9b0091](https://github.com/koharu-rs/koharu/commit/a9b00912391b8ebcf68d6220d1cd8659e95f915f))
- Add `ru-RU` translations ([#195](https://github.com/koharu-rs/koharu/issues/195)) - ([bbde259](https://github.com/koharu-rs/koharu/commit/bbde259e416c4d6abd716a8878e4265cc97c24ec))

### 🐛 Bug Fixes

- For issue https://github.com/koharu-rs/koharu/issues/197 - ([760eccd](https://github.com/koharu-rs/koharu/commit/760eccd1b6548d8ba04a15565c2b10a3067b99d4))

### 🚜 Refactor

- Testable ui - ([ae044ce](https://github.com/koharu-rs/koharu/commit/ae044ce8b765ea978775d90f757ed5bcb6ab56de))
- Reduce duplicates in ui - ([e6244fb](https://github.com/koharu-rs/koharu/commit/e6244fbd3214d1f9cfbbdfe6f0a95397436ef522))
- Reduce duplicates - ([06ef2f7](https://github.com/koharu-rs/koharu/commit/06ef2f76cb9726370a08dedfbadf21e681c91bf7))

### 🧪 Testing

- E2e - ([383b777](https://github.com/koharu-rs/koharu/commit/383b77733d05da64ae3895e29340690923a4401c))

### ⚙️ Miscellaneous Tasks

- Update deps - ([211ae37](https://github.com/koharu-rs/koharu/commit/211ae376ac9a5293f05cf6921bd345b9251c9d19))


## [0.34.0](https://github.com/koharu-rs/koharu/compare/0.33.0..0.34.0) - 2026-02-21

### ⛰️  Features

- Feat: persist font family
close #176 - ([3837499](https://github.com/koharu-rs/koharu/commit/3837499196bb218e14e973cdc4cfb4901003ef12))

### 🚜 Refactor

- Remove sys-locale - ([e3f180b](https://github.com/koharu-rs/koharu/commit/e3f180bdf479eeca9fe6cbb7c5682153220ae2d0))
- Simplify device name logic - ([cd0eb2f](https://github.com/koharu-rs/koharu/commit/cd0eb2fa41f1f841e9553f3218ae810ac797db8d))

### 📚 Documentation

- Tidy changelog - ([c24e00c](https://github.com/koharu-rs/koharu/commit/c24e00c28140c31e91d3fe89168b829efff0d440))
- Update copyright year to 2026 and owner to Mayo Takanashi in license files. - ([85ddf52](https://github.com/koharu-rs/koharu/commit/85ddf52832fafacf1dc1d585bc1cacc90ab7dd6e))
- Update READMEs to include Linux in the list of supported pre-built binary platforms. - ([0a4ba70](https://github.com/koharu-rs/koharu/commit/0a4ba70e518b69d641692ef28f60facc2b0133de))

### ⚙️ Miscellaneous Tasks

- Disable ad shown by i18next - ([2e19d48](https://github.com/koharu-rs/koharu/commit/2e19d48c1a96e1e159bdda5d46ed956a1e72dae4))
- Remove unused fonts - ([0c6d19d](https://github.com/koharu-rs/koharu/commit/0c6d19db564e88d6bbf971adfd6dbe9eb2984bdd))
- Update rust deps - ([d2cc55a](https://github.com/koharu-rs/koharu/commit/d2cc55a2e223bd05335cab384069fb522d960c89))
- Update deps - ([172913d](https://github.com/koharu-rs/koharu/commit/172913dc543e14967867267308421d15522d6c1f))
- Remove unused deps - ([6af61fc](https://github.com/koharu-rs/koharu/commit/6af61fc29a75283fbaa28f64ee590d95c9f47432))


## [0.33.0](https://github.com/koharu-rs/koharu/compare/0.32.8..0.33.0) - 2026-02-14

### ⛰️  Features

- Mcp server - ([d31f7df](https://github.com/koharu-rs/koharu/commit/d31f7dfdd666cf2d9ccd4262b4bfb8461f34f670))

### 🚜 Refactor

- Split modules - ([860f474](https://github.com/koharu-rs/koharu/commit/860f474dfa00e13e418be4f79913c40293fec60c))
- Remove unused result wrapper - ([96e2b19](https://github.com/koharu-rs/koharu/commit/96e2b193b92792e55cf9222d90ceba11fb4ebe68))

### 📚 Documentation

- Do not mention 7z - ([efdaa3b](https://github.com/koharu-rs/koharu/commit/efdaa3b3e96aacbe638e581171af9029c5d7b14d))

### ⚙️ Miscellaneous Tasks

- Remove portable path - ([990af47](https://github.com/koharu-rs/koharu/commit/990af47522e0ae9c6a8989ccdbb655e32f9b0c0f))


## [0.32.8](https://github.com/koharu-rs/koharu/compare/0.32.7..0.32.8) - 2026-02-11

### 🐛 Bug Fixes

- Fix: race condition in ui
Fixes #170 - ([88e6e7a](https://github.com/koharu-rs/koharu/commit/88e6e7aadeafb0aa2c80cd8ecee2abd0ad696a22))

### ⚙️ Miscellaneous Tasks

- Remove lint-stage & husky - ([8ac5b18](https://github.com/koharu-rs/koharu/commit/8ac5b18a3a2fab1ac2c368f6a11bcfb6d36bb009))


## [0.32.7](https://github.com/koharu-rs/koharu/compare/0.32.6..0.32.7) - 2026-02-11

### 🐛 Bug Fixes

- Add polyfill for file system api - ([d6c7362](https://github.com/koharu-rs/koharu/commit/d6c73624c6fd692479013dcfa55bfe2777ed01de))


## [0.32.6](https://github.com/koharu-rs/koharu/compare/0.32.5..0.32.6) - 2026-02-11

### ⚙️ Miscellaneous Tasks

- Update deps - ([f1cf37b](https://github.com/koharu-rs/koharu/commit/f1cf37b60f6accb7dc2c6d4f5a154a9a5b34c0a9))


## [0.32.5](https://github.com/koharu-rs/koharu/compare/0.32.4..0.32.5) - 2026-02-11

### 🐛 Bug Fixes

- Update tauri pubkey - ([bcc3f2d](https://github.com/koharu-rs/koharu/commit/bcc3f2da5618ecfa529745625cacf4f830b00ee4))


## [0.32.4](https://github.com/koharu-rs/koharu/compare/0.32.3..0.32.4) - 2026-02-11

### 🐛 Bug Fixes

- Update tauri pubkey - ([d3234b4](https://github.com/koharu-rs/koharu/commit/d3234b4f7474209340d62877f3192914c8556f30))


## [0.32.3](https://github.com/koharu-rs/koharu/compare/0.32.2..0.32.3) - 2026-02-11

### 🐛 Bug Fixes

- Update tauri key - ([50bfad4](https://github.com/koharu-rs/koharu/commit/50bfad47f64d6887ff84eb35c1c2a0733488d4bc))


## [0.32.2](https://github.com/koharu-rs/koharu/compare/0.32.1..0.32.2) - 2026-02-11

### 🐛 Bug Fixes

- Tauri updater password - ([3870dfa](https://github.com/koharu-rs/koharu/commit/3870dfa61c589e13fa5b073ad1116bb136f46171))


## [0.32.1](https://github.com/koharu-rs/koharu/compare/0.32.0..0.32.1) - 2026-02-11

### 🐛 Bug Fixes

- Add serde_json - ([5a25fdc](https://github.com/koharu-rs/koharu/commit/5a25fdcd53e2954aeb2e4c9f9033dc10ed67e857))


## [0.32.0](https://github.com/koharu-rs/koharu/compare/0.31.6..0.32.0) - 2026-02-11

### ⛰️  Features

- Tauri updater - ([cbb22e7](https://github.com/koharu-rs/koharu/commit/cbb22e7758c4d75b0120cedd2db593f6bcb78198))

### ⚙️ Miscellaneous Tasks

- Fix lint & build - ([ac11eeb](https://github.com/koharu-rs/koharu/commit/ac11eeb30523a50e4c7ae59afd88142fa8f87e74))


## [0.31.6](https://github.com/koharu-rs/koharu/compare/0.31.5..0.31.6) - 2026-02-11

### 🐛 Bug Fixes

- Windows release build - ([4aad5a5](https://github.com/koharu-rs/koharu/commit/4aad5a56383db15f60bb9682b680cb85d163c21a))


## [0.31.5](https://github.com/koharu-rs/koharu/compare/0.31.4..0.31.5) - 2026-02-11

### 🐛 Bug Fixes

- Tauri bundle config - ([5a53752](https://github.com/koharu-rs/koharu/commit/5a53752014afed65d421825cab255db92cf78a66))


## [0.31.4](https://github.com/koharu-rs/koharu/compare/0.31.3..0.31.4) - 2026-02-11

### 🐛 Bug Fixes

- Apple certificate - ([0b76382](https://github.com/koharu-rs/koharu/commit/0b7638275e9b1a760d535fc442bd9fa461ff67f2))


## [0.31.3](https://github.com/koharu-rs/koharu/compare/0.31.2..0.31.3) - 2026-02-11

### 🚜 Refactor

- Remove auto-update - ([35f2622](https://github.com/koharu-rs/koharu/commit/35f2622da098ec480365acc47a5e459814ed385f))


## [0.31.2](https://github.com/koharu-rs/koharu/compare/0.31.1..0.31.2) - 2026-02-11

### 🐛 Bug Fixes

- Linux release bundle - ([5ea500a](https://github.com/koharu-rs/koharu/commit/5ea500af877c391aeb07c9befd6fda943e255111))


## [0.31.1](https://github.com/koharu-rs/koharu/compare/0.31.0..0.31.1) - 2026-02-11

### 🐛 Bug Fixes

- Linux release build - ([804d1a6](https://github.com/koharu-rs/koharu/commit/804d1a65b415a82f6c3a5455823aeaed9e92858f))

### ⚙️ Miscellaneous Tasks

- Support linux - ([23c65a6](https://github.com/koharu-rs/koharu/commit/23c65a662ea9d39e8030a86d3a934cc8bd4d3788))


## [0.31.0](https://github.com/koharu-rs/koharu/compare/0.30.1..0.31.0) - 2026-02-08

### ⛰️  Features

- Websocket-based rpc - ([0304c5c](https://github.com/koharu-rs/koharu/commit/0304c5c1d4fc7769472065cefcf8fe42b2f497d0))

### 🐛 Bug Fixes

- Fix: patch cudarc to load cufft 12.x
Fix #169 - ([e09a2b2](https://github.com/koharu-rs/koharu/commit/e09a2b2d14e875763a756140d4c76194cf71e5b7))

### ⚙️ Miscellaneous Tasks

- Remove unused tower-http - ([40b84a0](https://github.com/koharu-rs/koharu/commit/40b84a097951ca510048253d3963b3ea368acab1))


## [0.30.1](https://github.com/koharu-rs/koharu/compare/0.30.0..0.30.1) - 2026-02-04

### 🐛 Bug Fixes

- Cuda missing header - ([ae317cc](https://github.com/koharu-rs/koharu/commit/ae317ccdf55007e5d4719cc36b1ac7aa2a7a296a))


## [0.30.0](https://github.com/koharu-rs/koharu/compare/0.29.0..0.30.0) - 2026-02-04

### ⛰️  Features

- Rust-based processing - ([df70fd2](https://github.com/koharu-rs/koharu/commit/df70fd259eaca3666de10c22ec63e7432693b3da))
- CUDA 13.1 - ([ff00dc2](https://github.com/koharu-rs/koharu/commit/ff00dc2925491d19896f6bf5e06a562a12c6c109))
- Download progress indicator - ([ce57af9](https://github.com/koharu-rs/koharu/commit/ce57af9d1ca7816f39736082472ed8d0d8a3f60b))

### 📚 Documentation

- Add contributors - ([dd6f3d6](https://github.com/koharu-rs/koharu/commit/dd6f3d6ddeeaca843492214216b6a02172938ba1))
- Add ja README - ([f14a0ee](https://github.com/koharu-rs/koharu/commit/f14a0eeff2a7182dbbaf559ede306bfbe071ce0a))

### ⚙️ Miscellaneous Tasks

- Make clippy happy - ([48e1c53](https://github.com/koharu-rs/koharu/commit/48e1c53ec7933aafb5e7554257c995a9bd392758))
- Remove unused settings - ([c48ed3b](https://github.com/koharu-rs/koharu/commit/c48ed3b4b3b80069d1e0868a0f1cf6fd079b6ff8))


## [0.29.0](https://github.com/koharu-rs/koharu/compare/0.28.0..0.29.0) - 2026-02-01

### ⛰️  Features

- Add color-picker component - ([45f804d](https://github.com/koharu-rs/koharu/commit/45f804d9c7ac4ea116200e8c8b47da64bb1e5e79))
- Unify components usage - ([75104d4](https://github.com/koharu-rs/koharu/commit/75104d45f1b9aeee666d83f179820a93b5047ba3))

### 🐛 Bug Fixes

- Use file system api - ([fa7607f](https://github.com/koharu-rs/koharu/commit/fa7607f3d50423773ebbf039b9a5da44e8c1f386))
- Make ui text not selectable - ([ec128e3](https://github.com/koharu-rs/koharu/commit/ec128e37e6d804654c11bc114e9224b44a6e7f0b))

### 🚜 Refactor

- Remove openai-compatible feature - ([8261e66](https://github.com/koharu-rs/koharu/commit/8261e66354afd1587d1dd0314126e87951791f3f))

### 📚 Documentation

- Add logo - ([b263cab](https://github.com/koharu-rs/koharu/commit/b263cab3056fcac25d560c12865711f2ffc1bede))

### ⚙️ Miscellaneous Tasks

- Remove logo - ([e19e2df](https://github.com/koharu-rs/koharu/commit/e19e2df29e8ae65e0557613d8fe9c2308eac30ef))
- Make indicator vertically align - ([5598069](https://github.com/koharu-rs/koharu/commit/5598069b8c682724b4f3e84f1de6b22f3915d59f))
- Update deps - ([cc19baa](https://github.com/koharu-rs/koharu/commit/cc19baa2553b8ff883ece7bfa965ff3750a407f5))
- Better i18n - ([ddef086](https://github.com/koharu-rs/koharu/commit/ddef086ac290f8f7cf28f4c5f2e55eb1cc69cf36))
- Make toolbar more attractive - ([4c67064](https://github.com/koharu-rs/koharu/commit/4c67064083173b7ad8038f511c44a1b36572cd43))
- Remove AGENTS.md - ([87b85bc](https://github.com/koharu-rs/koharu/commit/87b85bc1df9f23da92264d4826c195e58dfaa60f))
- Remove unused types - ([edf1142](https://github.com/koharu-rs/koharu/commit/edf1142b92c5126a672048edb73f43fc1ff2a1f4))


## [0.28.0](https://github.com/koharu-rs/koharu/compare/0.27.2..0.28.0) - 2026-02-01

### ⛰️  Features

- Custom titlebar - ([fa245d1](https://github.com/koharu-rs/koharu/commit/fa245d1a0e30b479f06d2af589bfe0cabfb71457))


## [0.27.2](https://github.com/koharu-rs/koharu/compare/0.27.1..0.27.2) - 2026-02-01

### 🐛 Bug Fixes

- Disable prefetch to make navigation work - ([c801192](https://github.com/koharu-rs/koharu/commit/c801192a0ee791f9b5c32645ccfa9e3a4905b788))
- Listen port lazy - ([e6957c0](https://github.com/koharu-rs/koharu/commit/e6957c012553bc54135d2fb6743f1853d1f600b8))
- Macos titlebar - ([c86f9aa](https://github.com/koharu-rs/koharu/commit/c86f9aa31c9f78c09e8d61e15f6c6d8907eef999))

### 🚜 Refactor

- Remove hf mirror - ([ae1698e](https://github.com/koharu-rs/koharu/commit/ae1698e33462374e83062c240a6ff498dfa5b8e0))
- Simplify app.rs - ([abb1961](https://github.com/koharu-rs/koharu/commit/abb1961bc95be39067a69ef189247f6cd8b5172a))
- Remove tauri http plugin - ([1c6c845](https://github.com/koharu-rs/koharu/commit/1c6c8454df0fe602cb3c845f96530ce411e76c5c))
- Remove macros - ([83849f7](https://github.com/koharu-rs/koharu/commit/83849f7a8a3908252d1b040c3dff471d0cbded31))

### ⚙️ Miscellaneous Tasks

- Add more context info - ([8aa7e38](https://github.com/koharu-rs/koharu/commit/8aa7e38e5827e9f2a9c39efd5b72f7f4b6acfdbf))
- Rename scripts - ([2eb84f3](https://github.com/koharu-rs/koharu/commit/2eb84f3b4085fa1f648d40cddb7c83c3e4e50467))


## [0.27.1](https://github.com/koharu-rs/koharu/compare/0.27.0..0.27.1) - 2026-01-27

### 🐛 Bug Fixes

- Use tauri based http - ([f82b376](https://github.com/koharu-rs/koharu/commit/f82b376c4b65fb21687fed59dc54a0bb441bcd69))

### ⚙️ Miscellaneous Tasks

- Change window size to 1024x768 - ([e57bf42](https://github.com/koharu-rs/koharu/commit/e57bf4221eb396d7de1170a33ef6b940b8248b72))


## [0.27.0](https://github.com/koharu-rs/koharu/compare/0.26.2..0.27.0) - 2026-01-27

### ⛰️  Features

- Add motion - ([c4ea493](https://github.com/koharu-rs/koharu/commit/c4ea493a85cc5dee4ff9f0af17921fe315b4ef2c))
- Redesign interface - ([67e5aff](https://github.com/koharu-rs/koharu/commit/67e5aff9691a43f0013de625a0172a8a5d731b98))
- Device info - ([cbe4597](https://github.com/koharu-rs/koharu/commit/cbe4597dcc222f25f0ce01bb0f6f51e545b34e52))
- About page - ([352166a](https://github.com/koharu-rs/koharu/commit/352166a57a3474b8572feec76ef87884afa4b722))
- Settings page & dark mode - ([c74f4fc](https://github.com/koharu-rs/koharu/commit/c74f4fc0fc2c2b18f3c5b5386509f907432e61d6))
- Use shadcn ui components - ([4221b2d](https://github.com/koharu-rs/koharu/commit/4221b2d337745927f0af89550b697cebf6bf1d3d))

### 🐛 Bug Fixes

- Status bar being hidden - ([f7ac35c](https://github.com/koharu-rs/koharu/commit/f7ac35cf1c50a5c68fc662726065314b769bdfca))
- Preview is stale - ([d8f48f9](https://github.com/koharu-rs/koharu/commit/d8f48f9afdbc1bc1e2f8ea3c553b05b119e2780a))
- Type error - ([e399363](https://github.com/koharu-rs/koharu/commit/e3993630c6959b7c5f2c519321a4b02700bd858f))
- Types mismatch - ([c5bbee6](https://github.com/koharu-rs/koharu/commit/c5bbee6e0b9a4afe3959c92413f7328ecb176434))
- Fix: crash when loading large images
Fixes #118 - ([8afb656](https://github.com/koharu-rs/koharu/commit/8afb65673af20fab1ffdd4ec5ffc5f03adaef1d7))
- Revert moe changes for candle - ([b70d9d2](https://github.com/koharu-rs/koharu/commit/b70d9d24b879901cd2d9b49f8997b5eea685de15))
- Patch ug - ([e8efb54](https://github.com/koharu-rs/koharu/commit/e8efb54ca174a49d6670bc19f7531b00c5ae0c2f))

### 🚜 Refactor

- I18n - ([c6a34d5](https://github.com/koharu-rs/koharu/commit/c6a34d575e01468258032d23eaa7106c00ab0391))
- Use single document view - ([4429015](https://github.com/koharu-rs/koharu/commit/4429015a9476bbf9de1c2df98343711f010181a1))
- Separate api.rs into endpoints.rs and server.rs - ([eb1018a](https://github.com/koharu-rs/koharu/commit/eb1018a1019ec3f452de3ecb0868b23e0c07dc9b))
- Introduce koharu-macros - ([bf2fa16](https://github.com/koharu-rs/koharu/commit/bf2fa1652e24ccc02ffeeb3bff95880bc7772bb8))
- Update API - ([37eca4f](https://github.com/koharu-rs/koharu/commit/37eca4fbe5fe6a98fd66b6c38351e3d1b7dda982))
- Use HTTP based API instead of Tauri commands - ([38d631e](https://github.com/koharu-rs/koharu/commit/38d631e441d9f6cb1ad1f0c9f0a2078d979a5c3a))
- Remove prefetch - ([27be8a1](https://github.com/koharu-rs/koharu/commit/27be8a1b0b23c5b8b25139c1d5df28cfb278ceee))
- Remove startup documents - ([926f467](https://github.com/koharu-rs/koharu/commit/926f4679e32df889d00bfc98e351dc0d9d72ea14))
- Create windows.rs - ([1148022](https://github.com/koharu-rs/koharu/commit/1148022b0dbff6beca339bbe09e2d4c18120a41f))
- Clean up embed - ([ce9280c](https://github.com/koharu-rs/koharu/commit/ce9280c27202c1fadc92b7382dfec6989f9e12df))

### 📚 Documentation

- Do not mention crs since it's buggy - ([ca1bcfa](https://github.com/koharu-rs/koharu/commit/ca1bcfa9951ded3de47c42680f76b7e9ce946910))
- Add CONTRIBUTING.md - ([445518d](https://github.com/koharu-rs/koharu/commit/445518deed0603af5d35c8c1184ac8001f199d9a))
- Remove mentioning 7z - ([d19b280](https://github.com/koharu-rs/koharu/commit/d19b2807f19dfa482cc84fae2306aed68ad20bab))

### ⚙️ Miscellaneous Tasks

- Remove unused deps - ([0945b5d](https://github.com/koharu-rs/koharu/commit/0945b5d2426dd3b60e18eef57cadcecef06b01ed))
- Remove unused dep - ([5516479](https://github.com/koharu-rs/koharu/commit/55164795ad8705c30742d46909cafd3d6822aaaa))
- Update deps - ([6942085](https://github.com/koharu-rs/koharu/commit/6942085bd9a1d6ff42e83c6be8c42ad7813224c4))
- Update frontend deps - ([4ec6b80](https://github.com/koharu-rs/koharu/commit/4ec6b80d9f90f0aa3a5cd2f0e907065ee32790f6))
- Make clippy happy - ([b86fcdc](https://github.com/koharu-rs/koharu/commit/b86fcdce0f09becd8f089aafdf6f78c4ebbcef77))
- Add AGENTS.md - ([91dc5cd](https://github.com/koharu-rs/koharu/commit/91dc5cd9f4aebae5d7ae4a4f026d6256514096e8))
- Update cudarc to 0.19.0 - ([b7f783d](https://github.com/koharu-rs/koharu/commit/b7f783d931506fcc1051e8eef143585937167e74))
- Remove unused deps - ([2ca38aa](https://github.com/koharu-rs/koharu/commit/2ca38aa269366d4117baf09666165fd74e963ad0))
- Add author fffonion - ([11941ae](https://github.com/koharu-rs/koharu/commit/11941aec5b13f730b53d97987b97b32ba00157f3))
- Update to candle 0.9.2 - ([51324a1](https://github.com/koharu-rs/koharu/commit/51324a17be06b47fe52d153ba611efceff290ee1))
- Make clippy happy - ([6426ad7](https://github.com/koharu-rs/koharu/commit/6426ad70c0503f16649dce6f7b646297c7ad91bd))
- Make clippy happy - ([0fe1803](https://github.com/koharu-rs/koharu/commit/0fe180320e46db90b2fe0c7de8dec104685be894))
- Add git-version - ([4e25aa7](https://github.com/koharu-rs/koharu/commit/4e25aa7962a8a682053f74004caafc93977ce455))


## [0.26.2](https://github.com/koharu-rs/koharu/compare/0.26.1..0.26.2) - 2026-01-21

### ⚙️ Miscellaneous Tasks

- Setup dotnet 9.0.x for macos - ([03d4fbc](https://github.com/koharu-rs/koharu/commit/03d4fbc2d7e93bdb127f7fc4d6474987cd51a08b))


## [0.26.1](https://github.com/koharu-rs/koharu/compare/0.26.0..0.26.1) - 2026-01-20

### ⚙️ Miscellaneous Tasks

- Setup dotnet - ([0d182d9](https://github.com/koharu-rs/koharu/commit/0d182d907a3f4fb0ebf0ce255cd20ba891269615))


## [0.26.0](https://github.com/koharu-rs/koharu/compare/0.25.4..0.26.0) - 2026-01-20

### ⛰️  Features

- Display release notes - ([cd7ab24](https://github.com/koharu-rs/koharu/commit/cd7ab24710ea8325dc9b4f474c320cb53dac4234))

### 🐛 Bug Fixes

- Pin cudnn version - ([ea64339](https://github.com/koharu-rs/koharu/commit/ea643396181a5328fb81607e183da2792b7cb6e3))
- Select default fonts based on OS and locale - ([f259e37](https://github.com/koharu-rs/koharu/commit/f259e37db76a3212018f8accaf07f41258a55829))
- Show release notes from markdown source - ([3ef1e1a](https://github.com/koharu-rs/koharu/commit/3ef1e1a7b33602483420158d5074010fc0b62f94))
- Enable ansi color mode on the debug console too - ([91145c1](https://github.com/koharu-rs/koharu/commit/91145c1b3e5408871809e2e71ee90ce5bca457a2))

### ⚙️ Miscellaneous Tasks

- Cargo update dependencies - ([90fc5a4](https://github.com/koharu-rs/koharu/commit/90fc5a45ade1927356b9f1b1da5e8e0f868a8ffe))
- Clarifying ml in readme - ([efe2fca](https://github.com/koharu-rs/koharu/commit/efe2fca9a6407558c3c3e6972dc1011292af0eaf))
- Update dependencies - ([f760621](https://github.com/koharu-rs/koharu/commit/f76062162f50b56c2be60e772f38a27863c4b8ef))
- Improve dev performance - ([0bc56ed](https://github.com/koharu-rs/koharu/commit/0bc56edd3dd6178696be414a8a7d78655d7c80b2))


## [0.25.4](https://github.com/koharu-rs/koharu/compare/0.25.3..0.25.4) - 2025-12-24

### ⚙️ Miscellaneous Tasks

- Bundle release notes in velopack - ([428b140](https://github.com/koharu-rs/koharu/commit/428b140e39fd51c4a314d13fd44bab38cbb0176e))
- Add release notes - ([c215017](https://github.com/koharu-rs/koharu/commit/c21501702a8a69ef23cf3649c1f2a309e061dd53))


## [0.25.2](https://github.com/koharu-rs/koharu/compare/0.25.1..0.25.2) - 2025-12-24

### 🐛 Bug Fixes

- Font fallbacks ([#115](https://github.com/koharu-rs/koharu/issues/115)) - ([af2ab38](https://github.com/koharu-rs/koharu/commit/af2ab388f9c26f71377da3b1a4f12125c9d73c3f))
- Lint happier - ([6f08d28](https://github.com/koharu-rs/koharu/commit/6f08d289967fd4b0093c5c989ee2a68cfc844170))

### ⚙️ Miscellaneous Tasks

- Add benchmark for renderer - ([a78fb20](https://github.com/koharu-rs/koharu/commit/a78fb20e515b3c21942d28c0f837f6e8bac57261))


## [0.25.1](https://github.com/koharu-rs/koharu/compare/0.25.0..0.25.1) - 2025-12-23

### 🐛 Bug Fixes

- Adjust tooltip - ([12feab7](https://github.com/koharu-rs/koharu/commit/12feab75697040efb15f0db0ab054690a36f3c2a))
- Rebase error - ([444858b](https://github.com/koharu-rs/koharu/commit/444858b0b13de8357be6c75cc2eae57c956652cc))

### 📚 Documentation

- Update readme - ([81646f8](https://github.com/koharu-rs/koharu/commit/81646f8ff8872fc337ca47aa78b46de446f9ee2d))


## [0.25.0](https://github.com/koharu-rs/koharu/compare/0.24.0..0.25.0) - 2025-12-23

### ⛰️  Features

- Add comic read script API - ([57ed7f5](https://github.com/koharu-rs/koharu/commit/57ed7f5fce7efc6cd1b3c139c3b4ea56211b6423))

### 🐛 Bug Fixes

- Crs api load llm on first request - ([77c26b3](https://github.com/koharu-rs/koharu/commit/77c26b3fb0f43686836043c2ef265379bf833fde))
- Lint - ([80878ee](https://github.com/koharu-rs/koharu/commit/80878eecbb071fe881df91a85fbf65d85c59520a))

### ⚙️ Miscellaneous Tasks

- Reorganize code - ([7ac87c9](https://github.com/koharu-rs/koharu/commit/7ac87c90735a5bf9a3397384c6bc69772973bf0f))


## [0.24.0](https://github.com/koharu-rs/koharu/compare/0.23.1..0.24.0) - 2025-12-23

### ⛰️  Features

- Add headless mode to be used in browser - ([57b1bc5](https://github.com/koharu-rs/koharu/commit/57b1bc5fe1650e36c1fb20661aad9aa7f12e7e79))

### 🐛 Bug Fixes

- Correct image layer - ([6f30121](https://github.com/koharu-rs/koharu/commit/6f3012167dde2d6b9d9209b36b423b94c801de84))


## [0.23.1](https://github.com/koharu-rs/koharu/compare/0.23.0..0.23.1) - 2025-12-22

### 🐛 Bug Fixes

- Smaller color input button - ([dd4675e](https://github.com/koharu-rs/koharu/commit/dd4675e9450c7ce870f54c838adbcdad00587696))
- Render brush - ([6907bd4](https://github.com/koharu-rs/koharu/commit/6907bd495d0dd80076f740059fcdd9b027090055))


## [0.23.0](https://github.com/koharu-rs/koharu/compare/0.22.0..0.23.0) - 2025-12-22

### ⛰️  Features

- Drag and zoom - ([857ddfb](https://github.com/koharu-rs/koharu/commit/857ddfb85cbc78384c3a256dadcfe79dd89fc912))
- Color brush and erasor - ([47432e5](https://github.com/koharu-rs/koharu/commit/47432e50fb7d669efb340ced2266c2e558f8c439))
- Smoother image transition - ([cac5378](https://github.com/koharu-rs/koharu/commit/cac5378e268e50f2f780cea446128106b8b0edbc))
- Inpaint brush and erasor - ([c0fc0c6](https://github.com/koharu-rs/koharu/commit/c0fc0c6d33ca0a99886beec890d305f3201ed1c7))

### 🐛 Bug Fixes

- More greedy partial inpaint for text block change - ([c35f6c4](https://github.com/koharu-rs/koharu/commit/c35f6c4776a8c180582c568cdd58914d4c1b3060))
- Add non-null assertion for segment - ([2ffb7c7](https://github.com/koharu-rs/koharu/commit/2ffb7c7b66b3ace55db1e1becf3c67c6028fa796))
- Clippy happy - ([5922ec0](https://github.com/koharu-rs/koharu/commit/5922ec0481db458f90548b349bbcf90c96404bbe))

### ⚡ Performance

- Reduce unused partial inpaint - ([8cbdf3f](https://github.com/koharu-rs/koharu/commit/8cbdf3fd9442e34d913a6d88a68e3c597a35875c))
- Partial update mask - ([4c0c7bf](https://github.com/koharu-rs/koharu/commit/4c0c7bf15f95440e718c295cbb2d8646ef531181))

### ⚙️ Miscellaneous Tasks

- Align code style - ([6c13daf](https://github.com/koharu-rs/koharu/commit/6c13daf4ac8a82ba753192bffb5bf9e100ecfda9))
- Update dependencies - ([8a577d8](https://github.com/koharu-rs/koharu/commit/8a577d88af060b66e7deddaf48dd71fe26edbcc2))
- Do not cache cargo targets - ([5a3b224](https://github.com/koharu-rs/koharu/commit/5a3b224c0558555230981065b08ea6c98f599ef1))


## [0.22.0](https://github.com/koharu-rs/koharu/compare/0.21.0..0.22.0) - 2025-12-21

### ⛰️  Features

- Block font and color - ([c1b2ef3](https://github.com/koharu-rs/koharu/commit/c1b2ef33d451a659429ed28c83b9e18e9e435eba))
- Add some text shader styles - ([7ee49eb](https://github.com/koharu-rs/koharu/commit/7ee49eb2d0570da601d95d96931504d661571c74))
- Wgpu renderer ([#105](https://github.com/koharu-rs/koharu/issues/105)) - ([2ff2c7f](https://github.com/koharu-rs/koharu/commit/2ff2c7f9aee185d581bcf8720a76037caa1d4db9))
- Add openai endpoint support - ([074d570](https://github.com/koharu-rs/koharu/commit/074d570de6204231856d978eadf47ff390e55cf1))

### 🐛 Bug Fixes

- Wgpu renderer minor issues - ([8480c6c](https://github.com/koharu-rs/koharu/commit/8480c6ce9effea37dd1f21b328e2f8b16ac60b15))
- Integration tests - ([98bf9e6](https://github.com/koharu-rs/koharu/commit/98bf9e65a3206d0373646caeb7eb07a46a580c09))
- Add missing ui/components/ResizableSidebar.tsx - ([4cb959f](https://github.com/koharu-rs/koharu/commit/4cb959f588221eed2f87aef3792b75e612afd02a))
- Activity bubble animation and per operation sub title - ([293dfcb](https://github.com/koharu-rs/koharu/commit/293dfcb7224f2df7357d494b834612c8917a7b44))

### 📚 Documentation

- Cleanup per-crate docs - ([d860703](https://github.com/koharu-rs/koharu/commit/d8607036c9794cf7792f97aeea6df1d02b69e648))

### 🧪 Testing

- Yolo ignore tests - ([dec2e05](https://github.com/koharu-rs/koharu/commit/dec2e056606156f8387df574634fb6ab530ede2b))
- Ignore dylib tests - ([01a7cff](https://github.com/koharu-rs/koharu/commit/01a7cffc33c5f4c330a381c712bf79f92dcc6bf5))

### ⚙️ Miscellaneous Tasks

- Make clippy happy - ([c570630](https://github.com/koharu-rs/koharu/commit/c5706305f7f3590693df1581bc5a1fdcaa4df858))
- Hide debug window - ([341c853](https://github.com/koharu-rs/koharu/commit/341c853dd553a97515bc11c5e39bd18ac22d0f9b))
- Update ui dependencies - ([7f4771c](https://github.com/koharu-rs/koharu/commit/7f4771c4550c60f334346efaaed5145acd8cbaa6))
- Mark rendering integration tests as ignored due to lack of fonts - ([83a06fd](https://github.com/koharu-rs/koharu/commit/83a06fd234ce2820c7cd4f975e396f3593bfdecb))
- Update dependencies - ([96f59da](https://github.com/koharu-rs/koharu/commit/96f59da49eacbb19714b75283cc9a5f62493277d))
- Use format mode - ([90da89c](https://github.com/koharu-rs/koharu/commit/90da89c530bfab7973337b1d894e3a8e9fa54e39))
- Remove unused deppendencies - ([353cf69](https://github.com/koharu-rs/koharu/commit/353cf693a42e743279b38aa0bcc365cf27712821))

### Fieat

- Resizable navigator and panels - ([6da24a6](https://github.com/koharu-rs/koharu/commit/6da24a63a1de6b1e34b9a0bc275e5c3c0ffb8348))


## [0.21.0](https://github.com/koharu-rs/koharu/compare/0.20.0..0.21.0) - 2025-12-19

### ⛰️  Features

- Multi lang llm translate - ([8961779](https://github.com/koharu-rs/koharu/commit/8961779084a31e979210e273987124b61a35c79b))

### ⚙️ Miscellaneous Tasks

- Bundled pack portable - ([7f43570](https://github.com/koharu-rs/koharu/commit/7f43570e75952e37fbb8d31349863a7a7064bfdd))
- Simplify bubble logic - ([43512d2](https://github.com/koharu-rs/koharu/commit/43512d2b38678133e6169501ce1616eb110741da))


## [0.20.0](https://github.com/koharu-rs/koharu/compare/0.19.0..0.20.0) - 2025-12-18

### ⛰️  Features

- Double click or CLI to open a file - ([e02db8f](https://github.com/koharu-rs/koharu/commit/e02db8f84728bde647cac3c70a0e21d04433d04c))


## [0.19.0](https://github.com/koharu-rs/koharu/compare/0.18.0..0.19.0) - 2025-12-18

### ⛰️  Features

- Operation bubble and cancellable batch - ([86a1d59](https://github.com/koharu-rs/koharu/commit/86a1d591090e106df73b9eded6f2fca52a6f06bb))
- Async update check - ([136f374](https://github.com/koharu-rs/koharu/commit/136f3748e60913e2d5933c880a51c42b1c4842be))
- .khr file association on windows - ([7e90a7d](https://github.com/koharu-rs/koharu/commit/7e90a7df35e4cb87de70c552d965610ff94399e3))
- Show version number - ([13bb974](https://github.com/koharu-rs/koharu/commit/13bb974340e58c832111d6fc70dd307103e32e21))
- Save and load projects - ([20fc1b9](https://github.com/koharu-rs/koharu/commit/20fc1b9aeaed38e7c0205d55240e56e257fddeee))
- Save and load projects - ([c5624ea](https://github.com/koharu-rs/koharu/commit/c5624eaa5d084583b2429748dde75e24497b9c34))
- Wysiwyg canvas - ([d9289a5](https://github.com/koharu-rs/koharu/commit/d9289a5b2c3012b2032d5c51651cefe1659124de))
- Add hunyuan to translate more languages - ([b0c2df7](https://github.com/koharu-rs/koharu/commit/b0c2df70e3792451e8f62f333fc8142e72ea6921))

### 🐛 Bug Fixes

- Add a textBlockSyncer to avoid block on mutex - ([fd59dea](https://github.com/koharu-rs/koharu/commit/fd59dea525d6643414a3b82afbf74163d24599f1))
- Cargo happy - ([2c89173](https://github.com/koharu-rs/koharu/commit/2c89173f642d9bcd88a6cc6899ec4713d8709907))
- Dom text layer bug - ([6e4e406](https://github.com/koharu-rs/koharu/commit/6e4e4061ec6236a94e956322814590c8e785af06))
- Stroke and font color normalization - ([d9f4e0a](https://github.com/koharu-rs/koharu/commit/d9f4e0a3ae0733ec38e07062c217ef7bd0bcc65c))

### 🚜 Refactor

- New renderer ([#99](https://github.com/koharu-rs/koharu/issues/99)) - ([2367f9b](https://github.com/koharu-rs/koharu/commit/2367f9bbbd1d50a8ee9b25c5c545a606841126cf))

### 📚 Documentation

- Change demo image - ([14f9c43](https://github.com/koharu-rs/koharu/commit/14f9c435e7b4275054339a0ee2173efe64fb06a4))

### ⚡ Performance

- Improve renderer performance - ([674654b](https://github.com/koharu-rs/koharu/commit/674654b7be147767c85c74f380e273433a6af5f9))

### ⚙️ Miscellaneous Tasks

- Use workspace Cargo - ([0425648](https://github.com/koharu-rs/koharu/commit/04256480968974abca7884fa7eb50bc42b610edd))
- Clippy happier - ([fd1b29c](https://github.com/koharu-rs/koharu/commit/fd1b29c0b49aec9624b0e542f5de776049541abd))


## [0.18.0](https://github.com/koharu-rs/koharu/compare/0.17.0..0.18.0) - 2025-12-16

### ⛰️  Features

- Renderer to clamp near black - ([c7b1be3](https://github.com/koharu-rs/koharu/commit/c7b1be35713e40ad85525ffd768db2c4b811806f))
- Auto choose color and stroke - ([7f3c281](https://github.com/koharu-rs/koharu/commit/7f3c28150849da69f7d7ef35ed8fad81512d529c))
- Font detection using yuzumaker.FontDetection - ([c3f09fb](https://github.com/koharu-rs/koharu/commit/c3f09fb59bb65a22698deb705b1297dca2313f35))
- Google fonts - ([9f23d62](https://github.com/koharu-rs/koharu/commit/9f23d62c9e998d12020ea4c72da4021f6720b0dc))
- Reimplement load tokenizer from gguf - ([dae7821](https://github.com/koharu-rs/koharu/commit/dae78211d014afeff0dc08fd958ac73acb6fa4d4))

### 🐛 Bug Fixes

- Remove stroke auto color, keep font auto color - ([2cb0a4e](https://github.com/koharu-rs/koharu/commit/2cb0a4ede31a8822b5e4bde492346d5a1c57e3b8))
- Add font detect test - ([a26e623](https://github.com/koharu-rs/koharu/commit/a26e623c85fa653c4131c1b40bd1e09e2ca852f6))
- Close bigger holes in ctd - ([2e079c4](https://github.com/koharu-rs/koharu/commit/2e079c4ea93ff592758bcb6f971c6f1f00136d56))
- Clippy happy - ([5ffa16c](https://github.com/koharu-rs/koharu/commit/5ffa16c4d12556459f5e8cafa4739beea3a59935))
- Windows to search system32 for DLLs too - ([23b5c01](https://github.com/koharu-rs/koharu/commit/23b5c01357dca7408504205da19a6dd92c31921c))
- Layout CJK and English at same time in horizontal view - ([0122d45](https://github.com/koharu-rs/koharu/commit/0122d453f0f4fc2cb8af548ccb4000979a0281dd))
- Avoid conflict with existing dlls on windows - ([be28837](https://github.com/koharu-rs/koharu/commit/be28837811df9eeacb9f6aa590cf0543261b0c5e))
- Llm test running locally - ([220ec14](https://github.com/koharu-rs/koharu/commit/220ec14702436c88aba77a02017ddeecb5de5d31))

### 🧪 Testing

- Add vertical CJK layout test - ([5401b5b](https://github.com/koharu-rs/koharu/commit/5401b5b02433115e928a77d6f28d9e9d7c2953f0))
- Add layout line wrapping test - ([e386370](https://github.com/koharu-rs/koharu/commit/e38637099c437bf959962f99af927e9f1f3404b2))
- Use google fonts - ([869d828](https://github.com/koharu-rs/koharu/commit/869d8288f1b91cf501bb57c20cbacfd8df108a4b))
- Fix tests - ([cfd7625](https://github.com/koharu-rs/koharu/commit/cfd762509a262666d590fabb9f5df7dea6ddddd8))

### ⚙️ Miscellaneous Tasks

- Remove unused import in koharu-renderer - ([92e8db6](https://github.com/koharu-rs/koharu/commit/92e8db64620421d43957f7ffd5f4831d7d3db926))
- Add layout tests - ([9070fe5](https://github.com/koharu-rs/koharu/commit/9070fe548bace04fa619028ed7b136aab2dd54d0))
- Remove unnessary conditional deps - ([7e02624](https://github.com/koharu-rs/koharu/commit/7e02624ec2ff247ea57b6f1b3d11aeabd8c630bf))
- Only create span when cache is not valid - ([210e1c8](https://github.com/koharu-rs/koharu/commit/210e1c8518dc024a250ecfc107669be0460e18a0))
- Make clippy happy - ([3928a51](https://github.com/koharu-rs/koharu/commit/3928a51b63cfaf4b1d3b11f92d12eeec44d45cc6))
- Remove reviewdog - ([4efdf54](https://github.com/koharu-rs/koharu/commit/4efdf54eb4791f68542318076681e3aa4b9beb35))
- Add ignore annotation for llm test requiring large model downloads - ([ffbc3d8](https://github.com/koharu-rs/koharu/commit/ffbc3d84078205ca92c58986092f7e797fc6f699))
- Update cargo clippy command to treat warnings as errors - ([3bb25b1](https://github.com/koharu-rs/koharu/commit/3bb25b14f3e0c4eab1299da105084ec036a0e9db))


## [0.17.0](https://github.com/koharu-rs/koharu/compare/0.16.0..0.17.0) - 2025-12-14

### ⛰️  Features

- Load tokenizer from gguf - ([3f5a285](https://github.com/koharu-rs/koharu/commit/3f5a28503165613c3c8fbacfc6029e024d2f3d12))

### 🐛 Bug Fixes

- Llm template should add generation prompt - ([b5cd3e3](https://github.com/koharu-rs/koharu/commit/b5cd3e3771c4d322c11608526d871bd9f21f7019))


## [0.16.0](https://github.com/koharu-rs/koharu/compare/0.15.0..0.16.0) - 2025-12-14

### ⛰️  Features

- Normalize font sizes - ([626d06b](https://github.com/koharu-rs/koharu/commit/626d06b1e3ce964e95a42511036b4804a85033d3))
- Predict word wrap in horizontal layout - ([d5b8a45](https://github.com/koharu-rs/koharu/commit/d5b8a452fbf1c9feb4782eda5bda12ef842c3401))
- Add cpu only mode - ([125da9f](https://github.com/koharu-rs/koharu/commit/125da9f46b847dc24264a26c59f28d229636d20e))

### 🐛 Bug Fixes

- Vntl incorrect template - ([dc4d69e](https://github.com/koharu-rs/koharu/commit/dc4d69efeaf0a6f4f4ff784026b66c45725542f7))
- Use cfg target_os in Cargo.toml - ([49b3e4e](https://github.com/koharu-rs/koharu/commit/49b3e4e2a046d40282ddf8af5fed77ec6ef8b281))
- Better compatiblity with windows conhost - ([6539560](https://github.com/koharu-rs/koharu/commit/65395601e27120178abf03d6dba0cff3efe83163))
- Unload llm on change - ([4e2fd99](https://github.com/koharu-rs/koharu/commit/4e2fd996b37e40285dcceb98e0da9ed525a8167a))
- Cpu only mode ctd and model sorting - ([b8a1e71](https://github.com/koharu-rs/koharu/commit/b8a1e71b1de8922aa95293f9c01a43c98acac7e0))

### ⚡ Performance

- Speed up metal fft - ([7308b92](https://github.com/koharu-rs/koharu/commit/7308b9294f68950cee6e638b4deb9db13f45943f))
- Batch inference for manga-ocr - ([6ce8023](https://github.com/koharu-rs/koharu/commit/6ce8023515fbb86061348e54af3557039f03c7cb))

### 🧪 Testing

- Add ctd integration test - ([9f85981](https://github.com/koharu-rs/koharu/commit/9f859815868b642c11f0421742bdf5fc96d52bfc))

### ⚙️ Miscellaneous Tasks

- Add cargo tests ([#89](https://github.com/koharu-rs/koharu/issues/89)) - ([693bf07](https://github.com/koharu-rs/koharu/commit/693bf07f9a740d86c141c810dc9b5c3e67c68ed8))
- Add reviewdog - ([f5e5f0c](https://github.com/koharu-rs/koharu/commit/f5e5f0c1f0ef4fda449cbba36da63fa4ef83f650))
- Add husky & lint-staged - ([9f30041](https://github.com/koharu-rs/koharu/commit/9f30041295d8229dede2aed55a128c9f877cb847))
- Make clippy happy again - ([496bb37](https://github.com/koharu-rs/koharu/commit/496bb37154c5d604590f124bd88c97bfa9f79d79))
- Fix linter - ([0ad3375](https://github.com/koharu-rs/koharu/commit/0ad33751f4f5b4f90312da23caa792db3e9ea60d))
- Make clippy happy - ([259e52a](https://github.com/koharu-rs/koharu/commit/259e52acdea3a9bc9b427c09ce2e2a8996a89454))
- Add linter - ([eee6250](https://github.com/koharu-rs/koharu/commit/eee6250de2cf2cf8ef30257d5b3dde639e1f9f0e))
- Remove unused deps - ([511e080](https://github.com/koharu-rs/koharu/commit/511e080a8d75ad66485cc17a5168e0693860bfe8))
- Switch to cudarc 0.18.2 - ([32da9bb](https://github.com/koharu-rs/koharu/commit/32da9bb48e22f50ba7dbb8f4cd73315a36efc015))


## [0.15.0](https://github.com/koharu-rs/koharu/compare/0.14.5..0.15.0) - 2025-12-11

### 🐛 Bug Fixes

- Add top level tracing and mark others as debug - ([a5eb2d0](https://github.com/koharu-rs/koharu/commit/a5eb2d0461351a869a1fdca7689063892835d3a6))

### 🚜 Refactor

- Move fft to seperate files - ([b0d3986](https://github.com/koharu-rs/koharu/commit/b0d3986ba3884ed3defaae99e17eb1213a3db4d3))

### ⚡ Performance

- Cache cufft plan - ([4aa74ba](https://github.com/koharu-rs/koharu/commit/4aa74ba78f2eba9ba04a5376fdc9f6fcb2823143))

### ⚙️ Miscellaneous Tasks

- Format lama code with LF - ([a399564](https://github.com/koharu-rs/koharu/commit/a399564f297da75a0072893b7e5617c14ea05387))


## [0.14.5](https://github.com/koharu-rs/koharu/compare/0.14.3..0.14.5) - 2025-12-11

### 🐛 Bug Fixes

- Sakura-1.5b-qwen2.5-v1.0 incorrect eos_token_id - ([e10c8ec](https://github.com/koharu-rs/koharu/commit/e10c8eca933c167cac457519698346ddcad45c00))
- Remove unused deserialize - ([8d4a00e](https://github.com/koharu-rs/koharu/commit/8d4a00e8c1805e99f0681cf408adfb571adaffc4))
- Format code and llm example - ([f9ac15c](https://github.com/koharu-rs/koharu/commit/f9ac15c030af2044fae9077f58b2b934687032c5))
- Use templates from gguf file - ([b78f870](https://github.com/koharu-rs/koharu/commit/b78f8703272e83019abf83cfe0f3b2e144774a22))

### 📚 Documentation

- Update changelog - ([ee60acf](https://github.com/koharu-rs/koharu/commit/ee60acf993c0b6d2538028fc73c1cec239658761))

### ⚙️ Miscellaneous Tasks

- Remove unwrap - ([2be6f04](https://github.com/koharu-rs/koharu/commit/2be6f044bdcaf03fa0d78298f9497750a335b8e7))


## [0.14.3](https://github.com/koharu-rs/koharu/compare/0.14.2..0.14.3) - 2025-12-11

### 🐛 Bug Fixes

- Support non-nvidia gpu on windows - ([f942a38](https://github.com/koharu-rs/koharu/commit/f942a3834b0083c85c2ba794d273ed7c7b42a384))

### 📚 Documentation

- Add changelog.md - ([01c8788](https://github.com/koharu-rs/koharu/commit/01c87887cdacd6204efd9bde1e991fb12dc601e9))

### ⚙️ Miscellaneous Tasks

- Do not publish crates - ([80c5ce3](https://github.com/koharu-rs/koharu/commit/80c5ce36c83e0d20fe0d5125360f8293f0c3358e))
- Add release-please - ([f5f6d03](https://github.com/koharu-rs/koharu/commit/f5f6d0302864ceb2d250fe73352683c9e467db90))

### Release

- 0.14.3 - ([71dbaf4](https://github.com/koharu-rs/koharu/commit/71dbaf474bd9b8951ef00789ec417448dd6168b1))


## [0.14.2](https://github.com/koharu-rs/koharu/compare/0.14.1..0.14.2) - 2025-12-11

### 🐛 Bug Fixes

- Fix: llm markers
close #65 - ([f3039b6](https://github.com/koharu-rs/koharu/commit/f3039b64ff45f18b2cc550c2a8737d8f56f4efb8))

### Release

- 0.14.2 - ([60778d6](https://github.com/koharu-rs/koharu/commit/60778d60db89e9e8b8624fcf828720478e44c533))


## [0.14.1](https://github.com/koharu-rs/koharu/compare/0.14.0..0.14.1) - 2025-12-10

### 🐛 Bug Fixes

- Macos to use local data dir - ([1c910d3](https://github.com/koharu-rs/koharu/commit/1c910d31693c8007e558b729909db0281bcfa324))

### Release

- 0.14.1 - ([3396619](https://github.com/koharu-rs/koharu/commit/3396619fde1acf62e6644392fc0b75ce3fd169d8))


## [0.14.0](https://github.com/koharu-rs/koharu/compare/0.13.1..0.14.0) - 2025-12-09

### ⛰️  Features

- Translate single textblock - ([f1bbd23](https://github.com/koharu-rs/koharu/commit/f1bbd2313bffc28a34c841e90e5820a349b31537))
- Delete and backspace key can delete text blocks - ([51eb001](https://github.com/koharu-rs/koharu/commit/51eb0015cb6305b2a15724c142b9f5f13bc72edf))
- I18n - ([06879d6](https://github.com/koharu-rs/koharu/commit/06879d607fa07ae9ae171d67dafb6aff7bfab6dc))

### 🐛 Bug Fixes

- Unable to add textblocks - ([f52c608](https://github.com/koharu-rs/koharu/commit/f52c6089cad5dd59b0f06d73fac6660272bb578f))
- Llm idle i18n - ([ff98cd9](https://github.com/koharu-rs/koharu/commit/ff98cd928943fa330d2484da801fcbb37848ca5b))
- Menbar overlay - ([5a1b50f](https://github.com/koharu-rs/koharu/commit/5a1b50f644957ebbe81f7c3973e9dddbcc11231f))
- Try to make lfm2 output better results - ([90dd883](https://github.com/koharu-rs/koharu/commit/90dd883cf85a4c676644b4f65f5936d85d0ac21b))
- Set progress message is not awaited - ([9a9949f](https://github.com/koharu-rs/koharu/commit/9a9949f3c0dc79e8e2e024d7ce4ccf2e3a90eca9))

### 📚 Documentation

- Fix broken words - ([4f30a52](https://github.com/koharu-rs/koharu/commit/4f30a524323ccd0feedc9fe4e47d62d1f165b3e5))

### ⚙️ Miscellaneous Tasks

- Add version to cli - ([907e996](https://github.com/koharu-rs/koharu/commit/907e996bfe68a23b15df8b682795b3ce82faf849))

### Release

- 0.14.0 - ([8ce0bc7](https://github.com/koharu-rs/koharu/commit/8ce0bc7669a4a71c6204f8aa976217e7235126f8))


## [0.13.1](https://github.com/koharu-rs/koharu/compare/0.13.0..0.13.1) - 2025-12-09

### ⛰️  Features

- Lfm2 iterration 2 - ([69b15b5](https://github.com/koharu-rs/koharu/commit/69b15b5e00c398db29a5df9f5145d9e9a2433d18))

### 🐛 Bug Fixes

- Prefer to use huggingface.co since hf-mirror is unstable - ([c4424ee](https://github.com/koharu-rs/koharu/commit/c4424eef03f4ab2a82dcb6e2b9d3d8b935258fa3))
- Lfm2 remove extra debugging code - ([7d907a8](https://github.com/koharu-rs/koharu/commit/7d907a8f5bba68ca508f7b4d9b5603b23c634d21))

### 📚 Documentation

- Update readme on llm - ([9dacce7](https://github.com/koharu-rs/koharu/commit/9dacce759dae94be21380b332762538c72f05b5e))

### ⚙️ Miscellaneous Tasks

- Cleanup tracing logs - ([5635f4f](https://github.com/koharu-rs/koharu/commit/5635f4f0b01bd170f50ca237b48620e7dc0a3bd4))

### Release

- 0.13.1 - ([5b822cf](https://github.com/koharu-rs/koharu/commit/5b822cf82ac73f3fc6e678d5d4399333df53b578))


## [0.13.0](https://github.com/koharu-rs/koharu/compare/0.12.1..0.13.0) - 2025-12-08

### ⛰️  Features

- Add experimental smaller model support - ([c929422](https://github.com/koharu-rs/koharu/commit/c929422e79bb7d844dfd34940903ec4203452a66))
- Lfm2 iterration 1 - ([b3237f1](https://github.com/koharu-rs/koharu/commit/b3237f1315f8c3d0ad9b1c5bdc3c05ee381ffd30))
- Add task bar progress bar - ([46158ea](https://github.com/koharu-rs/koharu/commit/46158eaa2370badd6ec1b47c6da5c079a883605d))

### 🐛 Bug Fixes

- Correct llm loading status - ([684c168](https://github.com/koharu-rs/koharu/commit/684c16866e2ceddae528015d023c406054c3fe68))
- Sort llm - ([6fd3db6](https://github.com/koharu-rs/koharu/commit/6fd3db63f415b5ca03065989db4c1ec3bd22f7bb))

### Release

- 0.13.0 - ([1368a80](https://github.com/koharu-rs/koharu/commit/1368a8007784de2f6308eb37e8c00b2ed9014a57))


## [0.12.1](https://github.com/koharu-rs/koharu/compare/0.12.0..0.12.1) - 2025-12-07

### ⛰️  Features

- Add cli mode & download only - ([bfce290](https://github.com/koharu-rs/koharu/commit/bfce2902417630be8dabd1e4e5555e0ea1b9613f))

### 🐛 Bug Fixes

- Hf hub download also uses cache location - ([78f84cf](https://github.com/koharu-rs/koharu/commit/78f84cf60085d9e8b9a53bde569235fd8d7ae10b))

### 🚜 Refactor

- Re-introduce koharu-core - ([db71712](https://github.com/koharu-rs/koharu/commit/db7171258096ec0dc7e454fb7f8347147a4bc79a))

### ⚙️ Miscellaneous Tasks

- Skip preloading llm models - ([fc53896](https://github.com/koharu-rs/koharu/commit/fc53896126b2df90988c77ee9d10ab103d11a37c))
- Add tracing logs for download - ([b778fd1](https://github.com/koharu-rs/koharu/commit/b778fd198db642b13141a65c1ee490b68a6975b7))
- Install nvcc and upload artifacts - ([e4388b3](https://github.com/koharu-rs/koharu/commit/e4388b3140d349cece103a3a28bc707de72bb718))
- Speed up cuda install - ([01e2a42](https://github.com/koharu-rs/koharu/commit/01e2a427a8c3fba3f1f2d703f8092bf1f7b0e2f5))

### Cd

- Upload correct artifact - ([68970b0](https://github.com/koharu-rs/koharu/commit/68970b01bc533f24b589478960237004413f4bf2))
- Reduce bundled file size - ([74781fe](https://github.com/koharu-rs/koharu/commit/74781fe9cba93a09c619f1cfc51d33c70fceba05))
- Create bundle archive for windows - ([5b3128e](https://github.com/koharu-rs/koharu/commit/5b3128e676b318797cff2cd01ff07235f27e7b6c))

### Release

- 0.12.1 - ([96e6059](https://github.com/koharu-rs/koharu/commit/96e60593866099c9c45eb8395d42f260a67c6b41))


## [0.12.0](https://github.com/koharu-rs/koharu/compare/0.11.0..0.12.0) - 2025-12-06

### ⛰️  Features

- Add hf mirror - ([9ddaa43](https://github.com/koharu-rs/koharu/commit/9ddaa43414ff75bd633e6d209a9b425ca0a98566))
- Select pypi mirror - ([8ca464d](https://github.com/koharu-rs/koharu/commit/8ca464dc42d98adbfb5cd14a283158bcb44f6915))

### 🐛 Bug Fixes

- Portable mode libs and models location - ([e82d5bc](https://github.com/koharu-rs/koharu/commit/e82d5bc9464684e7192e35781fa5d9c18ea0525d))
- Refine url - ([58fdc69](https://github.com/koharu-rs/koharu/commit/58fdc6904e117496177a855087ea7aae52b215e1))
- Pypi mirror - ([4cb21fd](https://github.com/koharu-rs/koharu/commit/4cb21fd4bea54d16566c407da1e3d84e28457aeb))

### ⚙️ Miscellaneous Tasks

- Format code - ([02ce9c8](https://github.com/koharu-rs/koharu/commit/02ce9c861835666c28601aa6b77a44b28c049748))
- Format code - ([fea5b3d](https://github.com/koharu-rs/koharu/commit/fea5b3d9474d728db980f08fae2d7dabbbfff1b1))

### Release

- 0.12.0 - ([369cd64](https://github.com/koharu-rs/koharu/commit/369cd64a4c252b994dd8f6bb8c26c377c1cc79a1))


## [0.11.0](https://github.com/koharu-rs/koharu/compare/0.10.1..0.11.0) - 2025-12-04

### ⛰️  Features

- Use local directory if portal - ([25dddf7](https://github.com/koharu-rs/koharu/commit/25dddf7e3c9ca582d686fc1aec4eb5a0dd41e970))
- Auto detect font family order - ([2fc5af2](https://github.com/koharu-rs/koharu/commit/2fc5af27a6132656d5f767eb09bb38eab8b95569))
- Export and batch export - ([a788ed3](https://github.com/koharu-rs/koharu/commit/a788ed38836b6642d8576737c31aa59d1280bf70))
- Only inpaint mask inside boxes - ([8d1cacf](https://github.com/koharu-rs/koharu/commit/8d1cacf3c678e0aef18a04f2ee0755dfb30b5cae))
- Batch show status - ([a716b65](https://github.com/koharu-rs/koharu/commit/a716b65757b37829a2fb94833a2211ba97e9d067))
- Batch process - ([cc22d75](https://github.com/koharu-rs/koharu/commit/cc22d752aadc92cd6bd43f55113f69c2d9aa21fa))
- Metal fft - ([cf44d26](https://github.com/koharu-rs/koharu/commit/cf44d2684b71a8c072d19e622ce67394950c24f9))
- Use cufft - ([20a3080](https://github.com/koharu-rs/koharu/commit/20a3080e4fed760a03d97e35bfac686c63905f4a))
- Add cudnn support - ([ef89ee8](https://github.com/koharu-rs/koharu/commit/ef89ee8e0375b19cc5039a37a63932c410eb5c05))

### 🐛 Bug Fixes

- Wheels matching on Linux ([#49](https://github.com/koharu-rs/koharu/issues/49)) - ([9dbb400](https://github.com/koharu-rs/koharu/commit/9dbb4009f0b2ccc7ac4f2441e75d16c24a44dbc2))
- Center compensation should always be positive - ([8a65ee7](https://github.com/koharu-rs/koharu/commit/8a65ee78afcd04ec8c4348ed9a4c83a6621f3201))
- More merge errors - ([0857670](https://github.com/koharu-rs/koharu/commit/08576708c2eff117657ee7b2e2dfb4fe2b5a5423))
- Merge error - ([1e28f7d](https://github.com/koharu-rs/koharu/commit/1e28f7d17136fd12ca1cca76d05ef8129118e494))
- Center compensation bug in vertical layout - ([c47efae](https://github.com/koharu-rs/koharu/commit/c47efae558a6c73df77c51d36d19478cbeb71afc))
- Resize, batch and UI fixes - ([35e4f90](https://github.com/koharu-rs/koharu/commit/35e4f903c5e2a5961432860591f2a548cd1e4c62))
- Only compensate on selected axis - ([8398630](https://github.com/koharu-rs/koharu/commit/83986300c58c71b142fdb9d5239882f914efeb21))
- Horizontal latin min fontsize limit - ([7e5e016](https://github.com/koharu-rs/koharu/commit/7e5e01653dede0c8416f5a4d246ea6b244788538))
- Break on whitespace and centeralize text - ([5306828](https://github.com/koharu-rs/koharu/commit/530682879b6d14cab37d0a5662d6969e367e7f94))
- Latin should not in vertical - ([13bb6d3](https://github.com/koharu-rs/koharu/commit/13bb6d356fe8876fb43ea6d6af98693b083db886))
- Add macOS default fonts and skip non normal variants - ([15177dc](https://github.com/koharu-rs/koharu/commit/15177dc7830b3b377e43f05c7f111597104e3cfe))
- Vertical magic spacing - ([400e822](https://github.com/koharu-rs/koharu/commit/400e822893ea5fe80644d3318c6b086c33124495))
- Adjust vertical layout for small glyphs - ([012c217](https://github.com/koharu-rs/koharu/commit/012c217c1b2a244411441767057d912ccbcf1924))
- Latin character in CJK text vertical hack - ([a30d525](https://github.com/koharu-rs/koharu/commit/a30d525ee36a583bf519fe7d2aa7be5551cb9125))
- Order font by preferred language - ([8ec4d94](https://github.com/koharu-rs/koharu/commit/8ec4d9470758928abb71153e91085288450ac6b0))
- Font size bisearch fix - ([4d6d0ce](https://github.com/koharu-rs/koharu/commit/4d6d0cefd1b3297a6cd0a32e92b234f8fd65eba8))
- UI fixes - ([d6b13a7](https://github.com/koharu-rs/koharu/commit/d6b13a74d0d82db208b121c50c032d2b18e8a0b8))
- Fallback to Yahei - ([f1ab8a2](https://github.com/koharu-rs/koharu/commit/f1ab8a2506d8183f5a41469b197e888b3b22f3dc))
- Order llm by locale - ([005829c](https://github.com/koharu-rs/koharu/commit/005829c84091f19751729db12614108db5583476))
- Use gpu to postprocess - ([5f870ec](https://github.com/koharu-rs/koharu/commit/5f870ecf7eb544cef5fe572c4dc82405235952dc))
- Fft precision - ([236f923](https://github.com/koharu-rs/koharu/commit/236f9231e9e22f646de762439a5702632e5afabe))
- Mask - ([2b423b9](https://github.com/koharu-rs/koharu/commit/2b423b97cf90ae17fbc3b647e0e5bfda99bc6594))
- Metal - ([d47b91c](https://github.com/koharu-rs/koharu/commit/d47b91cef3df1a92e76808467d0df87f9997bdae))
- Macos - ([45ca74b](https://github.com/koharu-rs/koharu/commit/45ca74bf4c72c2ae52966b330cb2f8f57dc393e7))
- Lama - ([0b49d62](https://github.com/koharu-rs/koharu/commit/0b49d622a528092322bd4bcf19d6f65cc0a5495e))
- Preprocess and postprocess - ([7f10dea](https://github.com/koharu-rs/koharu/commit/7f10dea8ad1ab6a55d461b145c1cbabae0aa1805))

### 🚜 Refactor

- Use koharu-ml - ([072273f](https://github.com/koharu-rs/koharu/commit/072273f89fa923543cf22e40a025661965237272))
- Rename models to ml - ([220750b](https://github.com/koharu-rs/koharu/commit/220750b3e472fcbe8cc9270d836cea0d69daf759))
- Replace onnx with candle - ([55ffc94](https://github.com/koharu-rs/koharu/commit/55ffc94323ecef33db8a8c66a5b99b2a84aae9a3))
- Replace onnx with candle - ([a594bfc](https://github.com/koharu-rs/koharu/commit/a594bfc09a6a8c23764a3c99bbb821f5c0832dae))
- Correct ctd behavor - ([324cd22](https://github.com/koharu-rs/koharu/commit/324cd22adb598c2869cc1db2ff9bf1ae522f1cd6))
- Replace onnx with candle - ([8a07783](https://github.com/koharu-rs/koharu/commit/8a07783dc4b210fd6a73ca8ed86826a0318ea46a))
- Dbnet & unet - ([8ceafdb](https://github.com/koharu-rs/koharu/commit/8ceafdb0e302e34631a4320398038f54c6811af0))
- Yolov5 - ([ba49323](https://github.com/koharu-rs/koharu/commit/ba493239b0b59534f265a2d2e36423c728c1e470))
- Use pure FFT for lama - ([7803ba8](https://github.com/koharu-rs/koharu/commit/7803ba8a2d8e6c662d2926691f3d7833883f2c3d))

### 📚 Documentation

- Add horizontal rule in README for better section separation - ([55fab1d](https://github.com/koharu-rs/koharu/commit/55fab1dd4ee0bdeeda831e17e558161ff68876e1))
- Update license section in README files - ([e2cdf7a](https://github.com/koharu-rs/koharu/commit/e2cdf7a185de8dcdff996cb0f85cf062af1f3c9c))

### ⚡ Performance

- Add instruments - ([017f2a7](https://github.com/koharu-rs/koharu/commit/017f2a75a7747731ab495a8b5da9e6b99eab7cce))
- Use GPU to resize - ([bb98ff4](https://github.com/koharu-rs/koharu/commit/bb98ff42f735bb551cbf6b72bf043ab5c4706821))
- Use GPU to resize and postprocess - ([8a9a1f3](https://github.com/koharu-rs/koharu/commit/8a9a1f3044a15f846f4ac5bd4fbbe93e340012cd))
- Avoid cpu offload - ([da1097d](https://github.com/koharu-rs/koharu/commit/da1097d537af64ef7317e12b34c67b1760a1466a))

### ⚙️ Miscellaneous Tasks

- Update deps - ([a65f8fe](https://github.com/koharu-rs/koharu/commit/a65f8feb795d3d98f12d3201f42823dcfbcffadb))
- Move sys-locale to workspace deps - ([d5ab31c](https://github.com/koharu-rs/koharu/commit/d5ab31c464d99b41641f7f77d42709e38d86ab77))
- Code style - ([93c86f1](https://github.com/koharu-rs/koharu/commit/93c86f112b5fda4c0d100e44941c24c47b5dfb9b))
- Remove unused deps - ([0a4e185](https://github.com/koharu-rs/koharu/commit/0a4e185d8a2e523def1e02ad5ec6e86d54b10621))

### Release

- 0.11.0 - ([ab47678](https://github.com/koharu-rs/koharu/commit/ab476788fd0a707df68748628e602fb52688af26))


## [0.10.1](https://github.com/koharu-rs/koharu/compare/0.10.0..0.10.1) - 2025-11-25

### 🐛 Bug Fixes

- Use onnxruntime-gpu when cuda enabled - ([c4dcdfb](https://github.com/koharu-rs/koharu/commit/c4dcdfba64a585d5724eb2efbbb4edf0aed16d22))

### Release

- 0.10.1 - ([a33903e](https://github.com/koharu-rs/koharu/commit/a33903e80648f5e8b952f0d3e0a50a0bdd42c01c))


## [0.10.0](https://github.com/koharu-rs/koharu/compare/0.9.13..0.10.0) - 2025-11-25

### ⛰️  Features

- Dynamic loading cuda & onnxruntime - ([02e419e](https://github.com/koharu-rs/koharu/commit/02e419ef1a2f333c119564010355754aad85cc36))

### 📚 Documentation

- Support metal - ([54cf8dc](https://github.com/koharu-rs/koharu/commit/54cf8dc33f154842685839a93f4ecbb9fde148c4))

### ⚙️ Miscellaneous Tasks

- Move update to init phase - ([2cbff02](https://github.com/koharu-rs/koharu/commit/2cbff02162a77c33ca7d43ba10803e36dd181f70))
- Remove profile - ([526d6c8](https://github.com/koharu-rs/koharu/commit/526d6c8620e179b934af24238b68f04f4c578cd8))

### Release

- 0.10.0 - ([d6c3814](https://github.com/koharu-rs/koharu/commit/d6c3814df48cc8fb9354beb396b90c01f592d8c5))


## [0.9.13](https://github.com/koharu-rs/koharu/compare/0.9.12..0.9.13) - 2025-11-24

### 🐛 Bug Fixes

- Release - ([e7e6d3f](https://github.com/koharu-rs/koharu/commit/e7e6d3fe93dc36905204ea06e4ce886cf10f22de))

### Release

- 0.9.13 - ([1855fd7](https://github.com/koharu-rs/koharu/commit/1855fd70d53bc38b6176212058a9d2f21c7c4abe))


## [0.9.12](https://github.com/koharu-rs/koharu/compare/0.9.11..0.9.12) - 2025-11-24

### 🐛 Bug Fixes

- Ci - ([959b0bd](https://github.com/koharu-rs/koharu/commit/959b0bd195d64c172005848aa3045c4e22ac9cba))

### Release

- 0.9.12 - ([e43af4c](https://github.com/koharu-rs/koharu/commit/e43af4c895d7bb16a8fbf5b1e98c5ee846770b91))


## [0.9.11](https://github.com/koharu-rs/koharu/compare/0.9.10..0.9.11) - 2025-11-24

### ⛰️  Features

- Support coreml - ([8816e0c](https://github.com/koharu-rs/koharu/commit/8816e0cbe5d09f091b2be5e7bb3750091570f1ca))
- Support metal - ([d4d67a2](https://github.com/koharu-rs/koharu/commit/d4d67a2f06e78496e453b15c2df8a8e346d3bad2))

### 🐛 Bug Fixes

- Release - ([65787ba](https://github.com/koharu-rs/koharu/commit/65787baedb3f167c0abe982e908ee4f76866d91f))
- Remove suffix - ([ee4a5ba](https://github.com/koharu-rs/koharu/commit/ee4a5badb8e8aa3ca57bd14fb79d67b61d4e0173))
- Support macos - ([d7af7ac](https://github.com/koharu-rs/koharu/commit/d7af7ac4dcc7cdbe44a602757dedc40852172552))
- Update macOS platform tag to universal2 - ([148bfa1](https://github.com/koharu-rs/koharu/commit/148bfa1b0e0a77844e94979e35f6bba26939a0eb))
- Support macos - ([dd91bb7](https://github.com/koharu-rs/koharu/commit/dd91bb76eb00caf76d373d877a0d3c7958721954))

### 📚 Documentation

- Fix link - ([4f70012](https://github.com/koharu-rs/koharu/commit/4f70012b704c435e90a82985d35c082e6ca28ca0))
- Sponsorship - ([e9d56ce](https://github.com/koharu-rs/koharu/commit/e9d56ce66b99c280da00450eac9839e95b9a4c1f))

### ⚙️ Miscellaneous Tasks

- *(ci)* Setup macos - ([5363dc3](https://github.com/koharu-rs/koharu/commit/5363dc3a414060aa126ca808cd2163a3c2967414))

### Release

- 0.9.11 - ([e1c0f09](https://github.com/koharu-rs/koharu/commit/e1c0f094ad1fcb069da45b5154f3f6637d75bd5f))


## [0.9.10](https://github.com/koharu-rs/koharu/compare/0.9.9..0.9.10) - 2025-11-23

### Release

- 0.9.10 - ([4345900](https://github.com/koharu-rs/koharu/commit/43459002f3c14f6aa4917b75cf7b2ff1579ec4ab))


## [0.9.9](https://github.com/koharu-rs/koharu/compare/0.9.8..0.9.9) - 2025-11-23

### ⛰️  Features

- Auto-update - ([38ab31a](https://github.com/koharu-rs/koharu/commit/38ab31a4d83ddcbdb246709797a055039429337f))

### Release

- 0.9.9 - ([89798c2](https://github.com/koharu-rs/koharu/commit/89798c228976cde5456ba22bb5082379e09efd25))
- 0.9.9-alpha.2 (auto-update test) - ([b29d2e8](https://github.com/koharu-rs/koharu/commit/b29d2e868750b45e34c5f5615341f0d11d82d127))
- 0.9.9-alpha.1 - ([d4275c9](https://github.com/koharu-rs/koharu/commit/d4275c9121a6bc7a1d75432e0ca8617cade70512))


## [0.9.8](https://github.com/koharu-rs/koharu/compare/0.9.7..0.9.8) - 2025-11-23

### Release

- 0.9.8 - ([bf42787](https://github.com/koharu-rs/koharu/commit/bf42787aae76f0db4a0c8a7a10711a91bb9186d2))
- 0.9.8-alpha.1 - ([42870fd](https://github.com/koharu-rs/koharu/commit/42870fd2aec837ae064796a68ce1363067c995d7))


## [0.9.7](https://github.com/koharu-rs/koharu/compare/0.9.6..0.9.7) - 2025-11-23

### Release

- 0.9.7 - ([2fe9a62](https://github.com/koharu-rs/koharu/commit/2fe9a620752569ed5a26ed41d3dcf296d08326b7))


## [0.9.6](https://github.com/koharu-rs/koharu/compare/0.9.5..0.9.6) - 2025-11-22

### Release

- 0.9.6 - ([00f048c](https://github.com/koharu-rs/koharu/commit/00f048c8d6155b738037a29c6ef1ee27cfd57e5a))


## [0.9.5](https://github.com/koharu-rs/koharu/compare/0.9.4..0.9.5) - 2025-11-22

### Release

- 0.9.5 - ([1dc8515](https://github.com/koharu-rs/koharu/commit/1dc85157de618f10c68d704c0fd7d2d34a65cb52))


## [0.9.4](https://github.com/koharu-rs/koharu/compare/0.9.3..0.9.4) - 2025-11-22

### 🐛 Bug Fixes

- Bundle cuda at build time - ([dcee290](https://github.com/koharu-rs/koharu/commit/dcee290fe5f8ecdf8a6a02383d77f42195569d4b))

### Release

- 0.9.4 - ([42c1951](https://github.com/koharu-rs/koharu/commit/42c1951058167a341b4f09337a7c6bbe030d8a1b))


## [0.9.3](https://github.com/koharu-rs/koharu/compare/0.9.2..0.9.3) - 2025-11-22

### ⛰️  Features

- Support English text rendering - ([dd25eb5](https://github.com/koharu-rs/koharu/commit/dd25eb55918ae1a87aa16fd161db01046faf13ed))

### 📚 Documentation

- Add screenshots - ([0c9fc3c](https://github.com/koharu-rs/koharu/commit/0c9fc3cd878bb1570018f998337527976a81d153))
- Update README for clarity and consistency in feature descriptions - ([6c8dec2](https://github.com/koharu-rs/koharu/commit/6c8dec2134dc55f9598f2a58f2eebda08c09f79b))

### ⚙️ Miscellaneous Tasks

- UX improvements - ([0e19d01](https://github.com/koharu-rs/koharu/commit/0e19d01e133e4cf51bd5c8c98039f950f5b8df12))

### Release

- 0.9.3 - ([565dd8a](https://github.com/koharu-rs/koharu/commit/565dd8a277262e73077765009a63fb2a4c83e35e))


## [0.9.2](https://github.com/koharu-rs/koharu/compare/0.9.1..0.9.2) - 2025-11-22

### 🐛 Bug Fixes

- Add --no-verify flag to cargo publish command in workflow - ([28286db](https://github.com/koharu-rs/koharu/commit/28286db594c7efd088d070216d83aa0b1ad32832))

### Release

- 0.9.2 - ([93dd4ee](https://github.com/koharu-rs/koharu/commit/93dd4eeda1d3befecb519af48e12b4fce0764ec6))


## [0.9.1](https://github.com/koharu-rs/koharu/compare/0.9.0..0.9.1) - 2025-11-22

### ⛰️  Features

- Auto-prompt - ([af2fd81](https://github.com/koharu-rs/koharu/commit/af2fd81f29715e5c6bf5b9e93c023fee3fd89a00))
- Support SakuraLLM/Sakura-GalTransl-7B-v3.7 - ([f935bd3](https://github.com/koharu-rs/koharu/commit/f935bd3e042a5b0673cac44de7f0c356f81db7e0))
- Add vntl-llama3-8b-v2 support & remove Gemma3 and Qwen2.5 - ([6a04db0](https://github.com/koharu-rs/koharu/commit/6a04db0767505c882e2a83544d8d7e468434adc1))

### 🐛 Bug Fixes

- Correct enum variant casing for SakuraGalTransl7Bv3_7 - ([edebb2c](https://github.com/koharu-rs/koharu/commit/edebb2ca02ed09bdcc6e316447041705d02f6115))

### 🚜 Refactor

- Remove system prompt handling from LlmControls and store - ([773362a](https://github.com/koharu-rs/koharu/commit/773362aea20d7829b5337ec2c72b3220e0c7900b))

### 📚 Documentation

- Update README - ([27280fc](https://github.com/koharu-rs/koharu/commit/27280fc7e73be8bd2e24d13630bc9115b139dcde))

### ⚙️ Miscellaneous Tasks

- *(ci)* Add publish workflow - ([6b87cab](https://github.com/koharu-rs/koharu/commit/6b87cabbfa1c1a117cc89f19a96e63b23c68c794))
- Add Arial font to default font families in TextStyle - ([7eb7f25](https://github.com/koharu-rs/koharu/commit/7eb7f250717f4650e53a5477d104999d86638845))
- Publish to crates.io - ([b2310ae](https://github.com/koharu-rs/koharu/commit/b2310ae7b25b20f363b476e285631b386b7c79ad))

### Release

- 0.9.1 - ([b9b06d5](https://github.com/koharu-rs/koharu/commit/b9b06d5da49f31fbe54c902523adae6a56cfa1d6))


## [0.9.0](https://github.com/koharu-rs/koharu/compare/0.8.0..0.9.0) - 2025-11-21

### ⛰️  Features

- Auto font size - ([a8f2a08](https://github.com/koharu-rs/koharu/commit/a8f2a08bcbac3b45fbf545121cd5754a779b4f56))
- Initial implement for text redering on ui - ([475dc8f](https://github.com/koharu-rs/koharu/commit/475dc8f9ce32ba0831989f8a200324f6d8b8dad7))
- Text rendering via Rust - ([9593057](https://github.com/koharu-rs/koharu/commit/95930575becb26323b747df2ee6262bccfbe514d))
- Add vertical layout features for text shaping - ([48e7ede](https://github.com/koharu-rs/koharu/commit/48e7edeb137ffbad0cb65c1ca595faf58a4d913e))
- Initial impl for text block renderer - ([ed8b7ac](https://github.com/koharu-rs/koharu/commit/ed8b7ac1978eb9811a948ad599a088b4da03a781))

### 🐛 Bug Fixes

- Typo - ([61a09ec](https://github.com/koharu-rs/koharu/commit/61a09ecdf25f7be63812462daaa2c5502b390639))

### 🚜 Refactor

- Reorganize font module exports and add query method - ([702efa5](https://github.com/koharu-rs/koharu/commit/702efa51586cb4578407e39ba25d1de006db4b5d))
- Reorganize module imports and update dylib function calls - ([8975be9](https://github.com/koharu-rs/koharu/commit/8975be92bb9f9ebef12f0d9a7d3e9b77dae0a09d))
- Rewrite renderer - ([0cba676](https://github.com/koharu-rs/koharu/commit/0cba676dd72577d6bb01fe087233019a5727900f))
- Better stucture - ([cbb619c](https://github.com/koharu-rs/koharu/commit/cbb619ceb4c5369dcb9a45d0d82aa81ac5264e18))
- Remove unnecessary componetns - ([7f3ee84](https://github.com/koharu-rs/koharu/commit/7f3ee8442d3a5bdfd50e4c9b228615621e7df06b))

### 🧪 Testing

- Place text to corner - ([f115da9](https://github.com/koharu-rs/koharu/commit/f115da9b886a4d681bc085836ffdc109eef2a157))
- Add vertical text rendering - ([8cd59d8](https://github.com/koharu-rs/koharu/commit/8cd59d88b274d0828cfacce802f022c3acb7d4ff))
- Add test to ensure vertical feature works - ([ae8bdd9](https://github.com/koharu-rs/koharu/commit/ae8bdd9999bb15537052b26afc2c88e65636b209))

### ⚙️ Miscellaneous Tasks

- *(scripts)* Auto-detect cuda - ([45377d2](https://github.com/koharu-rs/koharu/commit/45377d22064825963ee81f9b443cf149ebdde980))
- Add cargo script to package.json - ([ac39540](https://github.com/koharu-rs/koharu/commit/ac39540ad5bb36d22e0652966f8150ac5ddb2794))
- Make clippy happy - ([1d8768e](https://github.com/koharu-rs/koharu/commit/1d8768e2712b49c41076d942e8a32a5826004274))
- Remove unused dependencies (backon, ndarray, unicode-linebreak, rayon) - ([2f867f0](https://github.com/koharu-rs/koharu/commit/2f867f0aa2d13bbdc598220e05f202d7b581343c))
- Remove unused unicode dependencies from Cargo.toml - ([e78a115](https://github.com/koharu-rs/koharu/commit/e78a1154da56ef29e3d8d381de907f575271fad2))

### Release

- 0.9.0 - ([9fffbb8](https://github.com/koharu-rs/koharu/commit/9fffbb86367c64fe86c2d4eb649b27134751b903))


## [0.8.0](https://github.com/koharu-rs/koharu/compare/0.7.6..0.8.0) - 2025-11-12

### ⛰️  Features

- Update llmSystemPrompt for Japanese→English translation - ([a7192ab](https://github.com/koharu-rs/koharu/commit/a7192aba5830b44775c02d9d6a1be529a3747c95))
- Auto scale - ([bbe3a84](https://github.com/koharu-rs/koharu/commit/bbe3a841b9d2ddb71598bcb08933aebcce6b1f84))
- Add context menu for text block management in Workspace - ([1391da9](https://github.com/koharu-rs/koharu/commit/1391da9a945d71bef8b39ce94d9abc7a50f53cfa))
- Implement draft block functionality for text selection in Workspace - ([9958ce6](https://github.com/koharu-rs/koharu/commit/9958ce63d6fdcf8c0a58fb029b8adf433945b901))
- Enhance TextBlockAnnotations with update functionality and transformer support - ([f584401](https://github.com/koharu-rs/koharu/commit/f584401651078229a675fff4ab1184466d2f3456))
- Add update_text_blocks command to modify text blocks in documents - ([ac1eeb9](https://github.com/koharu-rs/koharu/commit/ac1eeb91289b31f9ca53c547f8c377f64dc5a8a2))
- Add default capability configuration - ([4f2e43d](https://github.com/koharu-rs/koharu/commit/4f2e43d45a4a924a9d40b9bad4eac1333c8163b0))
- Add tracing events for download events - ([806b310](https://github.com/koharu-rs/koharu/commit/806b3108078a2a5d521f1ea137dc32eb47dc102c))
- Implement core application structure and functionality - ([5e129e9](https://github.com/koharu-rs/koharu/commit/5e129e938af7beccd28612806518b76dfad01c08))

### 🐛 Bug Fixes

- Ui issue - ([7d0b081](https://github.com/koharu-rs/koharu/commit/7d0b081065448b1bf3c5fdf0182d7d6d90b307a1))
- Ui layout - ([0631160](https://github.com/koharu-rs/koharu/commit/0631160a5a4659e943503238f09b3714305ecd6c))
- Scroll bar - ([272b620](https://github.com/koharu-rs/koharu/commit/272b620e98501cd9250cba2a9cb9b9a425a08ea5))
- Update PROGRESS_WINDOW_EVENT URL to match naming convention - ([b6c7287](https://github.com/koharu-rs/koharu/commit/b6c7287dea0c30806c4204ea6f2314e25fac8835))

### 🚜 Refactor

- Use DOM-based canvas - ([93a9799](https://github.com/koharu-rs/koharu/commit/93a97998b58c4c9694c2c6169a827b0bcbc090a7))
- Update features section in README for clarity and consistency - ([4ce562c](https://github.com/koharu-rs/koharu/commit/4ce562c5720156d863cb891fc78411123a071340))
- Cleanup ui - ([8f0f1b8](https://github.com/koharu-rs/koharu/commit/8f0f1b8408ff472061ae4eaa4fcdaf39761166c8))
- Implement accordion for text blocks in Panels component - ([686698f](https://github.com/koharu-rs/koharu/commit/686698f9835890d4afd3825b19f71cd09f7c603d))
- Streamline Canvas component structure and remove unused functions - ([70e7c71](https://github.com/koharu-rs/koharu/commit/70e7c717b68cb3b0a262ac24f200878d0319f001))
- Simplify MenuBar component structure - ([8bb04f7](https://github.com/koharu-rs/koharu/commit/8bb04f7ffd0919a9cc2fe376776235576117882e))
- Improve UX - ([f97c9ed](https://github.com/koharu-rs/koharu/commit/f97c9edf5f8d874bdade5e5f19b7397c611022e1))
- Enhance logging configuration in initialize function - ([82bfd38](https://github.com/koharu-rs/koharu/commit/82bfd38e334fdb213053fd498512bf4fdddac70f))
- Use stream for http request - ([f4bddb1](https://github.com/koharu-rs/koharu/commit/f4bddb1564e3f04b40622e8231bff6d775d5fdac))
- Remove error logging in setup and replace with panic - ([0339985](https://github.com/koharu-rs/koharu/commit/0339985d58b3326f33ebdcfdcc5fd91855f67248))
- Use async api - ([295e4ee](https://github.com/koharu-rs/koharu/commit/295e4ee059694fec3bda574a283b37e7b0ec34c1))
- Renmae koharu-app to koharu - ([407f1ea](https://github.com/koharu-rs/koharu/commit/407f1eacc1f5b886701747b25302bc75a6b4c330))
- Reorganize dylib handling and update HTTP client references - ([0bd8409](https://github.com/koharu-rs/koharu/commit/0bd840918c1fc0e881fdece48f016ef33b62d87c))
- Simplify model load and offload commands - ([43a39c5](https://github.com/koharu-rs/koharu/commit/43a39c535d49a78fa0ec91e869cea0915fa4763d))

### ⚡ Performance

- Download with http range - ([b6d9cda](https://github.com/koharu-rs/koharu/commit/b6d9cda1da0ea382531e40f3ba689bcaf4d7caae))

### ⚙️ Miscellaneous Tasks

- Enable segmentation mask and inpainted image display - ([cdabe23](https://github.com/koharu-rs/koharu/commit/cdabe230f6396e109357d62c888d58474b6c6700))
- Rename koharu-app to koharu - ([172e06f](https://github.com/koharu-rs/koharu/commit/172e06faead8401628157cfc89fadc0932ec5bb6))
- Update tool modes in Workspace and store, removing 'navigate' and 'text' options - ([eac5950](https://github.com/koharu-rs/koharu/commit/eac5950365d08e9fa627734f2445bc68b7f51313))
- Replace updateBlock with updateTextBlocks for batch updates in TextBlocksPanel - ([90808a0](https://github.com/koharu-rs/koharu/commit/90808a0b8e3c2253ad02bb9cea377741533abbca))
- Add configuration store for detection and inpainting parameters - ([b40d3ba](https://github.com/koharu-rs/koharu/commit/b40d3ba17fc7b576719652c8fcaedf0aa8117649))
- Remove koharu_preview script - ([804958c](https://github.com/koharu-rs/koharu/commit/804958c84e47d12935733941f42811ff5e22cf38))
- Move signing to .github - ([3a62c72](https://github.com/koharu-rs/koharu/commit/3a62c726d38a1b4973f511cf58ed1dd23370ad1a))
- Clippy style - ([d2968f0](https://github.com/koharu-rs/koharu/commit/d2968f02fb6b0c36a706ba32abd62d1d3994a2e7))
- Make clippy happy - ([68bc706](https://github.com/koharu-rs/koharu/commit/68bc706bb4a7447ac78ad55744c0890072632d5f))

### Release

- 0.8.0 - ([7493466](https://github.com/koharu-rs/koharu/commit/7493466fa51387309c6b8b0f1c409fa9533cd592))


## [0.7.6](https://github.com/koharu-rs/koharu/compare/0.7.5..0.7.6) - 2025-11-07

### ⛰️  Features

- Retry on failures - ([430d1a3](https://github.com/koharu-rs/koharu/commit/430d1a3f108dcb769e2562da2f4aa74c329ea8c6))

### Release

- 0.7.6 - ([1e2e14c](https://github.com/koharu-rs/koharu/commit/1e2e14c68248b36bd0878de7de8477f0b4b23b60))


## [0.7.5](https://github.com/koharu-rs/koharu/compare/0.7.4..0.7.5) - 2025-11-07

### 🐛 Bug Fixes

- Download while not preload - ([e4b0c87](https://github.com/koharu-rs/koharu/commit/e4b0c8780929e2fa359484d3b1f3d6f51bd7a249))

### Release

- 0.7.5 - ([eeb4f59](https://github.com/koharu-rs/koharu/commit/eeb4f596be60af23c2df795466645e64b59012f9))


## [0.7.4](https://github.com/koharu-rs/koharu/compare/0.7.3..0.7.4) - 2025-11-07

### 🐛 Bug Fixes

- Onnxruntime dynamic loading - ([7052100](https://github.com/koharu-rs/koharu/commit/7052100856e73210e5ae05e5fd4c5dd71ba2ce27))
- Improve ONNX Runtime dynamic library loading conditions - ([7157186](https://github.com/koharu-rs/koharu/commit/7157186d2eeb3bca275e44b6b24ede4aec5ae4fc))
- Hard-coded dylib loading order - ([ea853e9](https://github.com/koharu-rs/koharu/commit/ea853e9b8c7341f0659abd4bc0fab806e2cb28c6))

### 🚜 Refactor

- Load dylib in data local dir - ([3bcebc8](https://github.com/koharu-rs/koharu/commit/3bcebc8de86f63a68ca83c05e083befb458dca63))

### ⚙️ Miscellaneous Tasks

- We don't need dll at bundle time - ([e3e2a16](https://github.com/koharu-rs/koharu/commit/e3e2a16a43565d7368968c82626c5e8bb290d882))

### Release

- 0.7.4 - ([746669e](https://github.com/koharu-rs/koharu/commit/746669eede6486bdd1e73ef854e9d0745f6a0709))


## [0.7.3](https://github.com/koharu-rs/koharu/compare/0.7.2..0.7.3) - 2025-11-06

### ⛰️  Features

- Refactor CUDA package handling and improve dylib loading in app initialization - ([d442e3c](https://github.com/koharu-rs/koharu/commit/d442e3cd867e32482c1786254be2c14b062bfe26))
- Update dependencies and improve CUDA integration in app initialization - ([32113f3](https://github.com/koharu-rs/koharu/commit/32113f3a6e41c1fe350a7470e5d0ab40d61d830f))
- Add tempfile dependency and implement tests for ensure_dylibs function - ([b0591aa](https://github.com/koharu-rs/koharu/commit/b0591aa9671cd499fa21e39d831ca1f11bd72190))
- Skip if hash matches - ([8717f47](https://github.com/koharu-rs/koharu/commit/8717f47608ba177dac8946a57c6ee4bb806b989b))
- Add zip via http - ([2dd0971](https://github.com/koharu-rs/koharu/commit/2dd0971fb46b0674038422359f83632158e9ada6))

### 🐛 Bug Fixes

- Download cuda libs to current dir - ([17b1e4d](https://github.com/koharu-rs/koharu/commit/17b1e4db9b346a01b001bd01f3e542017a6515a6))

### 🚜 Refactor

- Remove unused dependencies from Cargo.lock - ([a6a6494](https://github.com/koharu-rs/koharu/commit/a6a649464bb2356c6e5a0c61b9ccf635edc9202c))
- Speedup cuda-rt - ([26fdcc6](https://github.com/koharu-rs/koharu/commit/26fdcc6855e1d5dc9246dbce3ef18286829310f2))
- Replace ureq with reqwest in zip.rs and update dependencies in Cargo.toml and Cargo.lock - ([d6853c3](https://github.com/koharu-rs/koharu/commit/d6853c381a800fbcfcbf8863d17d806ea85a0298))
- Replace ureq with reqwest for HTTP requests in fetch_and_extract - ([6c94ee1](https://github.com/koharu-rs/koharu/commit/6c94ee1ec71aadf6ec57472958cbf05413e0d090))
- Switch reqwest client to blocking for synchronous operations - ([86683f0](https://github.com/koharu-rs/koharu/commit/86683f0211a69585c504af8a3d429ec15299bf6e))
- Mark RecordEntry struct as unused - ([23bbf0f](https://github.com/koharu-rs/koharu/commit/23bbf0fce3a827252c7bc8d35b0e9db7ec2caede))
- Make it a runtime lib - ([234c0e9](https://github.com/koharu-rs/koharu/commit/234c0e97a2c20a383ef2a893b93032cc4a0a96ac))
- Simplify workspace members list in Cargo.toml - ([aa643aa](https://github.com/koharu-rs/koharu/commit/aa643aa0054f2209ef33e5d1b864ed1830b338d5))
- Remove CUDA setup function from build script - ([6a9197f](https://github.com/koharu-rs/koharu/commit/6a9197f4236eda4ebbfdce367b093d23421e0169))
- Add cuda-rt crate - ([1613497](https://github.com/koharu-rs/koharu/commit/1613497f9bec78539d90327cd0041b0747958e5c))

### 📚 Documentation

- Remove optional Python dependency from installation instructions - ([81c8a99](https://github.com/koharu-rs/koharu/commit/81c8a99dda16175afbc86aafc3e264584a40dfb9))

### ⚙️ Miscellaneous Tasks

- Set default workspace member to koharu - ([0e668f8](https://github.com/koharu-rs/koharu/commit/0e668f865730e1003fa7d9034116ed0cb9c428d6))
- Enable cuda on windows by default - ([7840aa0](https://github.com/koharu-rs/koharu/commit/7840aa0f25a922bea426892332e997ba5eaf98d9))

### Release

- 0.7.3 - ([db7986a](https://github.com/koharu-rs/koharu/commit/db7986a665bd681be37a518ef8d30d2420345fd1))


## [0.7.2](https://github.com/koharu-rs/koharu/compare/0.7.1..0.7.2) - 2025-11-05

### Release

- 0.7.2 - ([4db1618](https://github.com/koharu-rs/koharu/commit/4db16186984f4bbda99e9caa24d586cc808072dc))


## [0.7.1](https://github.com/koharu-rs/koharu/compare/0.7.0..0.7.1) - 2025-11-05

### 🐛 Bug Fixes

- Update resource paths in tauri.windows.conf.json to specify release directory - ([ae49aa6](https://github.com/koharu-rs/koharu/commit/ae49aa6d2e465c13a1e196de8e7bf893538b96e3))
- Update artifact upload path to include all executable files - ([0aac81d](https://github.com/koharu-rs/koharu/commit/0aac81d1f9baa437bcc5899d1b970c3b6062b6fe))
- Add CUDA_COMPUTE_CAP environment variable to build workflow - ([59b1030](https://github.com/koharu-rs/koharu/commit/59b10307f873493bfd56f3413682c70bc04fe02b))

### 🚜 Refactor

- Switch to tauri nsis - ([67d1a06](https://github.com/koharu-rs/koharu/commit/67d1a0630b1d958faaa967e916d431af9840dfea))

### ⚙️ Miscellaneous Tasks

- Remove unused crate - ([be72ff6](https://github.com/koharu-rs/koharu/commit/be72ff6774718ec6a141b5b174b32b8166206158))
- Enable bundle - ([6706de8](https://github.com/koharu-rs/koharu/commit/6706de871fab31a88ce367dfb978f96de38f6db9))
- Enable bundle - ([31c4e18](https://github.com/koharu-rs/koharu/commit/31c4e18b2b264fb4d65b877d7106a8e3731a1b9f))
- Add updater - ([f12a6a4](https://github.com/koharu-rs/koharu/commit/f12a6a4de089115f0ecd0a09aede626799eee652))
- Add code signing - ([8da4ede](https://github.com/koharu-rs/koharu/commit/8da4edef4b9cbc65567d5da76e6f8cb863453a2c))

### Release

- 0.7.1 - ([5d184ce](https://github.com/koharu-rs/koharu/commit/5d184ce0623251f95d36f3cbf1b91c2cb9e613a3))


## [0.7.0](https://github.com/koharu-rs/koharu/compare/0.6.1..0.7.0) - 2025-11-05

### ⛰️  Features

- LLM translation - ([a1b0ad2](https://github.com/koharu-rs/koharu/commit/a1b0ad2877e9c01eebcdf1c2ef3940220cd09afd))
- Add llm commands - ([aac6474](https://github.com/koharu-rs/koharu/commit/aac6474b6f7b9564b966c56fa52789e3925819c0))
- Implement llm in koharu - ([9240ca1](https://github.com/koharu-rs/koharu/commit/9240ca1a1bcb7bdb57b14e66e09c1d97f1e9de1c))
- Prompt format - ([3a2d078](https://github.com/koharu-rs/koharu/commit/3a2d078268c427d13b5672b26147e500c969f462))
- Add Sakura1_5BQwen2_5_1_0 - ([326fc34](https://github.com/koharu-rs/koharu/commit/326fc34e8584aa74b2c68e4f2c7681db16da0c17))
- Add Qwen2_5_1_5BInstruct - ([5fbd1ba](https://github.com/koharu-rs/koharu/commit/5fbd1bac8c02f980d93d8445d80d58161999c992))
- Llm crate - ([bcf118d](https://github.com/koharu-rs/koharu/commit/bcf118d8d8e55c7f455cbd78be6b6fc9e59397e8))
- Add dilate and erode param - ([f21ec06](https://github.com/koharu-rs/koharu/commit/f21ec062cb91cfd7d7886503ebc1cf5f008695b3))

### 🐛 Bug Fixes

- Disable default features of tokenizers to avoid build issue - ([b2fe080](https://github.com/koharu-rs/koharu/commit/b2fe0803b49df6db7dd357861d000d2c87e279ba))
- Add missing step for MSVC development command setup - ([d08925f](https://github.com/koharu-rs/koharu/commit/d08925f17c2a7e56e5352c0bf63d2c586b4e5f69))
- Update CUDA_COMPUTE_CAP to 121 - ([d1cf241](https://github.com/koharu-rs/koharu/commit/d1cf241c7d0ca09098aeb518abe0dcbede96192d))
- Remove futures - ([93e007e](https://github.com/koharu-rs/koharu/commit/93e007e65eb19ec972b6ca177848e0d9344a614c))
- Correct model identifiers for Qwen2 and update README links - ([f7405d2](https://github.com/koharu-rs/koharu/commit/f7405d2753fd5178b2d646aa2a571f462941baf0))
- Use lowercase and remove prompt from output - ([29dd2c0](https://github.com/koharu-rs/koharu/commit/29dd2c02bed147feddf1573637ba8bbcbc177a4b))
- Centralize SplashScreen - ([f7b7171](https://github.com/koharu-rs/koharu/commit/f7b71718e3027027728433f496d764fa068785fd))

### 🚜 Refactor

- Production-ready - ([6438cc4](https://github.com/koharu-rs/koharu/commit/6438cc4cc9dd5024891ea8de93af568d79414f12))
- Make the llm ready to use - ([c9bb0c7](https://github.com/koharu-rs/koharu/commit/c9bb0c711f64cb651844cb864212f4c94d6ae6cf))

### 📚 Documentation

- Add logo to README for improved visual appeal - ([b8cfe32](https://github.com/koharu-rs/koharu/commit/b8cfe32f2e3a7bb50a45b675cd8b658c694cbc6f))
- Enhance README with detailed features and model descriptions - ([4abc8e8](https://github.com/koharu-rs/koharu/commit/4abc8e83192c2c676ef8aef4124830375d7a0cbc))
- Update README for improved clarity and structure - ([6404944](https://github.com/koharu-rs/koharu/commit/6404944ace0c3c26f3697bd1bb7f9a0c6d35ffd9))
- Update README to include CUDA installation instructions for candle - ([3672c2e](https://github.com/koharu-rs/koharu/commit/3672c2eac2c7c7c6fbaf8d4f2e3a4cc12ca8da2c))

### ⚙️ Miscellaneous Tasks

- Add cuda setup to release workflow - ([32e2a09](https://github.com/koharu-rs/koharu/commit/32e2a095064d64ea858350c595cac3f3ac23060e))
- Build cuda features - ([f1e962c](https://github.com/koharu-rs/koharu/commit/f1e962c79ee94ad984a806b1e656a03243263a68))
- Remove unused futures dependency from Cargo.toml - ([6a7a285](https://github.com/koharu-rs/koharu/commit/6a7a2852436c825b57549caced6998a1d015a988))
- Update package version to 0.7.0 and add related projects section in README - ([9f05afa](https://github.com/koharu-rs/koharu/commit/9f05afa75a85f367ca83e1e549080345e5c487bf))

### Wip

- Llm crate - ([40f79c7](https://github.com/koharu-rs/koharu/commit/40f79c7a7b05e035559926e17585f6081f681634))


## [0.6.1](https://github.com/koharu-rs/koharu/compare/0.6.0..0.6.1) - 2025-11-03

### Release

- 0.6.1 - ([d64cd79](https://github.com/koharu-rs/koharu/commit/d64cd79e92578027a2eee2b13ffeae1ac67cf3b2))


## [0.6.0](https://github.com/koharu-rs/koharu/compare/0.5.0..0.6.0) - 2025-11-03

### 🐛 Bug Fixes

- Canvas scollbar - ([a39f552](https://github.com/koharu-rs/koharu/commit/a39f5524eb610ff4219b3ff0025754a648a012c8))
- Add page number to thumbnails - ([91ba251](https://github.com/koharu-rs/koharu/commit/91ba25197da814b809dc126efb0ef79fa43476e8))
- Thumbnails panel - ([988c057](https://github.com/koharu-rs/koharu/commit/988c057478a88869b726ca147209d21c7bb57aad))

### 🚜 Refactor

- Make Rust the source of truth - ([dc2f168](https://github.com/koharu-rs/koharu/commit/dc2f168263674e4c7fdcb8224fe43f504ca0802e))
- Simplify image.rs - ([4f18e80](https://github.com/koharu-rs/koharu/commit/4f18e805e55e763438ac29c49c04f426929631c8))
- Replace react context with zustand - ([df068e7](https://github.com/koharu-rs/koharu/commit/df068e76646fe11d29b5a7be37d3b935f2fbdb03))
- Remove update - ([b63d6a3](https://github.com/koharu-rs/koharu/commit/b63d6a3aa9556da7142a319a01c64485540cde60))
- Impl state - ([4294399](https://github.com/koharu-rs/koharu/commit/429439902eec755a38c6a7a4291c4716e8c01966))
- Setup Tauri - ([0293296](https://github.com/koharu-rs/koharu/commit/0293296c3ae230aa8e6bbebdc74cf8ee6387ac76))
- Style components - ([0853572](https://github.com/koharu-rs/koharu/commit/08535727f0ff786a592c55de68f0db217213554b))
- Add initial next.js - ([7b15572](https://github.com/koharu-rs/koharu/commit/7b155725b87064f9e0287152329aedb9d19aa56f))
- Remove Slint - ([b747436](https://github.com/koharu-rs/koharu/commit/b74743694a13d15c4452f6bb8f5ffab802512716))

### 📚 Documentation

- Update README to clarify installation instructions and dependencies - ([3e1557d](https://github.com/koharu-rs/koharu/commit/3e1557dcc17da2dda484293046532058195d3352))
- Update GUI framework reference from Slint to Tauri - ([d5f3c58](https://github.com/koharu-rs/koharu/commit/d5f3c5890ee69251e460050a0d535628def94afe))
- Update dev steps - ([8cebcaa](https://github.com/koharu-rs/koharu/commit/8cebcaad4e52d0f18f466b71f9bc1cc12b53651f))

### 🎨 Styling

- Update splash screen text colors to pink - ([b8df66e](https://github.com/koharu-rs/koharu/commit/b8df66e03ba7c64eea23212b57e66a6b399cf13d))

### ⚙️ Miscellaneous Tasks

- Bump version to 0.6.0 - ([1b16609](https://github.com/koharu-rs/koharu/commit/1b1660944d424415e0e3726810f059bcd7b6f2fa))
- Add params to detect - ([27407a5](https://github.com/koharu-rs/koharu/commit/27407a55b4d87819b9b9710e2d5da35e1c7af41c))
- Remove unused lib - ([5f90727](https://github.com/koharu-rs/koharu/commit/5f90727a5f815fe9e0e680ff1f6c05714528e74a))
- Modify GHA to adapt tauri - ([31c298d](https://github.com/koharu-rs/koharu/commit/31c298db320564d13e10d6e96549a0cade5d33af))
- Stop publishing crates - ([074a15d](https://github.com/koharu-rs/koharu/commit/074a15db4e736611a8927ee9c2a57038b4a32b82))

### Build

- *(deps)* Bump clap from 4.5.50 to 4.5.51 ([#15](https://github.com/koharu-rs/koharu/issues/15)) - ([b295f12](https://github.com/koharu-rs/koharu/commit/b295f12bf065f5694795115e9275d5294b3dbd4e))

### Release

- 5.0.1 - ([ca9c856](https://github.com/koharu-rs/koharu/commit/ca9c856467e2b9a6ac2b8a9f01f6524909767674))


## [0.5.0](https://github.com/koharu-rs/koharu/compare/0.4.0..0.5.0) - 2025-10-28

### ⛰️  Features

- Inpaint - ([5de14dc](https://github.com/koharu-rs/koharu/commit/5de14dcc5ab8e53dc0bbf4f2982bbb7cc76f718a))
- Multiresolution blending with tiled inpainting - ([27d59d0](https://github.com/koharu-rs/koharu/commit/27d59d0ce197ea5e865da216d797da0f8646fa66))

### 📚 Documentation

- Reorganize README for clarity and remove outdated sections - ([d0645de](https://github.com/koharu-rs/koharu/commit/d0645dea0adb340034bd3349fddacb0a871a949e))
- Rewrite README.md - ([d38e9e2](https://github.com/koharu-rs/koharu/commit/d38e9e2f4053b98a786a850e2828eb50d539f9a0))
- Update README to clarify model downloading and ONNX conversion - ([1edc631](https://github.com/koharu-rs/koharu/commit/1edc6314909a5f6f85171970a4b1192a36a55c18))
- Enhance dev and cuda part - ([d46969d](https://github.com/koharu-rs/koharu/commit/d46969db161ec826ea326961ae7dd3c03c6ffea5))

### ⚙️ Miscellaneous Tasks

- Add publish workflow - ([0524502](https://github.com/koharu-rs/koharu/commit/0524502d10e2cc4eb007b4533db673d56dc43f4c))
- Remove unused ONNX inference script - ([43c0722](https://github.com/koharu-rs/koharu/commit/43c0722bd51ce344210776061b1c6f8f4f973761))
- Update deps - ([83c9275](https://github.com/koharu-rs/koharu/commit/83c927549f8c8506dd0e759b37df6855b058e843))
- Add app ico - ([56b4a75](https://github.com/koharu-rs/koharu/commit/56b4a756a19d4cf6ee62678c24aedae3be688c77))
- Apply cargo clippy - ([0cb2c97](https://github.com/koharu-rs/koharu/commit/0cb2c9715668ca11865c1dd9058c29bfd5656684))

### Release

- 0.5.0 - ([981079e](https://github.com/koharu-rs/koharu/commit/981079e29d85499f266e40a9f3ffb1dc2007fb0a))


## [0.4.0](https://github.com/koharu-rs/koharu/compare/0.3.1..0.4.0) - 2025-10-28

### ⛰️  Features

- Bundle cuda and cudnn - ([063c4bd](https://github.com/koharu-rs/koharu/commit/063c4bd939d206ba9d7ec175e73542320c5f7ccc))

### 🐛 Bug Fixes

- Ensure Spinner is indeterminate in InProgressOverlay - ([256a3a2](https://github.com/koharu-rs/koharu/commit/256a3a299be6efdc2d0d15d2f0bf346e4ccfdf19))

### ⚙️ Miscellaneous Tasks

- Add cuda flag - ([91e9d69](https://github.com/koharu-rs/koharu/commit/91e9d69c9b90dcc1195816ca1a3354d050629590))
- Bump version to 0.3.1 - ([74363ab](https://github.com/koharu-rs/koharu/commit/74363ab405ab0b9615f965b70ae2b335b4a44e35))
- Rename channel to win - ([cc860c2](https://github.com/koharu-rs/koharu/commit/cc860c2fbd6fd38d7409ab62044cdec6e07219c2))

### Release

- 0.4.0 - ([605525f](https://github.com/koharu-rs/koharu/commit/605525ffa184665e226fabd031ca4f1f6fc8b5d7))


## [0.3.1](https://github.com/koharu-rs/koharu/compare/0.3.0..0.3.1) - 2025-10-27

### 🐛 Bug Fixes

- Fix channel - ([0801903](https://github.com/koharu-rs/koharu/commit/0801903b14f27dd491384fb0bbc333aacef58c50))


## [0.3.0](https://github.com/koharu-rs/koharu/compare/0.2.3..0.3.0) - 2025-10-27

### 🐛 Bug Fixes

- Fix update logic - ([a92cbfd](https://github.com/koharu-rs/koharu/commit/a92cbfd0f114bae45deb6ca3e5694b54de9b58aa))
- Fix ci - ([71dacc7](https://github.com/koharu-rs/koharu/commit/71dacc7e3f12f7de6ea18dd21930da42bda282ad))

### 📚 Documentation

- Add dev steps - ([39d9443](https://github.com/koharu-rs/koharu/commit/39d9443f6551c24ab9aa009dd236a600ad394fab))

### Release

- 0.3.0 - ([cf07472](https://github.com/koharu-rs/koharu/commit/cf0747220cc4aae421adff3fb2c6091937e52fd8))


## [0.2.3](https://github.com/koharu-rs/koharu/compare/0.2.2..0.2.3) - 2025-10-27

### Release

- 0.2.3 - ([eedd6ea](https://github.com/koharu-rs/koharu/commit/eedd6eaa8242b629cf4ab41dd0224b69a4883ecc))


## [0.2.2](https://github.com/koharu-rs/koharu/compare/0.2.1..0.2.2) - 2025-10-27

### 📚 Documentation

- Add installation guide - ([62697df](https://github.com/koharu-rs/koharu/commit/62697df6d1a4f772d76bae1444a31704072a8007))

### ⚙️ Miscellaneous Tasks

- Bump version to 0.2.1 - ([d1b693d](https://github.com/koharu-rs/koharu/commit/d1b693d6bcb268785c63fc645d086d770b5cd3f5))

### Release

- 0.2.2 - ([e46a02c](https://github.com/koharu-rs/koharu/commit/e46a02c9fe7d5166d0112978e4ab6730583cc805))


## [0.2.1](https://github.com/koharu-rs/koharu/compare/0.2.0..0.2.1) - 2025-10-25


## [0.2.0](https://github.com/koharu-rs/koharu/compare/0.1.11..0.2.0) - 2025-10-25

### ⛰️  Features

- Ocr - ([05d6976](https://github.com/koharu-rs/koharu/commit/05d6976fab73657926c6eddfb76b4114ab53039b))
- Detection - ([d59d26e](https://github.com/koharu-rs/koharu/commit/d59d26efaef942f859def8bcc1ded24ee57f02c2))

### 🐛 Bug Fixes

- Fix release issue - ([2af5750](https://github.com/koharu-rs/koharu/commit/2af57504dbeaa81346b74210c630b06a7944165a))
- Images with wrong suffix not able to load - ([b71ff6e](https://github.com/koharu-rs/koharu/commit/b71ff6e5e40f8e47bd0a2f52bb06719fc34f5b7a))
- Improve error handling in image opening function - ([c5d9016](https://github.com/koharu-rs/koharu/commit/c5d901643fa9846171abead9838197c6c2e06495))
- Update image handling in detection function and improve error logging - ([0f3e3c7](https://github.com/koharu-rs/koharu/commit/0f3e3c7abbf2dc52f188ee50db03d69bd49cf352))
- Update state management to useEditorStore in App component - ([cf824f4](https://github.com/koharu-rs/koharu/commit/cf824f4d317959c40e70811497886bc6177ffad2))

### 🚜 Refactor

- Streamline operations - ([66b2cd9](https://github.com/koharu-rs/koharu/commit/66b2cd9884822f96895de8c72e084132249e8525))
- Single page editor - ([4826341](https://github.com/koharu-rs/koharu/commit/4826341f3c88051247c0505ddfa7ee9d2401ca05))
- Replace RwLock with Mutex for AppState management and move AppState to its own module - ([7f7a79c](https://github.com/koharu-rs/koharu/commit/7f7a79c3529172bf093e62d5f115d20b70143208))
- Rename Result type to CommandResult for consistency - ([5fee960](https://github.com/koharu-rs/koharu/commit/5fee96060c8216c11fedaeb39ad16ebd1b4f7f54))
- Add commands.rs - ([bcd7f70](https://github.com/koharu-rs/koharu/commit/bcd7f7025469343966916b80ba051093304370d1))
- Remove onnxruntime-web - ([5e23c2d](https://github.com/koharu-rs/koharu/commit/5e23c2d8fab6d37928487f57ec2212bab116c703))

### ⚡ Performance

- Improve image loading time - ([914c36d](https://github.com/koharu-rs/koharu/commit/914c36d47c85aa65eebfdf2c2cefd3f6de228944))

### 🎨 Styling

- Format slint features for better readability - ([f050dc0](https://github.com/koharu-rs/koharu/commit/f050dc018837ba7fb456f7d4a6e08e72921047aa))

### ⚙️ Miscellaneous Tasks

- Use winit with femtovg-wgpu render - ([ad8af8c](https://github.com/koharu-rs/koharu/commit/ad8af8ce0656a28a76149979f4a5be195943545e))
- Update deps - ([a6cc909](https://github.com/koharu-rs/koharu/commit/a6cc909a0bf1da02d5b9c296d8e9d96885cfa981))
- Add rust GHA - ([596488d](https://github.com/koharu-rs/koharu/commit/596488dda70ed59408a632ea50acd70875f957cc))
- Update deps - ([523df69](https://github.com/koharu-rs/koharu/commit/523df696ff6bde2851f4d0096b775777f1ab4e21))
- Separate logic into its own module - ([2e142fb](https://github.com/koharu-rs/koharu/commit/2e142fb66dbba8364ba3d6b3b059004f17301b13))
- Add alias and env to support live preview - ([8528d8a](https://github.com/koharu-rs/koharu/commit/8528d8ab8c5cb660c7a005aac05a0e7669d635e0))
- Publish sub-crates - ([e66999f](https://github.com/koharu-rs/koharu/commit/e66999ff89d64c36b8cd73f3a2508685e2cf47a3))
- Bump version for sub-crates - ([b93de9e](https://github.com/koharu-rs/koharu/commit/b93de9e956bf8b5ff4b678ab3e933ff6604babbb))
- Bump version for 0.2.0 - ([bbd3067](https://github.com/koharu-rs/koharu/commit/bbd30679502026a859080a3c371f72ecffca3afd))
- Update deps - ([8e9c41f](https://github.com/koharu-rs/koharu/commit/8e9c41faa813372f3654488db714774072ddc494))
- Update deps - ([dbc614a](https://github.com/koharu-rs/koharu/commit/dbc614a8b6e5c441c84b178ccdec1b90a464e562))
- Update dependencies in package.json and bun.lock to latest versions - ([fe6c1dd](https://github.com/koharu-rs/koharu/commit/fe6c1ddd7a81cdd126adcae0a766c6d590669e63))
- Remove unused wasm-bindgen dependency from Cargo.toml - ([1c0ada3](https://github.com/koharu-rs/koharu/commit/1c0ada3098dec0567c445855eab0ea699d83ecc3))
- Update @types/node and @types/react to latest versions - ([6605476](https://github.com/koharu-rs/koharu/commit/66054766a89adca0a67c4a63999e4b2534d2d596))
- Update dependencies in package.json and bun.lock - ([a4a9bbe](https://github.com/koharu-rs/koharu/commit/a4a9bbea856229edf6853cba8c56e0550adbfa15))
- Update description and windows capabilities in default.json - ([a560524](https://github.com/koharu-rs/koharu/commit/a56052484465156a25d60cd3e8b92e2bca8b3882))

### Release

- 0.2.0 - ([ba28a1a](https://github.com/koharu-rs/koharu/commit/ba28a1a5d8362c32414d95b8a15ff6b33ef9af40))


## [0.1.11](https://github.com/koharu-rs/koharu/compare/0.1.10..0.1.11) - 2025-08-06

### 🐛 Bug Fixes

- Correct formatting of CUDA paths in README.md - ([7dd4494](https://github.com/koharu-rs/koharu/commit/7dd44946f868eea7b4b983d0d70bf9708c6849f8))

### 🚜 Refactor

- Update section headers in README.md - ([0aeeccb](https://github.com/koharu-rs/koharu/commit/0aeeccba909289c79ff1650c24feb7a62cb7cec0))
- Load cuda - ([6c57cd8](https://github.com/koharu-rs/koharu/commit/6c57cd8041fd8fdff3de41984ea0c6b30097a53a))

### ⚙️ Miscellaneous Tasks

- Update package versions to 0.1.10 - ([66d24ba](https://github.com/koharu-rs/koharu/commit/66d24ba2d66248cfb2ed6d804ebf9999d8796da5))


## [0.1.10](https://github.com/koharu-rs/koharu/compare/0.1.2..0.1.10) - 2025-08-04

### ⛰️  Features

- Add cuda - ([cde9262](https://github.com/koharu-rs/koharu/commit/cde92629069e6724a7277795937ecc84ef90371e))
- Add p-limit library for limiting concurrent inference requests - ([3b825ef](https://github.com/koharu-rs/koharu/commit/3b825ef7ba54b37f11da46171c208db14e136173))

### 🐛 Bug Fixes

- Add tauri cli to root - ([df29d69](https://github.com/koharu-rs/koharu/commit/df29d69552e8fee11b0df8b56b9ad2274e6d7f1d))
- Reduce app size - ([fe85226](https://github.com/koharu-rs/koharu/commit/fe852264ef93e080845faf154ea75deea80b8f51))
- Improve error message formatting in model initialization - ([3736997](https://github.com/koharu-rs/koharu/commit/373699769be6b33f6a2a550a955e65f5b5079d1c))
- Ensure devIndicators is set to false in next.config.ts - ([7d0a2cd](https://github.com/koharu-rs/koharu/commit/7d0a2cda02a3be4a8cc40578c499a9fa30728563))
- Remove loop - ([829a740](https://github.com/koharu-rs/koharu/commit/829a7407d91d1366fb28e0596a587b2a90f5e2e1))
- Debug inpaint - ([6da800d](https://github.com/koharu-rs/koharu/commit/6da800d6ca19ab1afbd71c1fb7dcb1dab68c57ac))
- Use webgpu - ([0ef2b2a](https://github.com/koharu-rs/koharu/commit/0ef2b2ab1923fec3c7eb79ddcca3faf7e347f2cc))
- Initialize lib before using - ([759f7d6](https://github.com/koharu-rs/koharu/commit/759f7d62491aaff26aa3bc999234f8b290a751cb))

### 🚜 Refactor

- Rename selectedTool to tool in Canvas component - ([b3787d2](https://github.com/koharu-rs/koharu/commit/b3787d259f2e5215581856467d3b4e52ed2f8d6a))
- Rename selectedTool to tool in workflow store and related components - ([c3ae4e2](https://github.com/koharu-rs/koharu/commit/c3ae4e235fc97f1a04df0fbf3863b6a4e9b30a19))
- Remove unused settings button and related imports from Topbar component - ([4dd1479](https://github.com/koharu-rs/koharu/commit/4dd1479c90fafcf5ad09d9a9488a9bb34126a252))
- Remove VSCode extensions configuration file - ([e05656a](https://github.com/koharu-rs/koharu/commit/e05656a52c88330c9b93aef37ad440b0987314ae))
- Streamline image resizing and tensor creation in inference function - ([c672115](https://github.com/koharu-rs/koharu/commit/c672115a68dbe7dc0fda8b935a50d2b3c0d1138c))
- Add TODO for improved error handling in model initialization - ([3b97fe4](https://github.com/koharu-rs/koharu/commit/3b97fe45b98715f71858f65d44d42758c7ba3fbe))
- Remove settings page - ([c6273dd](https://github.com/koharu-rs/koharu/commit/c6273dd1e17fcc7eaee1f6d750d641df548fa6f2))
- Use ImageBitmap directly - ([3871b93](https://github.com/koharu-rs/koharu/commit/3871b93b1eb728cc01ec8eb4d9155a38b34d5259))
- Move maskThreshold to inference parameters and relocate non-maximum suppression implementation - ([e1adbd9](https://github.com/koharu-rs/koharu/commit/e1adbd9736e4cf3fd9fcf124bf47e319502f7343))
- Use ImageBitmap - ([e7e1745](https://github.com/koharu-rs/koharu/commit/e7e174577dbcc626a587c7733be42a111bf92fba))
- Use Jimp - ([1b2aebb](https://github.com/koharu-rs/koharu/commit/1b2aebb2e575575bdcc0cd88f04c80b456fc7de1))
- Rename util to utils - ([af35536](https://github.com/koharu-rs/koharu/commit/af3553619d4f365fa8b53fab3499617c5c1ee5c5))
- Remove hooks - ([aaeb76a](https://github.com/koharu-rs/koharu/commit/aaeb76a360fd3da556bb6599cecc315cac08d759))
- Consolidate image and model utility functions into a single directory - ([6269bea](https://github.com/koharu-rs/koharu/commit/6269bea1cc8336093a55137fd17353736eb25f94))
- Remove unused Tauri import and clean up code - ([36afe7f](https://github.com/koharu-rs/koharu/commit/36afe7ff481bb4b0b96f2d50ec5edd310ee2b13d))
- Remove ort and use onnxruntime-web - ([ea893ac](https://github.com/koharu-rs/koharu/commit/ea893ace52aa628e69bcb09c487fb6491ce1fa68))
- Add splash screen - ([1a1464f](https://github.com/koharu-rs/koharu/commit/1a1464f2a58f471925d5ebab03cbb41895828287))

### 📚 Documentation

- Add Rust version requirement to prerequisites - ([39ace09](https://github.com/koharu-rs/koharu/commit/39ace090c23ea55dba33b010c70fa8490b9ca917))
- Remove ONNX model references from README - ([033d3d0](https://github.com/koharu-rs/koharu/commit/033d3d067be04d01aafdc2429f0003c05b99e79d))
- Update README to reflect technology stack and application hosting - ([2a94def](https://github.com/koharu-rs/koharu/commit/2a94def7e404368c5d53fdb18a57bc30ec5553af))
- Update README to remove browser trial reference - ([9a5f7a6](https://github.com/koharu-rs/koharu/commit/9a5f7a68262f746c8b0463bd23380cd3c0c7e72a))

### ⚙️ Miscellaneous Tasks

- Update package version to 0.1.10 - ([b23070f](https://github.com/koharu-rs/koharu/commit/b23070f674cd25e0565147af7d6fa0430c393ee5))
- Rename web to next - ([c33ccd1](https://github.com/koharu-rs/koharu/commit/c33ccd1acf2c31ff9c9c54559cddc6e580055df8))
- Reduce app size - ([62c37fd](https://github.com/koharu-rs/koharu/commit/62c37fda982344a8c3560cbdae1634e98bbb267e))
- Update ort to rc10 - ([4cb3c18](https://github.com/koharu-rs/koharu/commit/4cb3c18491c764eca561e4b423465faebac412fa))
- Update cargo lock - ([c0caae1](https://github.com/koharu-rs/koharu/commit/c0caae197c293ba9b79e03e95d96d65a560d596c))
- Update dependencies in package.json - ([be7ff0b](https://github.com/koharu-rs/koharu/commit/be7ff0bd8da9faad950d2ad77ba9717301d79283))
- Update workspace resolver to version 3 - ([8371194](https://github.com/koharu-rs/koharu/commit/8371194b5ca60446045c71204ba85b16cff76070))
- Update onnxruntime-web to version 1.23.0-dev.20250703-7fc6235861 - ([a7b04c2](https://github.com/koharu-rs/koharu/commit/a7b04c28f87f8b13e649aacd48efc697c9ac6ab2))
- Update Next.js configuration and dependencies - ([0417750](https://github.com/koharu-rs/koharu/commit/0417750751052fe7a93c2362ab3cc4e6f373f287))
- Update prettier to version 3.6.2 in package.json and bun.lock - ([5d98dce](https://github.com/koharu-rs/koharu/commit/5d98dce07ceb7ee089f43ddc426c5d6667c32986))
- Remove .gitattributes file - ([4344ae4](https://github.com/koharu-rs/koharu/commit/4344ae4acccd4cfd15439d3f24a9d0eab3b80f7f))
- Remove web deployment workflow - ([efe2579](https://github.com/koharu-rs/koharu/commit/efe2579e5b2ef2e2d13ec47f36a652ee6c781621))
- Remove experimental directories from .gitignore - ([ab0799f](https://github.com/koharu-rs/koharu/commit/ab0799f61aac0108ba81feb3c242b3dd6a56a95e))
- Update dependencies in package.json files - ([510df2c](https://github.com/koharu-rs/koharu/commit/510df2c765883bfcd55a2e6cb0dd556d12c1453e))
- Update artifact upload condition to only trigger on tag pushes - ([0b1b153](https://github.com/koharu-rs/koharu/commit/0b1b1538b4db11ae04c34aee89afd1fb8c83e669))
- Bump package versions to 0.1.2 - ([63b0fb2](https://github.com/koharu-rs/koharu/commit/63b0fb22c0efb72f920fccc8896962e2a4810bc0))


## [0.1.2](https://github.com/koharu-rs/koharu/compare/0.1.1..0.1.2) - 2025-05-29

### 🐛 Bug Fixes

- Streamline Rust installation step in build workflow - ([45045f3](https://github.com/koharu-rs/koharu/commit/45045f37b4294918ca3780d4af7d222898cb54f0))
- Remove bundling step for Windows 2025 platform - ([31fc42e](https://github.com/koharu-rs/koharu/commit/31fc42e97dc2b26b87ec6a09b9cc820f366d85f8))
- Update condition for Windows Tauri app bundling - ([7cb17f0](https://github.com/koharu-rs/koharu/commit/7cb17f0591f2e7cd4e9cee94d17c0b3b2954cdaf))
- Fix Windows build - ([7b2c0f5](https://github.com/koharu-rs/koharu/commit/7b2c0f5d834eb84e503112f58848650a1e6710b3))
- Place dll into root dir - ([89d1024](https://github.com/koharu-rs/koharu/commit/89d102400eef9f0c45e5ea90098d05acfc702b00))
- Update jq command to use raw output for version retrieval - ([af2d684](https://github.com/koharu-rs/koharu/commit/af2d684d971295d9cf8ce4889c0147eb974c2298))

### ⚙️ Miscellaneous Tasks

- Bump version to 0.1.2 - ([73b06ae](https://github.com/koharu-rs/koharu/commit/73b06aefbf8e1fa9378111e31a6188b2cf34e7d9))
- Refine cuda workflow - ([1763290](https://github.com/koharu-rs/koharu/commit/1763290a4a42a90fbb3952637af29ecaf532398a))
- Fix windows bundle - ([f3fe058](https://github.com/koharu-rs/koharu/commit/f3fe0581ab69836bf2c623c1f88be02af2fe51e7))
- Fix windows - ([968c92a](https://github.com/koharu-rs/koharu/commit/968c92ae993b355dea4b136ca46ff146de25f327))
- Try to fix windows build - ([11f3f67](https://github.com/koharu-rs/koharu/commit/11f3f67229d1faa42bf94ee58699191390677da0))
- Fix windows build - ([4a73ccd](https://github.com/koharu-rs/koharu/commit/4a73ccd9880c8e2412530b7aefe741c1f5284b32))
- Remove unused setting - ([58f8ce6](https://github.com/koharu-rs/koharu/commit/58f8ce6a9974da36cbaae1686b1b28e1e8d69df2))
- Support nsis bundle - ([910e9de](https://github.com/koharu-rs/koharu/commit/910e9dee0515d7caf5b4cb1279b5b5952a362a96))
- Update version retrieval method in build workflow - ([79161c3](https://github.com/koharu-rs/koharu/commit/79161c35985be0de10575d7e42b9f7db1c99ce37))
- Use version from Cargo.toml - ([3c994c2](https://github.com/koharu-rs/koharu/commit/3c994c2914581a9d1fb9e7a9446951d09fbd1c37))
- Use underscore - ([3798941](https://github.com/koharu-rs/koharu/commit/37989411300131444dec7e7f5a44a535e38212b6))
- Fix windows - ([3e3ec59](https://github.com/koharu-rs/koharu/commit/3e3ec59b79b2d529107715803a10d98be2228c05))


## [0.1.1] - 2025-05-28

### ⛰️  Features

- Add progress indicator to splash screen during initialization - ([4af41a0](https://github.com/koharu-rs/koharu/commit/4af41a085ac111fcfc0718f34ba2a4dbeebb9426))
- Store model in indexeddb - ([22c3c51](https://github.com/koharu-rs/koharu/commit/22c3c517defc6645378be12117839558565ed9bc))
- Initial support of web - ([fbcd713](https://github.com/koharu-rs/koharu/commit/fbcd713e9c13d1e9046d2a8ca5b9c7a92ea13262))
- Add system prompt label to TranslationPanel for improved clarity - ([4e29341](https://github.com/koharu-rs/koharu/commit/4e293412f8f9c2c5be758f1820ed395e66227601))
- Update translation logic to use JSON format and improve response handling - ([1e656f0](https://github.com/koharu-rs/koharu/commit/1e656f036aeec88dfbceb7818cc34b32cca6bee6))
- Enhance OCR and Translation panels with badge indicators for text items - ([9d0eb7e](https://github.com/koharu-rs/koharu/commit/9d0eb7ebc8a27f98685bdbdbb014ad5bb32d498f))
- Add confidence and NMS threshold controls in DetectionPanel and update detect function to accept thresholds - ([62b44ba](https://github.com/koharu-rs/koharu/commit/62b44ba68166a8714f0acee9fc83d05bc0767f08))
- Integrate Radix UI themes and update components - ([a7a0eb3](https://github.com/koharu-rs/koharu/commit/a7a0eb3c08a5968a4ea6abc8591fc49bbada1411))
- Split encoder and decoder to reuse encoder outputs for manga-ocr, close #7 - ([f20112e](https://github.com/koharu-rs/koharu/commit/f20112eaf20091635c0aa2af5bbb808c3e48f1fa))
- Use english for UI - ([2fdf7ec](https://github.com/koharu-rs/koharu/commit/2fdf7ec864651b21d4e155a24e1dafcd641df3c0))
- Import react aria - ([fc13d80](https://github.com/koharu-rs/koharu/commit/fc13d80b95a3136a5ce683088af7f0ad43832cc7))
- Add notification - ([14e60e4](https://github.com/koharu-rs/koharu/commit/14e60e4290ce9587f04a82daa680358bc9910a52))
- Parallel inpaint - ([81eed82](https://github.com/koharu-rs/koharu/commit/81eed8261efe91b12eabd022b7fa4da79181d7d3))
- Use correct exported onnx inpaint model - ([15d15db](https://github.com/koharu-rs/koharu/commit/15d15db90fc98b8d13e21c76e4a1b87852cec327))
- Dilate segment mask - ([e47b714](https://github.com/koharu-rs/koharu/commit/e47b7143409a3a41eea17d59f5d524fade697fd2))
- Add logging support and update run function to return a result - ([35ab274](https://github.com/koharu-rs/koharu/commit/35ab2748737628891dce5aa78d17f170c9c080a1))
- Implement image resizing with padding and add revert functionality - ([0204a5d](https://github.com/koharu-rs/koharu/commit/0204a5d6f268e887542d4a4b9034a6e9eb0f6693))
- Free-draggable canvas layout with floating toolbars - ([01cc760](https://github.com/koharu-rs/koharu/commit/01cc760ecd82ff543822cce5f7624ea014e07d7c))
- Scroll mouse wheel to zoom the canvas - ([5c4724e](https://github.com/koharu-rs/koharu/commit/5c4724e757bafcba9026c1dddcd1cabf0cf31743))
- Add lama - ([4ae2fd6](https://github.com/koharu-rs/koharu/commit/4ae2fd6677665ab804b814c306b8a355c5e505a8))
- Add segmentation functionality and update state management - ([6aefd2d](https://github.com/koharu-rs/koharu/commit/6aefd2d0295f59fd753f82bc3fb803892e6c65f4))
- Add CUDA feature support for ort dependency and update build configuration - ([8965151](https://github.com/koharu-rs/koharu/commit/8965151c4faf6eadf561f9d311cdddaf5cdf16e8))
- Trigger OCR automatically when detect complete - ([223753a](https://github.com/koharu-rs/koharu/commit/223753a372d1490a6d5682cd191467b994835fa5))
- OCR text editable - ([5ea3746](https://github.com/koharu-rs/koharu/commit/5ea3746a6ef471b2948a884b1aabd10ff38f7b2f))
- Translate text using stream mode - ([c0aea96](https://github.com/koharu-rs/koharu/commit/c0aea96ff9c2f7ba552149881092edc8ed850fb7))
- Use GPU acceleration when available - ([d649095](https://github.com/koharu-rs/koharu/commit/d64909568ade8e4cdbd08f6559924666b18908c6))
- Highlight selected text item - ([9fd25f0](https://github.com/koharu-rs/koharu/commit/9fd25f0a8cb663274884f2e3e597e1107692ba39))
- Order bboxes - ([727e836](https://github.com/koharu-rs/koharu/commit/727e836dc49c5d233347a519eade047e19e7d521))
- Enhance canvas and detection panel with loading state and reset functionality - ([897f7af](https://github.com/koharu-rs/koharu/commit/897f7afe90c04b0d1e589fd70985a37d4deca478))
- Add blocks state management and render rectangles in canvas - ([c439ef0](https://github.com/koharu-rs/koharu/commit/c439ef07636b5a381eb61ed39743305d1f0e2a48))
- Implement comic text detection and update dependencies - ([0c7d25b](https://github.com/koharu-rs/koharu/commit/0c7d25be60447eb3f0b1e1774dbaedaa66c34b92))
- Add comic text detection functionality and update dependencies - ([9070f43](https://github.com/koharu-rs/koharu/commit/9070f43c3b7b197c8ae05d2415d115a19212607e))
- Add scale control - ([582ead8](https://github.com/koharu-rs/koharu/commit/582ead827920e466c208fed5334104e0cc943b15))
- Add Tauri plugins for store and persisted scope, update dependencies in Cargo and package files - ([120d876](https://github.com/koharu-rs/koharu/commit/120d876a4c5b41d904cc2791388599114bbe599c))
- Enhance canvas layout and improve image handling in Topbar component - ([1d54678](https://github.com/koharu-rs/koharu/commit/1d546780d6f201d2f0971268086efb905750cfd3))
- Add Tauri plugins for dialog and logging, enhance canvas functionality, and improve app structure - ([771255e](https://github.com/koharu-rs/koharu/commit/771255e3365ddaad268a49e79fe0cb4fe01d317b))
- Integrate react-konva for canvas rendering in App component - ([0aca2a1](https://github.com/koharu-rs/koharu/commit/0aca2a1570617768477cbbd562095deff9a3f592))
- Add layout components and tools for layer management - ([d9b5ff1](https://github.com/koharu-rs/koharu/commit/d9b5ff18ff83ae83741ea6553c41a38a4952aa49))
- Enhance image processing with configurable thresholds and bounding box drawing - ([5176216](https://github.com/koharu-rs/koharu/commit/517621624a5452cdd2a20e0957fedfcb69469b15))
- Add image processing and object detection functionality to comic-text-detector - ([96692b5](https://github.com/koharu-rs/koharu/commit/96692b5e6600a5a7ddc0ec5537eba94ee1095135))
- Update dependencies and enhance model session handling in main.rs - ([6a6b246](https://github.com/koharu-rs/koharu/commit/6a6b2464c3e5b1eacb12a6f5f380c6ccc1014b7b))
- Implement main functionality for comic-text-detector and update dependencies - ([7aba290](https://github.com/koharu-rs/koharu/commit/7aba290a2db4a1aa4e722179659b4d31ce455741))
- Add comic-text-detector package with initial implementation - ([155adde](https://github.com/koharu-rs/koharu/commit/155addec952f984b682e7eb169daea1b224d84a9))
- Simplify manga109 to YOLO conversion by removing book selection and updating class mapping - ([2ff157b](https://github.com/koharu-rs/koharu/commit/2ff157b3ba9a4304314864b6ca3f3e518f577e0b))
- Add script to download Blue Archive comics with asynchronous requests - ([98f31a0](https://github.com/koharu-rs/koharu/commit/98f31a0cf4d5c89019b7e98b99eaa716ee326bd6))
- Enhance manga109 to YOLO conversion with 80/20 train/val split and YAML config - ([021eeb3](https://github.com/koharu-rs/koharu/commit/021eeb3e6aa9ef497b03e38da7d3828ebf1ff082))
- Add Puppeteer script for Paperspace login and notebook access - ([adf207e](https://github.com/koharu-rs/koharu/commit/adf207e680a03b7f27973535ad8d7ceb590ad55f))
- Add detection notebook for dataset preparation and training with YOLO - ([af44ec4](https://github.com/koharu-rs/koharu/commit/af44ec49473fa08ae5c234fa4d2f20be287b42e6))
- Add manga109 to YOLO conversion script - ([85b28aa](https://github.com/koharu-rs/koharu/commit/85b28aa926eb0031f6ef3c8be4e7d567528bf69a))
- Initialize Tauri + React application with basic greeting functionality - ([b3ab757](https://github.com/koharu-rs/koharu/commit/b3ab757192fb12f4c4137d1811da12654fec10ac))

### 🐛 Bug Fixes

- Check for navigator before setting wasm.numThreads - ([c9eef09](https://github.com/koharu-rs/koharu/commit/c9eef09e51aacbf41454553439c2fe0e5c114724))
- Inpaint pixel index - ([a0991f2](https://github.com/koharu-rs/koharu/commit/a0991f251d0fcbe31c32c20b13d92e079250381b))
- Correct token ID calculation in OCR inference logic for accurate text extraction - ([92eab6a](https://github.com/koharu-rs/koharu/commit/92eab6a38041844f88bc45b8229d4a53cc614bd2))
- Ensure canvas dimensions default to zero when image data is unavailable - ([a4ed34d](https://github.com/koharu-rs/koharu/commit/a4ed34dd9092eb050c0bff5755c5c710f1fc76c5))
- Adjust position of ScaleControl component for better visibility - ([46d4457](https://github.com/koharu-rs/koharu/commit/46d445790a109e242c80534f658dac44349bd676))
- Add thresholded for mask - ([e379440](https://github.com/koharu-rs/koharu/commit/e37944017c26a7acbbe1b3bcd29824daba7cd5d6))
- Canvas not automatically updated after inpaint operation - ([b333008](https://github.com/koharu-rs/koharu/commit/b3330088a746cf3e146524bd2700d6bad2e8e3e5))
- Update model path in CLI and add BallonsTranslator to .gitignore - ([fe60a2d](https://github.com/koharu-rs/koharu/commit/fe60a2d895d699d02fbb9576c908d75706beb43a))
- Update model path and optimize mask value extraction - ([40801c4](https://github.com/koharu-rs/koharu/commit/40801c4ec79c8b91547e35ae8d7f6700ddd387b1))
- Prevent operations from previous file continuing on new file open - ([d13f2a1](https://github.com/koharu-rs/koharu/commit/d13f2a1272484dca0eec8d643fcf041238069777))
- Use for-await-of to handle asynchronous text processing in inpainting - ([07716e9](https://github.com/koharu-rs/koharu/commit/07716e9b46c2fb4ac108cf0fefee64f5e0999f54))
- Fix the minor error - ([f192ce2](https://github.com/koharu-rs/koharu/commit/f192ce2fe355d627573100b4abbae18daf03d063))
- Update mask interpretation logic for inpainting areas - ([8ca30a9](https://github.com/koharu-rs/koharu/commit/8ca30a945e9892f915741749bfde64ae3a2f20bb))
- Fix mask png and add more examples - ([89ebf36](https://github.com/koharu-rs/koharu/commit/89ebf3669cdd5e0f5c73f680c0a13a6e1459a08d))
- Handle null segmentData in Canvas and clear segment on new image load in Topbar - ([879cd0c](https://github.com/koharu-rs/koharu/commit/879cd0cb6ced6a9eb2ef66d3a310e0a7122ccffa))
- Update README to correct bundle size from less than 10MB to less than 20MB - ([7e3e783](https://github.com/koharu-rs/koharu/commit/7e3e7832ebfaffe7920f9b81e2efe55ca08f9d96))
- Canvas cannot be fully displayed when scrollbar is scrolled to the bottom - ([598c295](https://github.com/koharu-rs/koharu/commit/598c29598ff6ab248422d1d6048b6c50676dcc4d))
- Right panel scrollbar - ([e0d5e3e](https://github.com/koharu-rs/koharu/commit/e0d5e3e0bde84b28ebbedf232b5a5d5856b419af))
- Remove unnecessary blank line in README.md - ([ca0c16e](https://github.com/koharu-rs/koharu/commit/ca0c16ecc4a65d85d711ab52633c48edabe5ed96))
- Redetect when switch tools, settings - ([83ffeba](https://github.com/koharu-rs/koharu/commit/83ffeba9e31c75993bfd2b078a4136a91cc31102))
- Stream translate missing last piece - ([cb82324](https://github.com/koharu-rs/koharu/commit/cb823248dfbd44d7fa9ff623a989f2637a2959e3))
- Specify shell for artifact processing step in release workflow - ([937ece3](https://github.com/koharu-rs/koharu/commit/937ece3913048c6d019c05a311392f8f151d1bb4))
- Refactor artifact upload step to improve handling of paths - ([5a64297](https://github.com/koharu-rs/koharu/commit/5a64297221408f1397c3b4fc04badf554282ea15))
- Update artifact paths formatting for GitHub release upload - ([33c04e3](https://github.com/koharu-rs/koharu/commit/33c04e3c37f9efdd0f7048f293bd5c89f4a5d35c))
- Correct JSON parsing for artifact paths in release workflow - ([7e3d378](https://github.com/koharu-rs/koharu/commit/7e3d378c18c747df6dd43c96262745af28f06b38))
- Add max height and overflow to OCR panel for better layout - ([11e11db](https://github.com/koharu-rs/koharu/commit/11e11db728e1236167c599061b3c332938111065))
- Ocr results got cutted - ([4dacfa5](https://github.com/koharu-rs/koharu/commit/4dacfa5afbb67ed0720c24bbad04454ae8cd7ceb))
- Display detected texts in detection and OCR panels - ([79b902b](https://github.com/koharu-rs/koharu/commit/79b902b8ae59bc8223b73b208c39e45938246f7c))
- Clears blocks only when new image loaded - ([3426a34](https://github.com/koharu-rs/koharu/commit/3426a34567c50e739b81f9a44fda60b74d8cf6e0))
- Fix panic - ([4a0b947](https://github.com/koharu-rs/koharu/commit/4a0b947a27cc0752bbd7fdfdd8513dbfdb5e98d8))
- Simplify ort dependency declaration by removing unnecessary features - ([5d226c3](https://github.com/koharu-rs/koharu/commit/5d226c38f98b555f7e75fab88ea09f55f4cf5d44))
- Ensure stage size is set correctly to match image dimensions - ([10b47af](https://github.com/koharu-rs/koharu/commit/10b47afc997ce8738d54ef95acd1be47f2db348b))
- Ensure file selection is validated before processing in Topbar component - ([1b340a9](https://github.com/koharu-rs/koharu/commit/1b340a91057a0924e662db371953c2d52fc85639))
- Fix typo - ([1f68c49](https://github.com/koharu-rs/koharu/commit/1f68c491d834eae9983088ed82c76592f704e540))
- Fix build step - ([5e7c781](https://github.com/koharu-rs/koharu/commit/5e7c781f2c6a8ee248d8f117e2c48eae5b98260a))
- Fix image loaded - ([c1aee01](https://github.com/koharu-rs/koharu/commit/c1aee01ab43444c0168aae5f4e5619cd707252de))
- Move stageRef.current.destroyChildren() call to ensure proper cleanup before loading new image - ([41d02fb](https://github.com/koharu-rs/koharu/commit/41d02fb46c4002f429a50cbfcfc865ec963fb0a1))
- Add missing description for manga-ocr model in README - ([e9ad2cf](https://github.com/koharu-rs/koharu/commit/e9ad2cf0cc5cd30350fe1c82d3df4970c326eb0f))
- Fix typo - ([8e76902](https://github.com/koharu-rs/koharu/commit/8e7690222407688774d3e687e7f89ff86c7cc272))
- Update gdown command to use placeholder for ID - ([a9180ab](https://github.com/koharu-rs/koharu/commit/a9180ab7ef528926ebd2881922c6e05a0d34826a))
- Correct model import in main.rs - ([f6eab13](https://github.com/koharu-rs/koharu/commit/f6eab139b1da72b4a337dd2d46fdbfa21331387e))

### 🚜 Refactor

- Use sub modules - ([bbdb057](https://github.com/koharu-rs/koharu/commit/bbdb05794a5c5a14f97a48678742d6ce2a5dbffe))
- Implement caching for model downloads to improve performance - ([c8e1c39](https://github.com/koharu-rs/koharu/commit/c8e1c390cbd370c7b15d78a0acb4c26eda3a6748))
- Remove unused packages from Cargo.lock - ([93ea2a6](https://github.com/koharu-rs/koharu/commit/93ea2a6252eb52907e75432fd2e26e15c21e8e9f))
- Remove unused projects and clean up workspace - ([6e0ac91](https://github.com/koharu-rs/koharu/commit/6e0ac913e196d7e6876d435fddcc0059b6e9e83a))
- Remove unused output setting from Next.js configuration - ([4f3d651](https://github.com/koharu-rs/koharu/commit/4f3d6517836e3de16ecd6ad652057e73f99c55a0))
- Simplify Next.js configuration by removing unused image settings and asset prefix - ([fc446ed](https://github.com/koharu-rs/koharu/commit/fc446ed114020e3c7560751d5c4b717479c2dfc9))
- Update ONNX runtime import to use 'onnxruntime-web' for consistency across detection and OCR modules - ([3baedc8](https://github.com/koharu-rs/koharu/commit/3baedc84fc3f5b026f8f17459d2adef54089df9c))
- Optimize OCR processing logic in OCRPanel for improved text extraction - ([73a0753](https://github.com/koharu-rs/koharu/commit/73a0753e0b8c9b0d42d849083b0dfd470050ee96))
- Update image handling in components to use imagePath instead of image for improved file management - ([484aa8e](https://github.com/koharu-rs/koharu/commit/484aa8e6a31067326730fb61b7c77f78504484b6))
- Streamline Canvas component by removing redundant stage reference and adding inpaint layer reference - ([bf50a44](https://github.com/koharu-rs/koharu/commit/bf50a44018168294359577dfc0ace6bf0f00fbc6))
- Add tauri-plugin-fs for file system access and update dependencies in package.json and Cargo.lock - ([2d737b6](https://github.com/koharu-rs/koharu/commit/2d737b6689950699f1885e63ff7df9bb9900cd46))
- Remove bugy wheel event handler from Canvas component to streamline functionality - ([b9e0fad](https://github.com/koharu-rs/koharu/commit/b9e0fad913177d7d5a0555f93a454d307617f577))
- Improve layout and responsiveness of main components, remove unused window size hook - ([eb05d12](https://github.com/koharu-rs/koharu/commit/eb05d124ead639cff11a8e3773389e946d3ffd15))
- Update inference triggers in DetectionPanel and OCRPanel, and adjust imageSrc handling in Topbar - ([1ecc74e](https://github.com/koharu-rs/koharu/commit/1ecc74e63668f2b7535f4f97f8db7c5828d49082))
- Remove imageSrcHistory and related logic from canvas store - ([4af1b4a](https://github.com/koharu-rs/koharu/commit/4af1b4a727be64cbc77e5d8f0960eaf4594e08e8))
- Remove unused execution providers from model initialization - ([edc2ada](https://github.com/koharu-rs/koharu/commit/edc2ada6cf09b814c76ae367e322114cd0134984))
- Extract canvas logic into hooks and utils - ([3e454ae](https://github.com/koharu-rs/koharu/commit/3e454ae223ec249fe74161f52a98fc483498bf5e))
- Use available parallelism for intra-threads in model sessions - ([843b0ac](https://github.com/koharu-rs/koharu/commit/843b0acf360d1f26b4edd1265480efa24d1dd88f))
- Use konva-react - ([7bd4739](https://github.com/koharu-rs/koharu/commit/7bd473972a17fa4068982c0148720f47b500ba34))
- Update tool selection and improve topbar icon imports - ([1a8356a](https://github.com/koharu-rs/koharu/commit/1a8356a1d8b7a83dff933d7bc75bfdca270022d3))
- Improve scale control component structure and functionality - ([9aca40f](https://github.com/koharu-rs/koharu/commit/9aca40f580e6c4ad1535cec159cadebd7765d741))
- Update project structure and dependencies - ([f896aa1](https://github.com/koharu-rs/koharu/commit/f896aa11c4a23f75c20ee4c73260f662fea446b6))
- Remove execution output from validation cell in detection notebook - ([aced59d](https://github.com/koharu-rs/koharu/commit/aced59d9c362961a1f49d134a09f32dd59887551))

### 📚 Documentation

- Update README to include browser usage information - ([3c19aa4](https://github.com/koharu-rs/koharu/commit/3c19aa4fadef9862a57a632f62ee29a0344a7329))
- Update README to include instructions for enabling CUDA acceleration - ([6405f90](https://github.com/koharu-rs/koharu/commit/6405f9065956cf0541e271b9a05db593aa942015))
- Update README to include CUDA acceleration instructions and remove obsolete guidance - ([e5ad5a2](https://github.com/koharu-rs/koharu/commit/e5ad5a2ffa11a4466b2a4432f444c6d0a6b814a1))
- Add CUDA acceleration feature instructions to README - ([fc6ee3a](https://github.com/koharu-rs/koharu/commit/fc6ee3adb83b1b5c52d78a6dc033855864642b90))
- Add instructions for creating a debug build in README - ([b2412c7](https://github.com/koharu-rs/koharu/commit/b2412c7a251b80a55f70bf749c09c8897ddd4b38))
- Add Tauri prerequisites installation instructions to README - ([1ef16ef](https://github.com/koharu-rs/koharu/commit/1ef16efe205661f881ccff91eddfb498b566e832))
- Update workflow steps and model references in README - ([8c267b8](https://github.com/koharu-rs/koharu/commit/8c267b856fc07e1a3c9e6afa2966194a391bcbec))
- Add LaMa inpainting model to the models section of README - ([434c249](https://github.com/koharu-rs/koharu/commit/434c249f0459dafc8c5059a73031e33b32caacbe))
- Add Discord support note and fix punctuation in development status note - ([c5c1d31](https://github.com/koharu-rs/koharu/commit/c5c1d31cd73675c21842ae2f755ccd30cf5b5919))
- Add note about development status and issue reporting in README.md - ([c97b9dc](https://github.com/koharu-rs/koharu/commit/c97b9dc2d07304c4429feab3fcae9550656247c5))
- Update workflow section to use checklist format - ([5e8b6aa](https://github.com/koharu-rs/koharu/commit/5e8b6aa95df1d553011535e8941ca127ee33368a))
- Update README with preview section and download instructions - ([16a2704](https://github.com/koharu-rs/koharu/commit/16a27047762630e37ed7f8132fb2c860e441cecc))

### ⚡ Performance

- Use multi-thread wasm - ([a0b1863](https://github.com/koharu-rs/koharu/commit/a0b18636789ae564552a84c0cf74d20c5ae6a678))

### 🎨 Styling

- Simplify function calls and improve readability in DetectionPanel and Topbar components - ([bb26b56](https://github.com/koharu-rs/koharu/commit/bb26b56bb5942c443e317e5e90deb84d0e0040f4))
- Update height handling in OCRPanel and TranslationPanel for better layout consistency - ([a60a294](https://github.com/koharu-rs/koharu/commit/a60a2940b24dc896483c54734a5679929cc63895))
- Simplify button component in TranslationPanel for cleaner code - ([c0d1ac4](https://github.com/koharu-rs/koharu/commit/c0d1ac4fef3f109074ac4d9f50b45bdf44949b6e))
- Refine scale adjustment increments in ScaleControl for smoother user experience - ([597a7f2](https://github.com/koharu-rs/koharu/commit/597a7f2c45cd86e33fcdd52e7f3a0535137657bd))
- Adjust height and overflow handling in OCRPanel and TranslationPanel for improved layout - ([dc8adb3](https://github.com/koharu-rs/koharu/commit/dc8adb3bfa3e8bcb838026de8001331123c35c3c))
- Update Canvas component layout for improved alignment and overflow handling - ([6e58b52](https://github.com/koharu-rs/koharu/commit/6e58b52fb50bd8b10e47bb9920789979da129559))
- Increase width of Tools component and adjust layout in main application for improved usability - ([a6c74e2](https://github.com/koharu-rs/koharu/commit/a6c74e2d302e2b2f566dd33f74b7c6af7f901b83))
- Update SplashScreen component with new design and branding elements - ([8ec1e2d](https://github.com/koharu-rs/koharu/commit/8ec1e2df1b9bb2cebdebbbd9a50eb9792af4c61f))

### 🧪 Testing

- Add mask processing and saving functionality in main.rs - ([dbff208](https://github.com/koharu-rs/koharu/commit/dbff208f6bf88e8e55b15f2908eab107072189df))

### ⚙️ Miscellaneous Tasks

- Bump version to 0.1.1 - ([c732e9b](https://github.com/koharu-rs/koharu/commit/c732e9b132d76f47bfd95174a7cc1d9e794ed7e1))
- Add prefix for windows bundle - ([df25f5e](https://github.com/koharu-rs/koharu/commit/df25f5e7909500d1bb1489b15447f7751ca218df))
- Fix version - ([35af04a](https://github.com/koharu-rs/koharu/commit/35af04aa414318827915e5cc974968a81896ab5d))
- Fix windows bundle - ([040f942](https://github.com/koharu-rs/koharu/commit/040f942a84e515f2dca4b6a71c9cc8e1f764566b))
- Add distDir and output configuration to next.config.ts - ([9c8e278](https://github.com/koharu-rs/koharu/commit/9c8e2784592dd5ddc4d8a04e7da7408012fd372a))
- Trigger build - ([d7e4270](https://github.com/koharu-rs/koharu/commit/d7e42707ffa514f81b95dd80d18dc34115c77009))
- Test bundle - ([d590e08](https://github.com/koharu-rs/koharu/commit/d590e085efb228ed1158a13e0b891d86c4c994c1))
- Use workspace dependencies - ([3cbcb60](https://github.com/koharu-rs/koharu/commit/3cbcb60a4bb12b871380db9639fde688a78ef837))
- Add example for lama - ([2771e07](https://github.com/koharu-rs/koharu/commit/2771e07aef21d078755d4423f2a4a32e4f10dc74))
- Add example for manga-ocr - ([8c8d3b3](https://github.com/koharu-rs/koharu/commit/8c8d3b38b2abddcb5634767f23740811fc93ab0f))
- Format members list in Cargo.toml for consistency - ([4760ae3](https://github.com/koharu-rs/koharu/commit/4760ae33568591844e98090e22772d04bd197a88))
- Add example for ctd - ([93fec31](https://github.com/koharu-rs/koharu/commit/93fec3176e3859d73ba3bb2d30cad351d7dec8b4))
- Add sub modules - ([31b311b](https://github.com/koharu-rs/koharu/commit/31b311b1c588ed5fc83730dff01a2ee58babaa83))
- Update deps - ([5ce4cb8](https://github.com/koharu-rs/koharu/commit/5ce4cb85716ab9ee3dcdb8fdfe4d48e4870a98c7))
- Add icon for web - ([9075183](https://github.com/koharu-rs/koharu/commit/90751831624026c5620a91e135ec3a391af020e0))
- Add 'use client' directive to inpaint.ts - ([2661f4d](https://github.com/koharu-rs/koharu/commit/2661f4d7a95f8dce53cb7054320c88efc2ea6af4))
- Update dependencies and configuration files - ([3f0debc](https://github.com/koharu-rs/koharu/commit/3f0debccb27d1f7ace3216d2db3e73836ea3c288))
- Update dependencies to latest versions in bun.lock and package.json - ([24a7057](https://github.com/koharu-rs/koharu/commit/24a7057b40346536ea0bdeeada7ab28b1bcd0da5))
- Add web deployment - ([f554269](https://github.com/koharu-rs/koharu/commit/f554269af29100dea1430dcb36c9e8792312d2c0))
- Remove splashscreen window configuration and related initialization logic - ([84dd59d](https://github.com/koharu-rs/koharu/commit/84dd59d7a1aeee6d168a30ff3df212dd8e53f338))
- Update API settings documentation and improve placeholder text in form inputs - ([160375b](https://github.com/koharu-rs/koharu/commit/160375b461471aba363d3a8ef2ffb18d24285864))
- Replace textarea with TextArea component in TranslationPanel for consistency - ([6a2fcfd](https://github.com/koharu-rs/koharu/commit/6a2fcfd81e5746e99ffa9cc259c890566b629277))
- Conditionally render inpaint image based on selected tool - ([741abac](https://github.com/koharu-rs/koharu/commit/741abac5edd9d6ece9ae4eda77a033ac5f725a3a))
- Remove unused download button from topbar - ([de0bdae](https://github.com/koharu-rs/koharu/commit/de0bdaec6863c53bba4fc406cabc937846f1b326))
- Adjust logging level for event loop runner to Error - ([7333db0](https://github.com/koharu-rs/koharu/commit/7333db002c68710251dd041e1b295b361c33beae))
- Remove loading spinner from splash screen - ([d59ddd7](https://github.com/koharu-rs/koharu/commit/d59ddd7a5e282b199cd5da84d274dec9863288c8))
- Remove unused dependencies from Cargo.lock - ([67c36ac](https://github.com/koharu-rs/koharu/commit/67c36acb52596c0b0b4ad448460b5c18244b5603))
- Update workspace members and add .gitattributes for language detection - ([cf993f0](https://github.com/koharu-rs/koharu/commit/cf993f0afd3946819767b4074b301858d9e97c97))
- Cleanup - ([1a893b9](https://github.com/koharu-rs/koharu/commit/1a893b98b76d1472726d67f6306c6fa9a1c8e5e3))
- Use re-upload models - ([9848e54](https://github.com/koharu-rs/koharu/commit/9848e54fb96a1b5286dab9709aef966ef7198fff))
- Remove obsolete Paperspace credentials from example env file - ([16c0fcd](https://github.com/koharu-rs/koharu/commit/16c0fcd509885c08f4f6980b644719b5768787bd))
- Remove obsolete Paperspace automation script - ([8ea2475](https://github.com/koharu-rs/koharu/commit/8ea2475fc850b4dc4cbafaabe9c2b204a6a6807e))
- Restructure files - ([0f9dd41](https://github.com/koharu-rs/koharu/commit/0f9dd417f4bd8e937ca39ab166c3578cc8a9963a))
- Improve splashscreen - ([1e57d9a](https://github.com/koharu-rs/koharu/commit/1e57d9ad2d5b6d8c3252d23ef7288d8b40cb81ce))
- Update deps - ([47504af](https://github.com/koharu-rs/koharu/commit/47504af1f027ca1ef0f8507ee6f3855c58237082))
- Add test batch - ([b75caa6](https://github.com/koharu-rs/koharu/commit/b75caa63ecea8afcc06ad8ad6adb528d617359d6))
- Inference python scripts - ([4f0ac89](https://github.com/koharu-rs/koharu/commit/4f0ac89c8d6a654aa2e3d0ab85d16d0529426214))
- Rename model_path to model in CLI arguments for consistency - ([32ecf9d](https://github.com/koharu-rs/koharu/commit/32ecf9d22a1c7dd36c5e8f2633b989d3abac6aba))
- Set default feature to CUDA in Cargo.toml - ([e1796fd](https://github.com/koharu-rs/koharu/commit/e1796fd1b6846aa98d28af1fdc1d5802b59d864b))
- Move execution provider initialization to main function - ([fa63522](https://github.com/koharu-rs/koharu/commit/fa63522fee3f45e0959ab8be752ecd440c0451c7))
- Cleanup lama inference code - ([58f9df5](https://github.com/koharu-rs/koharu/commit/58f9df504d85618bed8e7e0cd4bf8694556b3944))
- Switch to a generic dotenv file ([#4](https://github.com/koharu-rs/koharu/issues/4)) - ([7a82465](https://github.com/koharu-rs/koharu/commit/7a824652e56b4ee2f733770720b2f71cc970ef5b))
- Remove unused useRef import in canvas component - ([e2cc9cb](https://github.com/koharu-rs/koharu/commit/e2cc9cba33fbb39c66310b6cc5ed59eb5d7239a7))
- Add tag trigger for versioned releases in build workflow - ([efc5114](https://github.com/koharu-rs/koharu/commit/efc51148cc41bb6cad4d971c36279ef1990473dd))
- Update publish workflow to trigger on version tags only - ([475be6c](https://github.com/koharu-rs/koharu/commit/475be6c030b73c323e4a48d4d4dc571a9c882fc4))
- Update macOS and Windows platform specifications in build configuration - ([cfe146c](https://github.com/koharu-rs/koharu/commit/cfe146cd237a0fb99b5bab4fa672655abd3b3e1f))
- Add "prettier-plugin-tailwindcss" and format - ([94690ee](https://github.com/koharu-rs/koharu/commit/94690ee21f994d44e933d670fb185fe4293fa966))
- Auto-trigger inference when imageSrc changes - ([ee1cea3](https://github.com/koharu-rs/koharu/commit/ee1cea351ded5ff7b9bda3c1d400829856a8157a))
- Rename job from publish-tauri to build-tauri in workflow - ([0fe3615](https://github.com/koharu-rs/koharu/commit/0fe36159e94fb21dbfebdf2251923964c7070c93))
- Add workflow for PR - ([2edcd4e](https://github.com/koharu-rs/koharu/commit/2edcd4e40b6aee5a54e5bae5510196cb9eaa085e))
- Update README and configuration for Windows setup and Tauri CLI dependencies - ([580b3f3](https://github.com/koharu-rs/koharu/commit/580b3f369b41fce136d3ec001fc629ad5e802d33))
- Upload portable executable - ([1bd1416](https://github.com/koharu-rs/koharu/commit/1bd141646082efa63197cdd0b71a3b4125685dca))
- Add rust cache - ([b12e77d](https://github.com/koharu-rs/koharu/commit/b12e77d33a1fb13ae7d8e12fe9f47fc7b79a2a16))
- Tauri just output the executable file - ([488c688](https://github.com/koharu-rs/koharu/commit/488c688475182e93195d19974a254c11a72241d2))
- Add prettier - ([50f8002](https://github.com/koharu-rs/koharu/commit/50f800242f77a68f0e8a6202fd378917075de1ea))
- Update dependencies to latest versions - ([5f949f1](https://github.com/koharu-rs/koharu/commit/5f949f1bf137976793b45036225c5dfdca456d06))
- Reorganize README structure and add models section - ([05d65f9](https://github.com/koharu-rs/koharu/commit/05d65f937ed6432a90583559b6ba20cd37858fb9))
- Update @types/react and vite to latest versions - ([09a9301](https://github.com/koharu-rs/koharu/commit/09a9301f7dc8e506f9f6244738fc56c2a2a91eca))
- Update dependencies to specific versions in bun.lock - ([f6b6f69](https://github.com/koharu-rs/koharu/commit/f6b6f6979f7c05dc37290b0202fb1d14520edc2e))

### Pref

- Use rwlock - ([cad2f74](https://github.com/koharu-rs/koharu/commit/cad2f742094885156d7b6af955c951837dd11ea6))

### Wip

- Add inpaint inference - ([4641857](https://github.com/koharu-rs/koharu/commit/4641857bf71faac327e92a48fcc0e6a4478cca98))
- Add ocr to web - ([a80b61b](https://github.com/koharu-rs/koharu/commit/a80b61b54f07926d3b8081105546f178ba22f399))


<!-- generated by git-cliff -->
