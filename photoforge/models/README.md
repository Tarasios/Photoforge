# models/

ONNX model files for the ML-backed features. **Nothing in this folder ships in
git by default** — drop the models in locally. Everything runs offline via the
`ort` crate (ONNX Runtime); there is no Python anywhere in this repo.

## Status

The ML phases (P2.2 ONNX screenshot classifier, P3.1 face pipeline) are **not
yet integrated** because they require model files and their normalization
constants, which are not part of the repo. The heuristic classifier (Phase 2
tier 1) works without any model. When the models below are added, integrate
`ort` in `photoforge-core` behind a `ml` feature flag.

## Expected files

| File | Purpose | Input | Notes |
|---|---|---|---|
| `screenshot_classifier.onnx` | photo vs. screenshot arbitration for files the heuristics mark `ambiguous` | 224x224 RGB | record normalization mean/std here when chosen |
| `scrfd.onnx` | face detection (SCRFD) | dynamic | source + license required below |
| `arcface.onnx` | 512-dim face embeddings (ArcFace) | 112x112 aligned crop | source + license required below |

## Licensing

For every model added, record here: where it came from (URL), its license, and
any attribution requirements. Do not commit models whose license forbids
redistribution.

## Windows bundling note

ONNX Runtime needs its DLLs shipped next to the app binary
(`onnxruntime.dll`, and `onnxruntime_providers_shared.dll` if using execution
providers). The `ort` crate's `download` feature can fetch them at build time;
for the installer, add them to the Tauri bundle resources.
