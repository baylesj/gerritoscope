# gerritoscopy

![CI](https://github.com/baylesj/gerritoscopy/actions/workflows/ci.yml/badge.svg)

A Rust CLI — and reusable GitHub Action — that fetches contribution data from one or
more Gerrit instances and renders a GitHub-profile heatmap card as an SVG, plus an
optional markdown report.

```
┌────────────────────────────────────────────────────────────┐
│  gerritoscopy · you@example.com                            │
│  hosts: chromium                                           │
└────────────────────────────────────────────────────────────┘

  Jan ····· Feb ····· Mar ····· … Dec
  [░░▒▒▓▓██░░▒▓░░░░▒▒▓▓██░░░░░░▒▒]

  Merged CLs       1,234  (all time)
  Last 90 days        42
  Lines changed  +98,765 / -12,345
  Streak         current 4 wk  ·  longest 12 wk
```

## Prerequisites

- A Gerrit account with an email address, username, or the ability to use `self`.
- For private Gerrit instances: an HTTP password (generated in Gerrit → Settings →
  HTTP Credentials) and your username.
- To commit the SVG automatically: a GitHub personal access token or the default
  `GITHUB_TOKEN` with write access to your profile repo.

## Usage as a GitHub Action

Add a workflow to your profile repository (typically `<username>/<username>`) that
runs on a schedule and commits the generated SVG back:

```yaml
# .github/workflows/gerrit-heatmap.yml
name: Update Gerrit heatmap

on:
  schedule:
    - cron: "0 4 * * *"   # daily at 04:00 UTC
  workflow_dispatch:

jobs:
  update:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Generate heatmap
        uses: baylesj/gerritoscopy@main
        with:
          owner: you@example.com      # your Gerrit account email
          hosts: chromium             # comma-separated aliases or URLs
          output-svg: gerrit-heatmap.svg
          svg-theme: github

      - name: Commit updated SVG
        uses: stefanzweifel/git-auto-commit-action@v5
        with:
          commit_message: "chore: update Gerrit heatmap"
          file_pattern: gerrit-heatmap.svg
```

### Inputs

| Input | Required | Default | Description |
|-------|----------|---------|-------------|
| `owner` | yes | — | Gerrit account email, username, or `self` |
| `hosts` | no | `chromium` | Comma-separated host aliases or full URLs |
| `after` | no | — | Only include changes on/after this date (`YYYY-MM-DD`) |
| `username` | no | — | HTTP Basic Auth username (private instances) |
| `password` | no | — | HTTP password (paired with `username`) |
| `output-svg` | no | `gerrit-heatmap.svg` | Output path for the SVG card |
| `output-md` | no | — | Output path for a markdown report |
| `svg-theme` | no | `github` | Color theme (see Themes below) |
| `svg-multi-color` | no | `false` | Color cells by Gerrit host/project family |

### Using credentials for private instances

Store your HTTP password as a repository secret (`GERRIT_PASSWORD`) and pass it in:

```yaml
- uses: baylesj/gerritoscopy@main
  with:
    owner: your-username
    hosts: https://gerrit.example.com
    username: your-username
    password: ${{ secrets.GERRIT_PASSWORD }}
    output-svg: gerrit-heatmap.svg
```

## In your README

After the workflow runs, embed the SVG in your profile README:

```html
<img src="gerrit-heatmap.svg" alt="Gerrit contribution heatmap" />
```

## Themes

| Theme | Description |
|-------|-------------|
| `github` | GitHub default (auto light/dark) |
| `github-light` | GitHub light mode |
| `github-dark` | GitHub dark mode |
| `solarized-light` | Solarized light |
| `solarized-dark` | Solarized dark |
| `gruvbox-dark` | Gruvbox dark |
| `gruvbox-light` | Gruvbox light |
| `tokyo-night` | Tokyo Night |
| `dracula` | Dracula |
| `catppuccin-mocha` | Catppuccin Mocha |

## Supported Gerrit hosts

Short aliases you can pass to `hosts`:

| Alias | Instance |
|-------|----------|
| `chromium` | chromium-review.googlesource.com |
| `go` | go-review.googlesource.com |
| `android` | android-review.googlesource.com |
| `fuchsia` | fuchsia-review.googlesource.com |
| `skia` | skia-review.googlesource.com |
| `gerrit` | gerrit-review.googlesource.com |
| `wikimedia` | gerrit.wikimedia.org |
| `qt` | codereview.qt-project.org |
| `libreoffice` | gerrit.libreoffice.org |
| `onap` | gerrit.onap.org |

Multiple hosts: `hosts: "chromium,go"` or repeat entries as needed.

## Local CLI

Install from source:

```bash
cargo install --git https://github.com/baylesj/gerritoscopy
```

Example invocations:

```bash
# Fetch from Chromium and write an SVG
gerritoscopy --owner you@example.com --output-svg heatmap.svg

# Multiple hosts, dark theme
gerritoscopy --owner you@example.com --hosts chromium,go --svg-theme github-dark --output-svg heatmap.svg

# Private instance with Basic Auth
gerritoscopy --owner your-username \
  --hosts https://gerrit.example.com \
  --username your-username \
  --password your-http-password \
  --output-svg heatmap.svg

# Only changes since a given date
gerritoscopy --owner you@example.com --after 2024-01-01 --output-svg heatmap.svg
```

## License

MIT
