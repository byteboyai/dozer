#!/usr/bin/env bash
# Spike helper: capture each goose hook event's raw stdin JSON, one line per
# event, appended to ~/.dozer-spike-capture.log. stdout stays empty (goose
# reads stdout as the decision channel; this plugin is observation-only).
set -u
payload="$(cat)"
printf '%s\n' "$payload" >> "${HOME}/.dozer-spike-capture.log"
exit 0
