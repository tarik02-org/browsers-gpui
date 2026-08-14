#!/usr/bin/env bash

set -euo pipefail

release_timezone="${RELEASE_TIMEZONE:-Europe/Kyiv}"
read -r year month day < <(TZ="$release_timezone" date '+%Y %-m %-d')

day_patch=$((day * 100))
next_daily_release=0

while IFS= read -r tag; do
  IFS=. read -r tag_year tag_month tag_patch remainder <<< "$tag"

  if [[ "$tag_year" != "$year" || "$tag_month" != "$month" ]]; then
    continue
  fi

  if [[ -n "${remainder:-}" || ! "$tag_patch" =~ ^[0-9]+$ ]]; then
    continue
  fi

  patch=$((10#$tag_patch))
  if ((patch < day_patch || patch > day_patch + 99)); then
    continue
  fi

  daily_release=$((patch - day_patch))
  if ((daily_release >= next_daily_release)); then
    next_daily_release=$((daily_release + 1))
  fi
done < <(git tag --list "${year}.${month}.*")

if ((next_daily_release > 99)); then
  echo "No release numbers remain for ${year}-${month}-${day} in ${release_timezone}." >&2
  exit 1
fi

printf '%s.%s.%d\n' "$year" "$month" "$((day_patch + next_daily_release))"
