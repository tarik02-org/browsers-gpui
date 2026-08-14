use std::env;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use chrono::{Datelike, Utc};
use chrono_tz::Tz;

const DEFAULT_RELEASE_TIMEZONE: &str = "Europe/Kyiv";

pub fn calculate(repo_root: &Path) -> Result<String> {
    let timezone_name =
        env::var("RELEASE_TIMEZONE").unwrap_or_else(|_| DEFAULT_RELEASE_TIMEZONE.to_owned());
    let timezone: Tz = timezone_name
        .parse()
        .with_context(|| format!("invalid release timezone {timezone_name:?}"))?;
    let today = Utc::now().with_timezone(&timezone);
    let year = today.year();
    let month = today.month();
    let day_patch = today.day() * 100;
    let year_string = year.to_string();
    let month_string = month.to_string();

    let tag_pattern = format!("{year}.{month}.*");
    let output = Command::new("git")
        .args(["tag", "--list", &tag_pattern])
        .current_dir(repo_root)
        .output()
        .context("failed to list Git tags")?;

    if !output.status.success() {
        bail!("git tag failed with {}", output.status);
    }

    let tags = String::from_utf8(output.stdout).context("git returned non-UTF-8 tag names")?;
    let mut next_daily_release = 0;

    for tag in tags.lines() {
        let mut parts = tag.split('.');
        let Some(tag_year) = parts.next() else {
            continue;
        };
        let Some(tag_month) = parts.next() else {
            continue;
        };
        let Some(tag_patch) = parts.next() else {
            continue;
        };

        if parts.next().is_some()
            || tag_year != year_string.as_str()
            || tag_month != month_string.as_str()
        {
            continue;
        }

        let Ok(patch) = tag_patch.parse::<u32>() else {
            continue;
        };

        if !(day_patch..=day_patch + 99).contains(&patch) {
            continue;
        }

        next_daily_release = next_daily_release.max(patch - day_patch + 1);
    }

    if next_daily_release > 99 {
        bail!(
            "no release numbers remain for {:04}-{:02}-{:02} in {timezone_name}",
            year,
            month,
            today.day()
        );
    }

    Ok(format!("{year}.{month}.{}", day_patch + next_daily_release))
}
