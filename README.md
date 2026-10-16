<p align="center"><img src="mentis-logo.svg" alt="Mentis logo" width="200"></p>

# Mentis

Offline dictation for Windows. Hold a key, speak, release: the text appears in the active text field of any app. Speech recognition runs locally on your GPU or CPU; nothing you say leaves your PC.

- Whisper or Parakeet via [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp), GPU through Vulkan, CPU fallback
- No account, no cloud, no installer: one folder to unzip
- German or English interface, following the Windows language
- Free software under the [MIT License](LICENSE)

## Install

Requires Windows 10 or 11 (64-bit), a microphone and 0.3–0.9 GB for the speech model.

1. Download the latest `Mentis-<version>.zip` from [Releases](../../releases).
2. Right-click the zip > Properties > tick "Unblock" > OK.
3. Extract it to any folder. Do not run Mentis from inside the zip.
4. Start `mentis.exe`. If Windows shows "Windows protected your PC": "More info" > "Run anyway".
5. "Set up Mentis" suggests a model for your PC. Pick a location and click "Download".

Mentis then runs as an icon in the notification area.

**Windows warnings:** Mentis is not code-signed, so Windows does not know the publisher. Some antivirus tools also flag its keyboard hook. The source is all here; you can [build it yourself](#build-from-source).

**Smart App Control** (Windows 11) blocks unsigned programs without a "Run anyway" option. Mentis only runs with it turned off.

**Privacy:** Mentis goes online only to download speech models from [huggingface.co/handy-computer](https://huggingface.co/handy-computer) and, once a day, to ask GitHub for a newer version (can be turned off in the settings; nothing is installed automatically). Audio and text stay on your PC; the log never contains dictated text.

**Coming from Echo:** Mentis was called Echo up to version 0.7. Quit Echo, then start Mentis: it takes over Echo's settings, models and autostart once. Then delete the old Echo folder.

**Uninstall:** right-click the icon > "Remove Mentis …", then delete the program folder.

## Usage

| Action | How |
|---|---|
| Dictate | Hold **right Ctrl**, speak, release |
| Longer dictation | Tap right Ctrl, speak, tap again |
| Cancel | Esc while recording |
| Settings | Click the icon |
| Menu | Right-click the icon |

The settings window covers everything: key, microphone, writing style (clean or exact), language, model, GPU/CPU, autostart, update check. Settings are stored in `%APPDATA%\Mentis\config.toml`, the log in `%LOCALAPPDATA%\Mentis`.

| Model | Size | Suggested for | License |
|---|---:|---|---|
| Whisper large-v3-turbo | 845 MB | GPU with 2 GB or more | Apache-2.0 |
| Parakeet TDT 0.6B v3 | 524 MB | no GPU, 25 European languages | CC-BY-4.0 |
| Whisper small | 257 MB | no GPU, other languages | Apache-2.0 |

Mentis cannot type into windows running as administrator.

## Build from source

One-time setup:

```powershell
winget install --id Microsoft.VisualStudio.BuildTools --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup
winget install --id KhronosGroup.VulkanSDK
winget install --id Git.Git
```

Then, in a new PowerShell:

```powershell
git clone https://github.com/Farbfinsternis/mentis.git
cd mentis
Set-ExecutionPolicy -Scope Process Bypass
.\scripts\Package.ps1
```

Result: `dist\Mentis` and `dist\Mentis-<version>.zip`. Options: `-CpuOnly` (no Vulkan SDK needed, CPU only), `-NoBuild` (package the existing build).

## Development

```powershell
. .\scripts\Enter-DevEnvironment.ps1
cargo fmt --all -- --check
cargo clippy --release --locked --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo test --release --locked --target x86_64-pc-windows-msvc
```

Test helpers: `mentis.exe --devices`, `--transcribe file.wav`, `--paste-test`, `--download <model>`, `--setup`, `--config <path>`. Append `| Out-Host` in PowerShell.

## Credits

Mentis is designed and directed by Farbfinsternis, who acts as its architect. All of the code was written by Claude (Anthropic), an AI model.

## License

MIT, see [LICENSE](LICENSE). Bundled components (transcribe.cpp, ggml, miniz, Silero VAD, Rust crates) keep their own licenses, collected in `LICENSES.txt` in each release. Speech models are not bundled and have their own licenses.
