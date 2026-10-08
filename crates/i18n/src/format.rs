//! Display-only formatting for the two shipped locales. JSON, identifiers,
//! ports, protocol timestamps and copied values must use their original data.
//! Both languages intentionally share decimal points, comma grouping and SI
//! symbols. Dates use the Gregorian calendar and ISO order; callers supply
//! their already-converted civil date and explicit time-zone label.
use crate::{Localizer, Message, ResolvedLocale};
use std::time::Duration;

pub const MAX_DECIMAL_PRECISION: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GregorianDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct LocaleFormat {
    locale: ResolvedLocale,
}
impl LocaleFormat {
    pub const fn new(locale: ResolvedLocale) -> Self {
        Self { locale }
    }
    pub fn integer(self, value: u64) -> String {
        group_digits(&value.to_string())
    }
    pub fn signed_integer(self, value: i64) -> String {
        if value < 0 {
            format!("-{}", self.integer(value.unsigned_abs()))
        } else {
            self.integer(value as u64)
        }
    }
    pub fn decimal(self, value: f64, precision: usize) -> String {
        if !value.is_finite() {
            return self.unknown();
        }
        let precision = precision.min(MAX_DECIMAL_PRECISION);
        let mut rendered = format!("{value:.precision$}");
        // Rounded readings do not show an accidental negative zero.
        if rendered.starts_with('-') && rendered[1..].chars().all(|c| c == '0' || c == '.') {
            rendered.remove(0);
        }
        let (integer, fraction) = rendered
            .split_once('.')
            .map_or((rendered.as_str(), None), |(integer, fraction)| {
                (integer, Some(fraction))
            });
        let (sign, digits) = integer
            .strip_prefix('-')
            .map_or(("", integer), |digits| ("-", digits));
        let integer = group_digits(digits);
        fraction.map_or_else(
            || format!("{sign}{integer}"),
            |fraction| format!("{sign}{integer}.{fraction}"),
        )
    }
    /// `ratio` is a proportion, e.g. 0.25 displays as 25%; this never parses or
    /// changes editable numeric inputs, audio gains or protocol values.
    pub fn percent(self, ratio: f64, precision: usize) -> String {
        let value = ratio * 100.0;
        if !value.is_finite() {
            return self.unknown();
        }
        format!("{}%", self.decimal(value, precision))
    }
    pub fn relative_time(self, seconds: u64) -> String {
        Localizer::new(self.locale).render(&Self::relative_time_message(seconds))
    }
    /// Rendering paths with an existing Localizer keep its shared cache and
    /// diagnostic owner by rendering this semantic message themselves.
    pub fn relative_time_message(seconds: u64) -> Message {
        match seconds {
            0..5 => Message::EventsJustNow,
            5..60 => Message::EventsSecondsAgo { count: seconds },
            60..3600 => Message::EventsMinutesAgo {
                count: seconds / 60,
            },
            _ => Message::EventsHoursAgo {
                count: seconds / 3600,
            },
        }
    }
    /// The caller measures elapsed time with `Instant`; locale changes never
    /// replace that clock or rebase an event's timestamp.
    pub fn relative_elapsed(self, elapsed: Duration) -> String {
        self.relative_time(elapsed.as_secs())
    }
    pub fn date(self, date: GregorianDate, timezone: &str) -> String {
        if !valid_date(date) || timezone.trim().is_empty() || timezone.chars().any(char::is_control)
        {
            return self.unknown();
        }
        format!(
            "{:04}-{:02}-{:02} {timezone}",
            date.year, date.month, date.day
        )
    }
    fn unknown(self) -> String {
        Localizer::new(self.locale).render(&Message::CommonUnknown)
    }
}
fn group_digits(digits: &str) -> String {
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index != 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(digit);
    }
    output
}
fn valid_date(date: GregorianDate) -> bool {
    if date.year == 0 || date.year > 9999 {
        return false;
    }
    let leap = date.year.is_multiple_of(4)
        && (!date.year.is_multiple_of(100) || date.year.is_multiple_of(400));
    let days = match date.month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return false,
    };
    date.day != 0 && date.day <= days
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_numbers_are_bounded_and_do_not_mutate_data() {
        for locale in [ResolvedLocale::ZhCn, ResolvedLocale::En] {
            let format = LocaleFormat::new(locale);
            let input = 1234.5;
            assert_eq!(format.integer(u64::MAX), "18,446,744,073,709,551,615");
            assert_eq!(
                format.signed_integer(i64::MIN),
                "-9,223,372,036,854,775,808"
            );
            assert_eq!(format.decimal(input, 2), "1,234.50");
            assert_eq!(input, 1234.5);
            assert_eq!(format.decimal(-0.0001, 2), "0.00");
            assert_eq!(format.decimal(1.0, usize::MAX), "1.000000");
            assert_eq!(format.percent(0.125, 1), "12.5%");
            for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                assert_eq!(format.decimal(value, 2), format.unknown());
                assert_eq!(format.percent(value, 1), format.unknown());
            }
            assert_eq!(format.percent(f64::MAX, 1), format.unknown());
            for unit in ["dB", "dBFS", "Hz", "kHz", "ms", "ppm"] {
                assert_eq!(
                    format!("{} {unit}", format.decimal(1.0, 1)),
                    format!("1.0 {unit}")
                );
            }
        }
    }
    #[test]
    fn relative_time_uses_semantic_messages_and_real_plural_rules() {
        let english = LocaleFormat::new(ResolvedLocale::En);
        let plain = |value: String| value.replace(['\u{2068}', '\u{2069}'], "");
        assert_eq!(plain(english.relative_time(4)), "Just now");
        assert_eq!(plain(english.relative_time(5)), "5 seconds ago");
        assert_eq!(plain(english.relative_time(60)), "1 minute ago");
        assert_eq!(plain(english.relative_time(120)), "2 minutes ago");
        assert_eq!(
            plain(english.relative_elapsed(Duration::from_secs(3600))),
            "1 hour ago"
        );
        assert!(
            LocaleFormat::new(ResolvedLocale::ZhCn)
                .relative_time(60)
                .contains("分钟前")
        );
    }
    #[test]
    fn gregorian_dates_require_caller_timezone_and_handle_leap_years() {
        let format = LocaleFormat::new(ResolvedLocale::En);
        let leap = GregorianDate {
            year: 2024,
            month: 2,
            day: 29,
        };
        assert_eq!(format.date(leap, "UTC+08:00"), "2024-02-29 UTC+08:00");
        assert_eq!(
            format.date(GregorianDate { year: 2023, ..leap }, "UTC"),
            "Unknown"
        );
        assert_eq!(
            format.date(GregorianDate { year: 1900, ..leap }, "UTC"),
            "Unknown"
        );
        assert_eq!(
            format.date(GregorianDate { year: 2000, ..leap }, "UTC"),
            "2000-02-29 UTC"
        );
        assert_eq!(format.date(leap, ""), "Unknown");
        assert_eq!(format.date(leap, "UTC\n"), "Unknown");
    }
}
