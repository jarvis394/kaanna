# Käännä

**Live, on-device Finnish → English translation in a floating window.** Käännä listens to your
Mac's system audio (or any input device), transcribes Finnish speech with Whisper and translates
it to English with a Marian NMT model. After a one-time model download everything runs locally,
with no cloud services.

Built with Tauri 2, React and [Base UI](https://base-ui.com). macOS (Apple Silicon) first;
the pipeline is cross-platform apart from system-audio capture.

## How it works

```
 System audio (ScreenCaptureKit) ─┐
 Input device (cpal)  ────────────┴─► 16 kHz mono ─► Silero VAD ─► Whisper ─► OPUS-MT ─► window
                                                    (segmenter)   (Metal)    (candle)
```

| Stage | Model | Runtime |
|---|---|---|
| Voice activity / sentence splitting | [Silero VAD v6](https://huggingface.co/ggml-org/whisper-vad) | whisper.cpp |
| Finnish speech recognition | [Finnish-NLP/whisper-large-finnish-v3](https://huggingface.co/Finnish-NLP/whisper-large-finnish-v3) (ggml) | whisper.cpp via `whisper-rs`, Metal |
| Finnish → English | [Helsinki-NLP/opus-mt-tc-big-fi-en](https://huggingface.co/Helsinki-NLP/opus-mt-tc-big-fi-en) | `candle` (Accelerate) |

* The **segmenter** cuts audio into utterances at pauses and, while someone is still talking,
  sends *drafts* so the window shows a live preview. If the machine falls behind, drafts are
  dropped rather than queued.
* Whisper's audio context is trimmed to the utterance length, which makes short sentences several
  times faster to encode.
* Model downloads are pinned to exact Hugging Face commits and verified by SHA-256. They are
  resumable and stored in `~/Library/Application Support/dev.kaanna.app/models`.

Measured on a MacBook Air M1 (16 GB) with the default Q5 model: 9.7 s of speech transcribed in
~2.3 s and translated in ~1 s. A typical sentence appears 1–2 s after the speaker pauses.

### Speech models

Pick one in Settings → Speech model:

* **Whisper Large v3 Finnish · Q5** (1.1 GB, default): 5-bit quantisation of the official
  Finnish-NLP ggml conversion (community upload, checksum-pinned).
* **Whisper Large v3 Finnish · F16** (3.1 GB): the official
  [Finnish-NLP ggml conversion](https://huggingface.co/Finnish-NLP/Finnish-finetuned-whisper-models-ggml-format).
* **Whisper Medium Finnish** (1.5 GB): official, faster, less accurate.

## Development

Requirements: macOS 13+, Xcode command line tools, Rust (stable), [Bun](https://bun.sh), CMake.

```bash
bun install
bun run tauri dev
```

The first build compiles whisper.cpp and candle and takes a few minutes. Dependencies are always
built with optimisations, even in dev, because unoptimised inference is unusably slow.

`bun run dev` on its own serves the UI in a normal browser with a **demo backend**
(`src/lib/demo.ts`) that plays a scripted transcript. Use it for UI work without Rust or models.

### Tests

```bash
cd src-tauri
cargo test                                        # unit tests
cargo test --release -- --ignored --nocapture     # end-to-end, uses the downloaded models
```

The end-to-end tests download the MT model if it's missing, transcribe and translate
`tests/fixtures/fi_sample.wav` (generated with macOS's Finnish `say` voice), and stream it through
the live VAD pipeline.

### Permissions

* **System audio** uses ScreenCaptureKit and needs *System Settings → Privacy & Security →
  Screen & System Audio Recording*. During `tauri dev` the permission is granted to your terminal
  app. Restart the app after granting it.
* **Input devices** need microphone permission.

### Release builds

`.github/workflows/build.yml` builds installers for macOS (Apple Silicon and Intel), Windows
(x64: `.msi` and `.exe` installers, plus a `_portable.zip` that runs without installing) and
Linux (x64: `.deb`, `.rpm`, `.AppImage`). Bundles and install folders are named `Kaanna`, in
plain ASCII; the app still calls itself Käännä. The Windows installers always install into a
`Kaanna` folder, even when you pick a parent folder like `D:\Apps`, and keep an existing
install where it is.

* **Push a tag** such as `v0.2.0` to build everything and attach it to a *draft* GitHub release.
  Bump `version` in `src-tauri/tauri.conf.json` first, since that's what the file names use.
* **Run it manually** from Actions → Build → *Run workflow* to get the installers as workflow
  artifacts without a release.

Metal and Accelerate are macOS-only. On Windows, Whisper runs on the GPU through Vulkan, falling
back to the CPU without one. *Settings → Graphics processor* picks a specific GPU or *CPU only*;
*Automatic* prefers a dedicated GPU. System audio on Windows is captured with WASAPI loopback
from the default output device. On Linux, Whisper runs on the CPU, which is much slower, and
only input devices can be captured. Translation always runs on the CPU (with Accelerate on
macOS); it's fast enough everywhere.

Windows release builds enable the `vulkan` Cargo feature, which needs the
[Vulkan SDK](https://vulkan.lunarg.com/sdk/home#windows) to build. Without it,
`bun run tauri build` produces a CPU-only build; add `--features vulkan` for GPU support.

The builds aren't signed with a developer certificate. On macOS they're signed ad hoc
(`signingIdentity: "-"` in `tauri.conf.json`): Apple Silicon refuses to run a bundle that isn't
signed at all, reporting it as "damaged". The hardened runtime is off, since it's only needed
for notarization and would block the microphone without extra entitlements.

To open it the first time on macOS, try to open the app, then click *Open Anyway* under
*System Settings → Privacy & Security*. If macOS still says the app is damaged, clear the
download quarantine with `xattr -dr com.apple.quarantine /Applications/Kaanna.app`. Ad-hoc
signatures change with every build, so macOS asks for the microphone and screen-recording
permissions again after each update. On Windows, click past SmartScreen with
*More info → Run anyway*.

### Notes

* `candle` is pinned to `0.10.2`: `0.11` uses the `stdarch_neon_f16` intrinsics, which this
  project's current Rust toolchain (1.93) doesn't support on stable. After a Rust upgrade
  (`rustup update`), bump candle and add a `rust-toolchain.toml`.
* `src-tauri/build.rs` adds `/usr/lib/swift` to the binary's rpath, which the ScreenCaptureKit
  Swift bridge needs at runtime.

## Layout

```
src/                     React UI (Base UI components)
  App.tsx                window chrome, state, events
  components/            source picker, transcript, settings, model setup
  lib/api.ts             typed Tauri commands/events
  lib/demo.ts            browser-only fake backend
src-tauri/src/
  audio/                 ScreenCaptureKit + cpal capture, resampling
  pipeline.rs            VAD segmentation, ASR/MT workers, events
  asr.rs                 whisper.cpp wrapper + hallucination filter
  mt.rs, spm.rs          Marian translation + minimal SentencePiece encoder
  models.rs              model catalog and verified downloader
```
