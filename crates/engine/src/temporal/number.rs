//! The small number vocabulary needed by dates, hours and minutes.

pub(super) fn prefix(text: &str) -> Option<(u32, &str)> {
    let end = text
        .char_indices()
        .take_while(|(_, ch)| {
            ch.is_ascii_digit()
                || matches!(
                    ch,
                    '零' | '〇'
                        | '一'
                        | '二'
                        | '两'
                        | '三'
                        | '四'
                        | '五'
                        | '六'
                        | '七'
                        | '八'
                        | '九'
                        | '十'
                )
        })
        .map(|(index, ch)| index + ch.len_utf8())
        .last()?;
    let value = whole(&text[..end])?;
    Some((value, &text[end..]))
}

pub(super) fn whole(text: &str) -> Option<u32> {
    if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) {
        return text.parse().ok();
    }
    let mut chars = text.chars();
    let first = chars.next()?;
    let second = chars.next();
    let third = chars.next();
    if chars.next().is_some() {
        return None;
    }
    match (first, second, third) {
        ('十', None, None) => Some(10),
        ('十', Some(ones), None) => positive_digit(ones).map(|ones| 10 + ones),
        (tens, Some('十'), ones) => {
            let tens = positive_digit(tens)?;
            let ones = match ones {
                Some(ones) => positive_digit(ones)?,
                None => 0,
            };
            Some(tens * 10 + ones)
        }
        (digit, None, None) => digit_value(digit),
        _ => None,
    }
}

fn positive_digit(ch: char) -> Option<u32> {
    digit_value(ch).filter(|value| *value > 0)
}

fn digit_value(ch: char) -> Option<u32> {
    Some(match ch {
        '零' | '〇' => 0,
        '一' => 1,
        '二' | '两' => 2,
        '三' => 3,
        '四' => 4,
        '五' => 5,
        '六' => 6,
        '七' => 7,
        '八' => 8,
        '九' => 9,
        _ => return None,
    })
}
