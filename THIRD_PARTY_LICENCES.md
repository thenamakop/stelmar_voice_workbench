# Third-party licences

Phase 2 starts this file with the components the speech engines contain. Phase 8 completes it with every
dependency of the app (Rust crates, npm packages, fonts, icons and models) and ships it with the installer.

The repository's allowed licences are MIT, Apache 2.0, BSD, ISC and OFL (AGENTS.md). One named exception
was approved on 10 November 2026: **whisper-rs and whisper-rs-sys are under the Unlicense** (public
domain dedication). The exception covers those two crates only. It does not extend to any other
dependency, and a licence checker, if one is added, must list the two crates individually.

## whisper.cpp v1.9.5 (MIT)

Vendored in `third_party/whisper.cpp` (commit `d1be6fde11ac6e0407606b4e42fe72d34add8037`) and compiled into
every `wb-engine` build, together with ggml.

MIT License

Copyright (c) 2023-2026 The ggml authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## whisper-rs 0.16.0 and whisper-rs-sys 0.15.0 (Unlicense)

Rust bindings to whisper.cpp. whisper-rs-sys is vendored in `third_party/whisper-rs-sys` with three small
changes to its `build.rs` (listed in `third_party/README.md`).

This is free and unencumbered software released into the public domain.

Anyone is free to copy, modify, publish, use, compile, sell, or
distribute this software, either in source code form or as a compiled
binary, for any purpose, commercial or non-commercial, and by any
means.

In jurisdictions that recognize copyright laws, the author or authors
of this software dedicate any and all copyright interest in the
software to the public domain. We make this dedication for the benefit
of the public at large and to the detriment of our heirs and
successors. We intend this dedication to be an overt act of
relinquishment in perpetuity of all present and future rights to this
software under copyright law.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR
OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
OTHER DEALINGS IN THE SOFTWARE.

For more information, please refer to <https://unlicense.org>

## Bench clips in `resources/bench` (CC BY 4.0)

The four 60-second clips the system check times (`bench-en.wav`, `bench-hi.wav`, `bench-ur.wav` and
`bench-codeswitch.wav`) are made of utterances from the test split of FLEURS (Google, "FLEURS: Few-shot
Learning Evaluation of Universal Representations of Speech", Conneau et al., 2022), published under the
Creative Commons Attribution 4.0 International licence (<https://creativecommons.org/licenses/by/4.0/>).
The utterances are joined with 0.8 s of silence and cut at 60 s; nothing else was changed. The list of
utterance ids and the SHA-256 of each file is in `resources/bench/clips.json`, and `scripts/make-bench-clips.py`
rebuilds them. `bench-codeswitch.wav` alternates English and Hindi utterances and stands in for real
code-switched speech.

*Note of 14 Nov 2026 (owner's decision on the review of the Phase 3 pull request):* the four `resources/bench/*.wav`
files are public FLEURS audio, published under CC BY 4.0 and credited above. They are not private audio, so
committing them is allowed under the rule in `AGENTS.md` that bans private audio and model binaries from the
repository. Recordings of real sessions, the Phase 5 local sets and anything under `eval/data/` stay out of Git.

