//! Calendar days in Europe/Brussels, for the per-day statistics.
//!
//! The dashboard shows Brussels time, so "a day" on the uptime strip is a
//! Brussels day, not a UTC one: a message acked at 00:30 local time belongs
//! to the new day. The rules are the EU's (CET, UTC+1, with CEST, UTC+2,
//! from 01:00 UTC on the last Sunday of March to 01:00 UTC on the last
//! Sunday of October), written out here because the day boundary is the
//! only piece of time zone logic kyu needs and a whole tz database would be
//! a heavy dependency for it (T6).
//!
//! Days are counted since 1970-01-01, which makes "the last 90 days" a
//! subtraction.

use super::clock::Millis;

const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 24 * HOUR_MS;

/// Days since 1970-01-01 of a proleptic Gregorian date (Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The inverse: (year, month, day) of a day count.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The last Sunday of `month` (31 days long: March or October) as a day
/// count.
fn last_sunday(year: i64, month: i64) -> i64 {
    let last = days_from_civil(year, month, 31);
    // 1970-01-01 was a Thursday, so (days + 4) % 7 is 0 on a Sunday.
    last - (last + 4).rem_euclid(7)
}

/// Brussels' offset from UTC at `at`, in milliseconds.
fn offset_ms(at: Millis) -> i64 {
    let (year, _, _) = civil_from_days(at.div_euclid(DAY_MS));
    let summer_from = last_sunday(year, 3) * DAY_MS + HOUR_MS;
    let summer_until = last_sunday(year, 10) * DAY_MS + HOUR_MS;
    if (summer_from..summer_until).contains(&at) {
        2 * HOUR_MS
    } else {
        HOUR_MS
    }
}

/// The Brussels calendar day `at` falls on, as days since 1970-01-01.
pub fn day_of(at: Millis) -> i64 {
    (at + offset_ms(at)).div_euclid(DAY_MS)
}

/// `YYYY-MM-DD` for a day count from [`day_of`].
pub fn day_label(day: i64) -> String {
    let (year, month, date) = civil_from_days(day);
    format!("{year:04}-{month:02}-{date:02}")
}

/// The start of the UTC hour `at` falls in.
pub fn hour_of(at: Millis) -> i64 {
    at.div_euclid(HOUR_MS) * HOUR_MS
}

pub const HOUR: i64 = HOUR_MS;
