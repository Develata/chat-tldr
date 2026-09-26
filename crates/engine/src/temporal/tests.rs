use super::normalize;
use chat_tldr_core::{Granularity, NormalizedBy, TemporalRelation};
use chrono::{DateTime, FixedOffset, NaiveDate};

fn anchor() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-23T21:00:00+08:00").unwrap()
}

macro_rules! examples {
    ($($name:ident: ($raw:literal, $date:literal, $time:expr, $relation:ident, $granularity:ident, $confidence:literal)),+ $(,)?) => {$ (
        #[test]
        fn $name() {
            let result = normalize($raw, anchor());
            assert_eq!(result.raw, $raw);
            assert_eq!(result.anchor, anchor());
            assert_eq!(result.bound_date.unwrap().to_string(), $date);
            assert_eq!(result.bound_time.map(|time| time.format("%H:%M").to_string()).as_deref(), $time);
            assert_eq!(result.relation, TemporalRelation::$relation);
            assert_eq!(result.granularity, Granularity::$granularity);
            assert_eq!(result.normalized_by, NormalizedBy::Rule);
            assert!((result.confidence - $confidence).abs() < 0.0001);
        }
    )+};
}

examples! {
    documented_friday_deadline: ("周五前", "2026-09-25", None, Before, Day, 0.9),
    documented_next_wednesday: ("下周三前", "2026-09-30", None, Before, Day, 0.9),
    documented_tomorrow_evening_clock: ("明晚8点", "2026-09-24", Some("20:00"), At, Hour, 0.9),
    documented_tonight: ("今晚", "2026-09-23", None, At, HalfDay, 0.9),
    documented_day_after_tomorrow: ("后天下午3点半之前", "2026-09-25", Some("15:30"), Before, Minute, 0.9),
    documented_three_days_within: ("3天内", "2026-09-26", None, Before, Day, 0.9),
    documented_month_day: ("10月1号", "2026-10-01", None, At, Day, 0.9),
    documented_wednesday_is_today: ("周三", "2026-09-23", None, At, Day, 0.9),
    today: ("今天", "2026-09-23", None, At, Day, 0.9),
    this_morning: ("今早", "2026-09-23", None, At, HalfDay, 0.9),
    tomorrow: ("明天", "2026-09-24", None, At, Day, 0.9),
    tomorrow_morning: ("明早", "2026-09-24", None, At, HalfDay, 0.9),
    tomorrow_evening: ("明晚", "2026-09-24", None, At, HalfDay, 0.9),
    after_tomorrow_is_at_not_after: ("后天", "2026-09-25", None, At, Day, 0.9),
    three_days_ahead: ("大后天", "2026-09-26", None, At, Day, 0.9),
    rolling_monday: ("周一", "2026-09-28", None, At, Day, 0.9),
    rolling_tuesday: ("星期二", "2026-09-29", None, At, Day, 0.9),
    rolling_sunday: ("星期天", "2026-09-27", None, At, Day, 0.9),
    alternative_sunday: ("礼拜日", "2026-09-27", None, At, Day, 0.9),
    alternative_thursday: ("礼拜四", "2026-09-24", None, At, Day, 0.9),
    current_monday_past: ("这周一", "2026-09-21", None, At, Day, 0.5),
    current_tuesday_past: ("本周二", "2026-09-22", None, At, Day, 0.5),
    current_friday_future: ("本周五", "2026-09-25", None, At, Day, 0.9),
    next_week_full_prefix: ("下个星期三", "2026-09-30", None, At, Day, 0.9),
    next_friday_crosses_month: ("下周五", "2026-10-02", None, At, Day, 0.9),
    next_sunday: ("下周日", "2026-10-04", None, At, Day, 0.9),
    morning_clock: ("明天上午8点", "2026-09-24", Some("08:00"), At, Hour, 0.9),
    early_clock: ("今天早上6点", "2026-09-23", Some("06:00"), At, Hour, 0.9),
    noon_twelve: ("中午12点", "2026-09-23", Some("12:00"), At, Hour, 0.9),
    evening_twelve_stays_twelve: ("下午12点", "2026-09-23", Some("12:00"), At, Hour, 0.9),
    evening_clock: ("晚上8点", "2026-09-23", Some("20:00"), At, Hour, 0.9),
    implied_afternoon: ("3点", "2026-09-23", Some("15:00"), At, Hour, 0.6),
    implied_morning: ("8点", "2026-09-23", Some("08:00"), At, Hour, 0.6),
    implied_six_boundary: ("6点", "2026-09-23", Some("18:00"), At, Hour, 0.6),
    implied_seven_boundary: ("7点", "2026-09-23", Some("07:00"), At, Hour, 0.6),
    twenty_four_hour_notation: ("23点", "2026-09-23", Some("23:00"), At, Hour, 0.9),
    colon_is_explicit_clock: ("3:00", "2026-09-23", Some("03:00"), At, Minute, 0.9),
    colon_date_and_deadline: ("周五23:59之前", "2026-09-25", Some("23:59"), Before, Minute, 0.9),
    chinese_fullwidth_colon: ("明天18：05", "2026-09-24", Some("18:05"), At, Minute, 0.9),
    chinese_two: ("明晚两点", "2026-09-24", Some("14:00"), At, Hour, 0.9),
    chinese_thirty_minutes: ("上午九点三十分", "2026-09-23", Some("09:30"), At, Minute, 0.9),
    chinese_fifty_nine_minutes: ("二十三点五十九分", "2026-09-23", Some("23:59"), At, Minute, 0.9),
    chinese_month_day: ("十月三十一日", "2026-10-31", None, At, Day, 0.9),
    chinese_three_days: ("三天后", "2026-09-26", None, After, Day, 0.9),
    chinese_two_days: ("两天以内", "2026-09-25", None, Before, Day, 0.9),
    zero_days: ("0天后", "2026-09-23", None, After, Day, 0.9),
    day_within_past_seven_days: ("16号", "2026-09-16", None, At, Day, 0.9),
    day_rolls_after_seven_days: ("15号", "2026-10-15", None, At, Day, 0.9),
    month_day_exactly_seven_days: ("9月16日", "2026-09-16", None, At, Day, 0.9),
    month_day_rolls_year: ("9月15日", "2027-09-15", None, At, Day, 0.9),
    date_and_period_without_time: ("明天下午", "2026-09-24", None, At, HalfDay, 0.9),
    before_suffix_variant: ("周五以前", "2026-09-25", None, Before, Day, 0.9),
    after_suffix_variant: ("周五之后", "2026-09-25", None, After, Day, 0.9),
    after_suffix_other_variant: ("周五以后", "2026-09-25", None, After, Day, 0.9),
    deadline_prefix: ("截止周五", "2026-09-25", None, Before, Day, 0.9),
    deadline_prefix_alternative: ("截至明天", "2026-09-24", None, Before, Day, 0.9),
    english_deadline_prefix: ("DDL:周五", "2026-09-25", None, Before, Day, 0.9),
    unicode_whitespace: ("　明天 下午 3 点 半 之前\n", "2026-09-24", Some("15:30"), Before, Minute, 0.9),
    earlier_date_keeps_lower_confidence_with_clock: ("本周一下午3点", "2026-09-21", Some("15:00"), At, Hour, 0.5),
}

#[test]
fn unsupported_or_invalid_expressions_never_invent_bounds() {
    for raw in [
        "尽快",
        "",
        " ",
        "周八",
        "下周",
        "星期三和周五",
        "明天或后天",
        "周五前交报告",
        "2026年9月27日",
        "9月31日",
        "13月1日",
        "0月1日",
        "10月0日",
        "31号",
        "2月29日",
        "上午25点",
        "8点60分",
        "24:00",
        "-3点",
        "3天前",
        "3天",
        "今晚上午8点",
        "截止周五后",
        "下午",
        "中午3点",
        "明天3点20秒",
        "999999999999999999999天后",
        "一二点",
        "1三点",
        "明天😀",
        "下午8:30:00",
        "三十一月五日",
    ] {
        let result = normalize(raw, anchor());
        assert!(result.bound_date.is_none(), "{raw}");
        assert!(result.bound_time.is_none(), "{raw}");
        assert_eq!(result.normalized_by, NormalizedBy::None, "{raw}");
        assert_eq!(result.granularity, Granularity::Unknown, "{raw}");
        assert_eq!(result.raw, raw);
        assert_eq!(result.confidence, 0.0);
    }
}

#[test]
fn calendar_rollovers_use_checked_dates() {
    for (anchor, raw, expected) in [
        ("2026-12-30T21:00:00+08:00", "下周一", "2027-01-04"),
        ("2026-12-31T21:00:00+08:00", "明天", "2027-01-01"),
        ("2026-12-30T21:00:00+08:00", "1号", "2027-01-01"),
        ("2028-02-28T21:00:00+08:00", "明天", "2028-02-29"),
        ("2028-02-29T21:00:00+08:00", "明天", "2028-03-01"),
        ("2028-02-20T21:00:00+08:00", "2月29日", "2028-02-29"),
        ("2026-01-31T21:00:00+08:00", "3天内", "2026-02-03"),
    ] {
        let normalized = normalize(raw, DateTime::parse_from_rfc3339(anchor).unwrap());
        assert_eq!(
            normalized.bound_date.unwrap().to_string(),
            expected,
            "{raw}"
        );
    }
    let leap_roll = DateTime::parse_from_rfc3339("2028-12-01T00:00:00+00:00").unwrap();
    assert_eq!(
        normalize("2月29日", leap_roll).normalized_by,
        NormalizedBy::None
    );
}

#[test]
fn negative_offset_uses_message_local_date_not_utc_date() {
    let local = DateTime::parse_from_rfc3339("2026-09-23T23:59:00-03:00").unwrap();
    assert_eq!(
        normalize("今天", local).bound_date.unwrap().to_string(),
        "2026-09-23"
    );
    assert_eq!(
        normalize("周三", local).bound_date.unwrap().to_string(),
        "2026-09-23"
    );
    assert_eq!(
        normalize("明天", local).bound_date.unwrap().to_string(),
        "2026-09-24"
    );
}

#[test]
fn chrono_range_boundaries_do_not_panic() {
    let utc = FixedOffset::east_opt(0).unwrap();
    let max =
        DateTime::from_naive_utc_and_offset(NaiveDate::MAX.and_hms_opt(23, 59, 59).unwrap(), utc);
    assert_eq!(normalize("明天", max).normalized_by, NormalizedBy::None);
    assert_eq!(
        normalize("4294967295天后", max).normalized_by,
        NormalizedBy::None
    );
    let beyond_local = max.with_timezone(&FixedOffset::east_opt(3600).unwrap());
    assert_eq!(
        normalize("今天", beyond_local).normalized_by,
        NormalizedBy::None
    );
    let min = DateTime::from_naive_utc_and_offset(
        NaiveDate::MIN.and_hms_opt(0, 0, 0).unwrap(),
        FixedOffset::west_opt(3600).unwrap(),
    );
    assert_eq!(normalize("今天", min).normalized_by, NormalizedBy::None);
}

#[test]
fn varied_unicode_inputs_are_total() {
    let alphabet: Vec<char> = "今明天周一十二点半月号前后内下午:😀零九ABC\0\n　"
        .chars()
        .collect();
    let mut state = 12345_u64;
    for length in 0..80 {
        for _ in 0..8 {
            let mut raw = String::new();
            for _ in 0..length {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                raw.push(alphabet[(state as usize) % alphabet.len()]);
            }
            let result = normalize(&raw, anchor());
            assert_eq!(result.raw, raw);
            assert!((0.0..=1.0).contains(&result.confidence));
            if result.normalized_by == NormalizedBy::None {
                assert!(result.bound_date.is_none() && result.bound_time.is_none());
            }
        }
    }
}
