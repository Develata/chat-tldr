use chrono::{Datelike, Days, NaiveDate, TimeDelta};

use super::{clock::Period, number};

pub(super) struct Date {
    pub value: NaiveDate,
    pub confidence: f32,
    pub period: Option<Period>,
}

pub(super) fn parse<'a>(
    text: &'a str,
    anchor: NaiveDate,
    relation_suffix: Option<&str>,
) -> Result<(Option<Date>, &'a str), ()> {
    for (prefix, days, period) in [
        ("大后天", 3, None),
        ("后天", 2, None),
        ("明天", 1, None),
        ("今天", 0, None),
        ("明晚", 1, Some(Period::Evening)),
        ("明早", 1, Some(Period::Morning)),
        ("今晚", 0, Some(Period::Evening)),
        ("今早", 0, Some(Period::Morning)),
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            let value = anchor.checked_add_days(Days::new(days)).ok_or(())?;
            return Ok((
                Some(Date {
                    value,
                    confidence: 0.9,
                    period,
                }),
                rest,
            ));
        }
    }
    for (prefix, week) in [
        ("下个星期", Week::Next),
        ("下星期", Week::Next),
        ("下周", Week::Next),
        ("下礼拜", Week::Next),
        ("这星期", Week::Current),
        ("本星期", Week::Current),
        ("这周", Week::Current),
        ("本周", Week::Current),
        ("这礼拜", Week::Current),
        ("本礼拜", Week::Current),
        ("星期", Week::Rolling),
        ("礼拜", Week::Rolling),
        ("周", Week::Rolling),
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            let day = rest.chars().next().ok_or(())?;
            let target = match day {
                '一' => 0,
                '二' => 1,
                '三' => 2,
                '四' => 3,
                '五' => 4,
                '六' => 5,
                '日' | '天' => 6,
                _ => return Err(()),
            };
            let delta = target - i64::from(anchor.weekday().num_days_from_monday());
            let delta = match week {
                Week::Next => delta + 7,
                Week::Current => delta,
                Week::Rolling => delta.rem_euclid(7),
            };
            let value = anchor
                .checked_add_signed(TimeDelta::days(delta))
                .ok_or(())?;
            let confidence = if value < anchor { 0.5 } else { 0.9 };
            return Ok((
                Some(Date {
                    value,
                    confidence,
                    period: None,
                }),
                &rest[day.len_utf8()..],
            ));
        }
    }
    if let Some((first, rest)) = number::prefix(text) {
        if let Some(rest) = rest.strip_prefix('月') {
            let (day, rest) = number::prefix(rest).ok_or(())?;
            let rest = rest.strip_prefix(['日', '号']).ok_or(())?;
            let mut value = NaiveDate::from_ymd_opt(anchor.year(), first, day).ok_or(())?;
            if anchor.signed_duration_since(value).num_days() > 7 {
                value =
                    NaiveDate::from_ymd_opt(anchor.year().checked_add(1).ok_or(())?, first, day)
                        .ok_or(())?;
            }
            return Ok((
                Some(Date {
                    value,
                    confidence: 0.9,
                    period: None,
                }),
                rest,
            ));
        }
        if let Some(rest) = rest.strip_prefix('号') {
            let mut value =
                NaiveDate::from_ymd_opt(anchor.year(), anchor.month(), first).ok_or(())?;
            if anchor.signed_duration_since(value).num_days() > 7 {
                let (year, month) = if anchor.month() == 12 {
                    (anchor.year().checked_add(1).ok_or(())?, 1)
                } else {
                    (anchor.year(), anchor.month() + 1)
                };
                value = NaiveDate::from_ymd_opt(year, month, first).ok_or(())?;
            }
            return Ok((
                Some(Date {
                    value,
                    confidence: 0.9,
                    period: None,
                }),
                rest,
            ));
        }
        if rest == "天"
            && matches!(
                relation_suffix,
                Some("后" | "之后" | "以后" | "内" | "以内")
            )
        {
            let value = anchor
                .checked_add_days(Days::new(u64::from(first)))
                .ok_or(())?;
            return Ok((
                Some(Date {
                    value,
                    confidence: 0.9,
                    period: None,
                }),
                "",
            ));
        }
    }
    Ok((None, text))
}

enum Week {
    Rolling,
    Current,
    Next,
}
