#!/usr/bin/env bash
# Скачивает корпуса для обучения (tools/trainer) и проверки (tools/eval) в ./corpora.
# Нужны только чтобы пересобрать модели — готовые модели лежат в crates/engine/data.
#
#   bash tools/fetch-corpora.sh
#
# Источники:
#   FrequencyWords (OpenSubtitles 2018), CC-BY-SA 4.0 — https://github.com/hermitdave/FrequencyWords
#   Tatoeba, CC-BY 2.0 FR — https://tatoeba.org
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p corpora
cd corpora

fetch() {
    local url="$1" out="$2"
    if [[ -s "$out" ]]; then
        echo "есть:     $out"
    else
        echo "качаю:    $out"
        curl -fL --retry 3 -o "$out.part" "$url"
        mv "$out.part" "$out"
    fi
}

fw=https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018
fetch "$fw/ru/ru_full.txt" ru_freq.txt
fetch "$fw/en/en_full.txt" en_freq.txt

tat=https://downloads.tatoeba.org/exports/per_language
for lang in rus eng; do
    if [[ ! -s "${lang}_sentences.tsv" ]]; then
        fetch "$tat/$lang/${lang}_sentences.tsv.bz2" "${lang}_sentences.tsv.bz2"
        bzip2 -d "${lang}_sentences.tsv.bz2"
    fi
done

ls -la
