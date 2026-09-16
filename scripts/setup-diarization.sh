#!/usr/bin/env bash
# Downloads the speaker-diarization toolchain into models/diarization/:
#   - sherpa-onnx's offline diarization binary (macOS arm64, prebuilt)
#   - pyannote segmentation 3.0 (ONNX)
#   - NeMo TitaNet-small speaker embeddings (ONNX, English)
# About 60 MB in total. Everything runs locally; nothing needs an account or token.
#
# Usage: scripts/setup-diarization.sh
set -euo pipefail

version="1.13.8"
root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/models/diarization"
mkdir -p "$dest"
cd "$dest"

if [[ "$(uname -s)-$(uname -m)" != "Darwin-arm64" ]]; then
  echo "prebuilt binary is for Apple Silicon macOS; see https://github.com/k2-fsa/sherpa-onnx/releases" >&2
  echo "and point DIARIZE_BIN at sherpa-onnx-offline-speaker-diarization" >&2
  exit 1
fi

releases="https://github.com/k2-fsa/sherpa-onnx/releases/download"

if [[ ! -x bin/sherpa-onnx-offline-speaker-diarization ]]; then
  echo "↓ sherpa-onnx $version"
  pkg="sherpa-onnx-v$version-osx-arm64-shared-no-tts"
  curl -fsSL "$releases/v$version/$pkg.tar.bz2" | tar xj
  rm -rf bin lib
  mv "$pkg/bin" "$pkg/lib" .
  rm -rf "$pkg"
fi

if [[ ! -f segmentation.onnx ]]; then
  echo "↓ pyannote segmentation 3.0"
  curl -fsSL "$releases/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2" \
    | tar xj sherpa-onnx-pyannote-segmentation-3-0/model.onnx
  mv sherpa-onnx-pyannote-segmentation-3-0/model.onnx segmentation.onnx
  rmdir sherpa-onnx-pyannote-segmentation-3-0
fi

if [[ ! -f embedding.onnx ]]; then
  echo "↓ TitaNet-small speaker embeddings"
  curl -fsSL -o embedding.onnx "$releases/speaker-recongition-models/nemo_en_titanet_small.onnx"
fi

echo "✓ diarization ready in models/diarization"
