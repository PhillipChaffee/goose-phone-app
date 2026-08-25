//! Cron as choices, not as a string.
//!
//! A schedule reaches this app as `0 30 9 * * 1-5`, which is a backend value
//! and therefore not something design rule 8 lets near a screen. It also has
//! to go back out in that form, and a phone is the worst possible place to
//! type one. So the app models the small set of schedules a person actually
//! sets from a phone — hourly, daily, weekdays, one weekday, one day of the
//! month — and the sheet builds the string.
//!
//! Two consequences, both deliberate:
//!
//!   - **Every value the sheet can produce is legal**, so there is no
//!     validation state and no error copy anywhere in the schedule flow. The
//!     picker cannot express `0 0 31 2 *`.
//!   - **A cron this grammar cannot hold is not rewritten.** [`parse`]
//!     answers `None`, the schedule row becomes a fact, and whoever set it
//!     from the CLI still has it. Silently normalising someone's
//!     `*/15 9-17 * * 1-5` into "every hour" would be the app losing data it
//!     did not understand.
//!
//! goose accepts both the 5-field and the 6-field form — `create_cron_task`
//! prepends a `0` seconds field to a 5-field job and logs it as legacy — and
//! its list returns whatever was stored, so [`parse`] reads both and [`build`]
//! writes the 6-field form goose prefers. Seconds are pinned to `0`: a
//! schedule that fires at a second nobody chose is not a schedule anybody
//! wanted.
//!
//! Day-of-week numbering is croner's (`tokio-cron-scheduler` 0.15 is built on
//! croner 3): `0`–`6` from Sunday, with `7` also Sunday. That is the ordinary
//! Vixie convention, so `1-5` means Monday to Friday.

/// How often a schedule fires. The five shapes the sheet offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Repeat {
    Hourly,
    Daily,
    Weekdays,
    Weekly,
    Monthly,
}

impl Repeat {
    /// Sheet order, coarsest interval last. Iterated rather than derived, so
    /// the order the user sees is stated in one place.
    pub(crate) const ALL: [Self; 5] = [
        Self::Hourly,
        Self::Daily,
        Self::Weekdays,
        Self::Weekly,
        Self::Monthly,
    ];

    /// Identifies the choice to the sheet's change handler. Never rendered.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Hourly => "hourly",
            Self::Daily => "daily",
            Self::Weekdays => "weekdays",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Hourly => "Every hour",
            Self::Daily => "Every day",
            Self::Weekdays => "Every weekday",
            Self::Weekly => "Every week",
            Self::Monthly => "Every month",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }
}

/// A schedule the sheet can express, and the only thing it can write.
///
/// `weekday`, `day` and `hour` are carried whether or not the current
/// [`Repeat`] reads them: switching from Weekly to Daily and back must not
/// silently reset the day you picked, and a struct that dropped them would.
/// [`build`] emits only the fields its `repeat` uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Schedule {
    pub repeat: Repeat,
    /// 0 = Sunday … 6 = Saturday. Read by [`Repeat::Weekly`].
    pub weekday: u8,
    /// Day of the month, 1–31. Read by [`Repeat::Monthly`].
    pub day: u8,
    /// 0–23. Read by everything except [`Repeat::Hourly`].
    pub hour: u8,
    /// A multiple of five, 0–55.
    pub minute: u8,
}

impl Default for Schedule {
    /// Weekday mornings at 9, which is what a schedule set from a phone
    /// almost always is — and what the sheet opens on for a recipe that has
    /// none.
    fn default() -> Self {
        Self {
            repeat: Repeat::Weekdays,
            weekday: 1,
            day: 1,
            hour: 9,
            minute: 0,
        }
    }
}

/// The minutes the sheet offers: five-minute granularity.
///
/// Nobody schedules :37 from a phone, and 60 rows is a scroll rather than a
/// choice.
pub(crate) const MINUTES: [u8; 12] = [0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55];

/// What a day-of-week field can say, once it is known to be one of the three
/// shapes this grammar holds.
enum Dow {
    Any,
    MonToFri,
    One(u8),
}

/// Read a cron expression, or answer `None` if this grammar cannot hold it.
///
/// `None` is not an error: it is the app declining to pretend it understands
/// a schedule set from somewhere with a full cron parser. The caller turns it
/// into a fact row rather than an editable control.
pub(crate) fn parse(cron: &str) -> Option<Schedule> {
    let fields: Vec<&str> = cron.split_whitespace().collect();
    let (second, minute, hour, day, month, weekday) = match *fields.as_slice() {
        // The legacy 5-field form goose still stores for jobs created by
        // older clients; its scheduler prepends the seconds itself.
        [minute, hour, day, month, weekday] => ("0", minute, hour, day, month, weekday),
        [second, minute, hour, day, month, weekday] => (second, minute, hour, day, month, weekday),
        _ => return None,
    };
    if second != "0" || month != "*" {
        return None;
    }

    let minute = number(minute, 0, 59)?;
    if minute % 5 != 0 {
        return None;
    }
    let hour = if hour == "*" {
        None
    } else {
        Some(number(hour, 0, 23)?)
    };
    let day = if day == "*" {
        None
    } else {
        Some(number(day, 1, 31)?)
    };
    let weekday = match weekday {
        "*" => Dow::Any,
        "1-5" => Dow::MonToFri,
        // 7 is Sunday's second spelling in croner; normalising it here means
        // the sheet shows "Sunday" rather than refusing the expression.
        other => Dow::One(number(other, 0, 7)? % 7),
    };

    let base = Schedule {
        minute,
        ..Schedule::default()
    };
    match (hour, day, weekday) {
        (None, None, Dow::Any) => Some(Schedule {
            repeat: Repeat::Hourly,
            ..base
        }),
        (Some(hour), None, Dow::Any) => Some(Schedule {
            repeat: Repeat::Daily,
            hour,
            ..base
        }),
        (Some(hour), None, Dow::MonToFri) => Some(Schedule {
            repeat: Repeat::Weekdays,
            hour,
            ..base
        }),
        (Some(hour), None, Dow::One(weekday)) => Some(Schedule {
            repeat: Repeat::Weekly,
            weekday,
            hour,
            ..base
        }),
        (Some(hour), Some(day), Dow::Any) => Some(Schedule {
            repeat: Repeat::Monthly,
            day,
            hour,
            ..base
        }),
        // A restricted day of month *and* day of week at once. croner ORs
        // them, which is a rule the sheet has no row for and most people do
        // not expect, so it stays with whoever wrote it.
        _ => None,
    }
}

/// The cron expression for a schedule, in the 6-field form goose prefers.
pub(crate) fn build(schedule: Schedule) -> String {
    let Schedule {
        repeat,
        weekday,
        day,
        hour,
        minute,
    } = schedule;
    match repeat {
        Repeat::Hourly => format!("0 {minute} * * * *"),
        Repeat::Daily => format!("0 {minute} {hour} * * *"),
        Repeat::Weekdays => format!("0 {minute} {hour} * * 1-5"),
        Repeat::Weekly => format!("0 {minute} {hour} * * {weekday}"),
        Repeat::Monthly => format!("0 {minute} {hour} {day} * *"),
    }
}

/// What a schedule does, as a sentence.
///
/// This is the whole reason [`Schedule`] exists: without it the row prints
/// `0 0 9 * * 1-5`, which is exactly the backend value design rule 8 keeps
/// off the screen.
pub(crate) fn describe(schedule: Schedule) -> String {
    let at = clock(schedule.hour, schedule.minute);
    match schedule.repeat {
        Repeat::Hourly if schedule.minute == 0 => "Runs every hour, on the hour".to_owned(),
        Repeat::Hourly => format!("Runs every hour at {}", minute_label(schedule.minute)),
        Repeat::Daily => format!("Runs every day at {at}"),
        Repeat::Weekdays => format!("Runs every weekday at {at}"),
        Repeat::Weekly => format!("Runs every {} at {at}", weekday_name(schedule.weekday)),
        Repeat::Monthly => format!(
            "Runs on the {} of every month at {at}",
            ordinal(schedule.day)
        ),
    }
}

/// What a stored cron string does, as a sentence — including the one this
/// grammar cannot hold, which still gets words rather than its own text.
pub(crate) fn summary(cron: &str) -> String {
    parse(cron).map_or_else(|| "Runs on a schedule".to_owned(), describe)
}

/// `9:00 AM`. Twelve-hour because the sentences around it are English.
pub(crate) fn clock(hour: u8, minute: u8) -> String {
    let (hour, meridiem) = twelve_hour(hour);
    format!("{hour}:{minute:02} {meridiem}")
}

/// `9 AM` — the hour row's value and the labels in its choice list.
pub(crate) fn hour_label(hour: u8) -> String {
    let (hour, meridiem) = twelve_hour(hour);
    format!("{hour} {meridiem}")
}

/// `:05`. The colon is what says this is a minute and not a count.
pub(crate) fn minute_label(minute: u8) -> String {
    format!(":{minute:02}")
}

pub(crate) fn weekday_name(weekday: u8) -> &'static str {
    const NAMES: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    NAMES.get(usize::from(weekday)).copied().unwrap_or("Sunday")
}

/// `1st`, `2nd`, `11th`, `21st`.
pub(crate) fn ordinal(day: u8) -> String {
    let suffix = match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{day}{suffix}")
}

const fn twelve_hour(hour: u8) -> (u8, &'static str) {
    match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour.saturating_sub(12), "PM"),
    }
}

/// A cron field that is a plain number in range, or `None` for anything with
/// a `*`, a `/`, a `,` or a name in it — all of which are legal cron and none
/// of which this grammar can hold.
fn number(field: &str, min: u8, max: u8) -> Option<u8> {
    let value: u8 = field.parse().ok()?;
    (min..=max).contains(&value).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the sheet rests on: anything it can build, it can read
    /// back as the same choices. Only the fields a given repeat actually uses
    /// are varied — `build` writes no others, so `parse` cannot invent them.
    #[test]
    fn every_schedule_the_sheet_can_build_parses_back_to_itself() {
        let base = Schedule::default();
        let cases = [
            Schedule {
                repeat: Repeat::Hourly,
                minute: 15,
                ..base
            },
            Schedule {
                repeat: Repeat::Daily,
                hour: 0,
                minute: 0,
                ..base
            },
            Schedule {
                repeat: Repeat::Daily,
                hour: 23,
                minute: 55,
                ..base
            },
            Schedule {
                repeat: Repeat::Weekdays,
                hour: 9,
                minute: 30,
                ..base
            },
            Schedule {
                repeat: Repeat::Weekly,
                weekday: 0,
                hour: 18,
                minute: 5,
                ..base
            },
            Schedule {
                repeat: Repeat::Weekly,
                weekday: 6,
                hour: 7,
                minute: 45,
                ..base
            },
            Schedule {
                repeat: Repeat::Monthly,
                day: 31,
                hour: 12,
                minute: 0,
                ..base
            },
        ];
        for case in cases {
            let cron = build(case);
            assert_eq!(parse(&cron), Some(case), "{cron} did not round-trip");
        }
    }

    /// goose stores whatever the client that created the job wrote, and its
    /// own CLI writes the 5-field form — so the sheet has to open on one.
    #[test]
    fn a_five_field_cron_reads_as_the_same_schedule_as_its_six_field_form() {
        let five = parse("30 8 * * 1-5");
        assert_eq!(
            five,
            Some(Schedule {
                repeat: Repeat::Weekdays,
                hour: 8,
                minute: 30,
                ..Schedule::default()
            })
        );
        assert_eq!(five, parse("0 30 8 * * 1-5"));
        // What goes back is the 6-field form; the schedule is unchanged, and
        // it is only written when the user picks something.
        assert_eq!(build(five.unwrap_or_default()), "0 30 8 * * 1-5");
    }

    /// The case the fact row exists for. Every one of these is a cron
    /// somebody could reasonably have written from a CLI, and rewriting any
    /// of them into the nearest thing this sheet can say would lose what they
    /// asked for.
    #[test]
    fn a_cron_this_grammar_cannot_hold_is_refused_rather_than_approximated() {
        for cron in [
            "*/15 * * * *",           // every quarter hour
            "0 9-17 * * 1-5",         // a range of hours
            "0 0 9 * * 1,3,5",        // a list of days
            "0 0 9 1 1 *",            // one month of the year
            "0 0 9 1 * 1",            // day of month AND day of week
            "0 37 9 * * *",           // a minute the picker cannot express
            "30 0 9 * * *",           // a seconds field that is not zero
            "0 0 9 * * MON",          // named days, which croner takes and this does not
            "@daily",                 // a macro
            "",                       // nothing at all
            "0 0 9 * *  extra * * *", // too many fields
        ] {
            assert_eq!(parse(cron), None, "{cron} should not have parsed");
        }
    }

    /// Rule 8, mechanically: whatever comes back from the server, what the
    /// screen gets is a sentence.
    #[test]
    fn a_schedule_is_described_in_words_and_never_as_its_cron() {
        let cases = [
            ("0 0 * * * *", "Runs every hour, on the hour"),
            ("0 5 * * * *", "Runs every hour at :05"),
            ("0 0 9 * * *", "Runs every day at 9:00 AM"),
            ("0 30 9 * * 1-5", "Runs every weekday at 9:30 AM"),
            ("0 0 18 * * 5", "Runs every Friday at 6:00 PM"),
            ("0 0 0 * * 0", "Runs every Sunday at 12:00 AM"),
            (
                "0 15 12 1 * *",
                "Runs on the 1st of every month at 12:15 PM",
            ),
            ("0 0 8 22 * *", "Runs on the 22nd of every month at 8:00 AM"),
        ];
        for (cron, expected) in cases {
            assert_eq!(summary(cron), expected);
            assert!(!summary(cron).contains('*'), "{cron} leaked into the copy");
        }
    }

    /// An un-representable cron still owes the list row a sentence: the row
    /// says the recipe is scheduled, and the detail screen is where the raw
    /// expression is shown as evidence.
    #[test]
    fn an_unreadable_cron_is_still_words_on_a_row() {
        assert_eq!(summary("*/15 * * * *"), "Runs on a schedule");
    }

    #[test]
    fn the_clock_reads_as_a_clock_at_both_ends_of_the_day() {
        assert_eq!(clock(0, 0), "12:00 AM");
        assert_eq!(clock(12, 5), "12:05 PM");
        assert_eq!(clock(23, 55), "11:55 PM");
        assert_eq!(hour_label(0), "12 AM");
        assert_eq!(hour_label(13), "1 PM");
        assert_eq!(minute_label(5), ":05");
    }

    #[test]
    fn ordinals_survive_the_teens() {
        let days: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 31]
            .into_iter()
            .map(ordinal)
            .collect();
        assert_eq!(
            days,
            ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "23rd", "31st"]
        );
    }
}
