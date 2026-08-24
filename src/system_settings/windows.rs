//! Reading the clock format from Windows.
//!
//! The fallback everywhere else is to format a time with the locale and look
//! for an "am" or "pm" on the end of it. That is right about the *locale*, but
//! Windows lets somebody override the format independently of it — Settings →
//! Time & language → Language & region → Regional format → Change formats,
//! which writes `sShortTime` — and the override is invisible to the locale. So
//! a user on `en-US` who has asked for 24-hour clocks gets 12-hour ones
//! everywhere except Commune, which is exactly the sort of small wrongness that
//! makes an application feel foreign.
//!
//! This is read once, at startup. Windows will tell an application when the
//! setting changes (`WM_SETTINGCHANGE`, or `RegNotifyChangeKeyValue` on the
//! key), but there is no message loop of ours to receive it in and the
//! difference only shows after a restart — the same place macOS leaves it.

use tracing::debug;
use windows::{
    Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW},
    core::w,
};

use super::ClockFormat;

/// The clock format the user has asked Windows for, if it can be read.
///
/// Returns `None` when the value is missing or unreadable, which leaves the
/// caller with the locale-derived answer rather than a guess.
pub(super) fn clock_format() -> Option<ClockFormat> {
    let format = short_time_format()?;

    // The format is a picture string like `h:mm tt` or `HH:mm`. A capital `H`
    // is the 24-hour hour and a lower-case `h` the 12-hour one, and that is the
    // whole of the distinction — `tt`, the AM/PM designator, is not reliable,
    // because it is legal (if odd) to have one alongside a 24-hour hour.
    //
    // Literal text can be quoted with single quotes and could contain either
    // letter, so those runs are skipped rather than read.
    let mut in_quotes = false;
    for character in format.chars() {
        match character {
            '\'' => in_quotes = !in_quotes,
            'H' if !in_quotes => {
                debug!("Windows short time format is {format:?}, using a 24-hour clock");
                return Some(ClockFormat::TwentyFourHours);
            }
            'h' if !in_quotes => {
                debug!("Windows short time format is {format:?}, using a 12-hour clock");
                return Some(ClockFormat::TwelveHours);
            }
            _ => {}
        }
    }

    None
}

/// The `sShortTime` value, which is where Windows records the format.
fn short_time_format() -> Option<String> {
    let mut size: u32 = 0;

    // Asked with no buffer, `RegGetValueW` reports the size one is needed, in
    // bytes.
    // SAFETY: the two `w!` strings are null-terminated wide literals with a
    // static lifetime, and `size` is a valid place to write a `u32`.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\International"),
            w!("sShortTime"),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&raw mut size),
        )
    }
    .ok()
    .ok()?;

    // The size is in bytes and the value is UTF-16.
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];

    // SAFETY: `buffer` is `size` bytes long, which is what the call above said
    // was needed, and `size` is passed again so the call knows that.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\International"),
            w!("sShortTime"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&raw mut size),
        )
    }
    .ok()
    .ok()?;

    // The value is null-terminated and the terminator is inside the length.
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever this machine is set to, reading it must not fail and must not
    /// panic.
    ///
    /// There is nothing to assert about the answer — it is whatever the person
    /// running the test has asked Windows for — but the two registry calls and
    /// the buffer sizing between them are worth exercising, because getting the
    /// size wrong is the kind of mistake that reads past the end of a buffer
    /// rather than returning the wrong answer.
    #[test]
    fn the_clock_format_is_readable() {
        let format = short_time_format();
        assert!(
            format.is_some_and(|format| !format.is_empty()),
            "sShortTime should be readable and not empty"
        );

        // And it should be understood, since Windows always writes a picture
        // string with an hour in it.
        assert!(clock_format().is_some());
    }
}
