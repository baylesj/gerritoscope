#!/bin/bash
set -euo pipefail
cd "${GITHUB_WORKSPACE:-/github/workspace}"

args=(--owner "${INPUT_OWNER}")

[[ -n "${INPUT_HOSTS:-}"             ]] && args+=(--hosts          "${INPUT_HOSTS}")
[[ -n "${INPUT_AFTER:-}"             ]] && args+=(--after          "${INPUT_AFTER}")
[[ -n "${INPUT_USERNAME:-}"          ]] && args+=(--username       "${INPUT_USERNAME}")
[[ -n "${INPUT_PASSWORD:-}"          ]] && args+=(--password       "${INPUT_PASSWORD}")
[[ -n "${INPUT_OUTPUT_SVG:-}"        ]] && args+=(--output-svg     "${INPUT_OUTPUT_SVG}")
[[ -n "${INPUT_OUTPUT_MD:-}"         ]] && args+=(--output-md      "${INPUT_OUTPUT_MD}")
[[ -n "${INPUT_SVG_THEME:-}"         ]] && args+=(--svg-theme      "${INPUT_SVG_THEME}")
[[ "${INPUT_SVG_MULTI_COLOR:-false}" == "true" ]] && args+=(--svg-multi-color)

exec /usr/local/bin/gerritoscopy "${args[@]}"
