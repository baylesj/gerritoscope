#!/bin/bash
set -euo pipefail
cd "${GITHUB_WORKSPACE:-/github/workspace}"

# GitHub Actions passes Docker action inputs with hyphens preserved in the env
# var name (e.g. INPUT_OUTPUT-SVG), not converted to underscores. Bash cannot
# reference hyphenated var names with ${} syntax, so we use printenv.
hosts="$(printenv INPUT_HOSTS || true)"
after="$(printenv INPUT_AFTER || true)"
username="$(printenv INPUT_USERNAME || true)"
password="$(printenv INPUT_PASSWORD || true)"
output_svg="$(printenv 'INPUT_OUTPUT-SVG' || true)"
output_md="$(printenv 'INPUT_OUTPUT-MD' || true)"
svg_theme="$(printenv 'INPUT_SVG-THEME' || true)"
svg_multi_color="$(printenv 'INPUT_SVG-MULTI-COLOR' || true)"

args=(--owner "${INPUT_OWNER}")

[[ -n "$hosts"         ]] && args+=(--hosts          "$hosts")
[[ -n "$after"         ]] && args+=(--after           "$after")
[[ -n "$username"      ]] && args+=(--username        "$username")
[[ -n "$password"      ]] && args+=(--password        "$password")
[[ -n "$output_svg"    ]] && args+=(--output-svg      "$output_svg")
[[ -n "$output_md"     ]] && args+=(--output-md       "$output_md")
[[ -n "$svg_theme"     ]] && args+=(--svg-theme       "$svg_theme")
[[ "$svg_multi_color" == "true" ]] && args+=(--svg-multi-color)

exec /usr/local/bin/gerritoscope "${args[@]}"
