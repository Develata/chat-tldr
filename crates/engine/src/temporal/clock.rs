use chat_tldr_core::Granularity;
use chrono::NaiveTime;

use super::number;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Period {
    Morning,
    Evening,
    Noon,
}

pub(super) struct Clock {
    pub value: Option<NaiveTime>,
    pub granularity: Granularity,
    pub confidence: f32,
}

pub(super) fn parse(mut text: &str, hint: Option<Period>, has_date: bool) -> Option<Clock> {
    let mut period = hint;
    for (prefix, selected) in [
        ("上午", Period::Morning),
        ("早上", Period::Morning),
        ("下午", Period::Evening),
        ("晚上", Period::Evening),
        ("中午", Period::Noon),
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            if hint.is_some_and(|hint| hint != selected) {
                return None;
            }
            period = Some(selected);
            text = rest;
            break;
        }
    }
    if text.is_empty() {
        return (has_date && period.is_some()).then_some(Clock {
            value: None,
            granularity: Granularity::HalfDay,
            confidence: 0.9,
        });
    }
    let (mut hour, rest) = number::prefix(text)?;
    if hour > 23 {
        return None;
    }
    let (minute, granularity, point_notation) = if let Some(rest) = rest.strip_prefix('点') {
        let (minute, granularity) = if rest.is_empty() {
            (0, Granularity::Hour)
        } else if rest == "半" {
            (30, Granularity::Minute)
        } else {
            (
                number::whole(rest.strip_suffix('分')?)?,
                Granularity::Minute,
            )
        };
        (minute, granularity, true)
    } else {
        (
            number::whole(rest.strip_prefix([':', '：'])?)?,
            Granularity::Minute,
            false,
        )
    };
    if minute > 59 {
        return None;
    }
    let mut confidence = 0.9;
    match period {
        Some(Period::Evening) if hour < 12 => hour += 12,
        Some(Period::Noon) if hour != 12 => return None,
        None if point_notation && (1..=11).contains(&hour) => {
            if hour <= 6 {
                hour += 12;
            }
            confidence = 0.6;
        }
        _ => {}
    }
    Some(Clock {
        value: Some(NaiveTime::from_hms_opt(hour, minute, 0)?),
        granularity,
        confidence,
    })
}
