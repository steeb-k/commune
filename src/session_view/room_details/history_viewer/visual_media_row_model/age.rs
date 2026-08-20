use gettextrs::gettext;
use gtk::glib;

use crate::ngettext_f;

/// The age of a media, as one of the ranges that the visual media history
/// viewer separates its content into.
///
/// The variants are declared from the most recent to the oldest and the
/// derived ordering follows, so an age that is greater than another one is
/// further in the past. The row model relies on that to keep its sections in
/// order when the timestamps of the events are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MediaAge {
    /// During the current week.
    ThisWeek,
    /// During the current month, before the current week.
    EarlierThisMonth,
    /// During the month before the current one.
    LastMonth,
    /// A number of months ago, between 2 and 11.
    MonthsAgo(u32),
    /// A number of years ago, 1 or more.
    YearsAgo(u32),
}

impl MediaAge {
    /// The age of a media sent at the given time, relative to the given
    /// current time.
    pub(crate) fn new(timestamp: &glib::DateTime, now: &glib::DateTime) -> Self {
        if timestamp.to_unix() >= now.to_unix() {
            // The media was sent in the future, which only happens when a clock is
            // wrong. Present it with the most recent ones.
            return Self::ThisWeek;
        }

        // These are ISO 8601 weeks, so they start on a Monday, and the week numbering
        // year is not the calendar year for the days around New Year's Day.
        if timestamp.week_numbering_year() == now.week_numbering_year()
            && timestamp.week_of_year() == now.week_of_year()
        {
            return Self::ThisWeek;
        }

        // The number of whole calendar months between the two dates. The timestamp is
        // in the past, so this cannot be negative.
        let months =
            u32::try_from((now.year() - timestamp.year()) * 12 + (now.month() - timestamp.month()))
                .unwrap_or_default();

        match months {
            0 => Self::EarlierThisMonth,
            1 => Self::LastMonth,
            2..=11 => Self::MonthsAgo(months),
            _ => Self::YearsAgo(months / 12),
        }
    }

    /// The heading presented above the media of this age.
    pub(crate) fn label(self) -> String {
        match self {
            Self::ThisWeek => gettext("This Week"),
            Self::EarlierThisMonth => gettext("Earlier This Month"),
            Self::LastMonth => gettext("Last Month"),
            Self::MonthsAgo(months) => ngettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this
                // is a variable name. This is the heading above the media that was sent
                // that many months ago.
                "{n} Month Ago",
                "{n} Months Ago",
                months,
                &[("n", &months.to_string())],
            ),
            Self::YearsAgo(years) => ngettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this
                // is a variable name. This is the heading above the media that was sent
                // that many years ago.
                "{n} Year Ago",
                "{n} Years Ago",
                years,
                &[("n", &years.to_string())],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local date, at noon.
    fn date(year: i32, month: i32, day: i32) -> glib::DateTime {
        glib::DateTime::from_local(year, month, day, 12, 0, 0.0).expect("date should be valid")
    }

    #[test]
    fn media_age() {
        // A Wednesday, so the current ISO week is the 17th to the 23rd.
        let now = date(2026, 8, 19);

        let age = |year, month, day| MediaAge::new(&date(year, month, day), &now);

        // The current week.
        assert_eq!(age(2026, 8, 19), MediaAge::ThisWeek);
        assert_eq!(age(2026, 8, 17), MediaAge::ThisWeek);

        // The rest of the current month.
        assert_eq!(age(2026, 8, 16), MediaAge::EarlierThisMonth);
        assert_eq!(age(2026, 8, 1), MediaAge::EarlierThisMonth);

        // The previous month.
        assert_eq!(age(2026, 7, 31), MediaAge::LastMonth);
        assert_eq!(age(2026, 7, 1), MediaAge::LastMonth);

        // The months before that.
        assert_eq!(age(2026, 6, 30), MediaAge::MonthsAgo(2));
        assert_eq!(age(2025, 9, 1), MediaAge::MonthsAgo(11));

        // A year is 12 whole months, not 365 days.
        assert_eq!(age(2025, 8, 31), MediaAge::YearsAgo(1));
        assert_eq!(age(2024, 9, 1), MediaAge::YearsAgo(1));
        assert_eq!(age(2024, 8, 1), MediaAge::YearsAgo(2));

        // A media from the future comes from a clock that is off.
        assert_eq!(age(2026, 8, 25), MediaAge::ThisWeek);
        assert_eq!(age(2027, 1, 1), MediaAge::ThisWeek);
    }

    #[test]
    fn media_age_is_ordered_from_the_most_recent() {
        let mut ages = [
            MediaAge::YearsAgo(2),
            MediaAge::MonthsAgo(2),
            MediaAge::ThisWeek,
            MediaAge::YearsAgo(1),
            MediaAge::LastMonth,
            MediaAge::MonthsAgo(11),
            MediaAge::EarlierThisMonth,
        ];
        ages.sort_unstable();

        assert_eq!(
            ages,
            [
                MediaAge::ThisWeek,
                MediaAge::EarlierThisMonth,
                MediaAge::LastMonth,
                MediaAge::MonthsAgo(2),
                MediaAge::MonthsAgo(11),
                MediaAge::YearsAgo(1),
                MediaAge::YearsAgo(2),
            ]
        );
    }

    #[test]
    fn media_age_around_new_years_day() {
        // The 1st of January 2027 is a Friday, so it is in the last ISO week of 2026.
        let now = date(2027, 1, 1);

        // The days of the same ISO week, in the previous calendar year.
        assert_eq!(MediaAge::new(&date(2026, 12, 28), &now), MediaAge::ThisWeek);
        // The rest of December is the previous month.
        assert_eq!(
            MediaAge::new(&date(2026, 12, 27), &now),
            MediaAge::LastMonth
        );
        assert_eq!(
            MediaAge::new(&date(2026, 11, 30), &now),
            MediaAge::MonthsAgo(2)
        );
    }
}
