//! Formatage des dates et heures (heure locale sous Windows), sans dépendance.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Convertit un nombre de secondes depuis 1970 (UTC) en `(année, mois, jour, heure, minute)`.
pub fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let in_day = secs.rem_euclid(86_400);
    // Algorithme « civil from days » (Howard Hinnant), valable pour tout le calendrier grégorien.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (
        year,
        month,
        day,
        (in_day / 3_600) as u32,
        ((in_day % 3_600) / 60) as u32,
    )
}

/// Format d'affichage : `jj/mm/aaaa hh:mm`.
pub fn format_fields(year: i64, month: u32, day: u32, hour: u32, minute: u32) -> String {
    format!("{day:02}/{month:02}/{year:04} {hour:02}:{minute:02}")
}

/// Date et heure locales d'un instant, ou « — » s'il est antérieur à 1970.
pub fn format_local(time: SystemTime) -> String {
    let Ok(since_epoch) = time.duration_since(UNIX_EPOCH) else {
        return "—".to_string();
    };
    imp::format_local(since_epoch)
}

/// Durée affichée : `m:ss` ou `h:mm:ss`.
pub fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3_600, (total % 3_600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(windows)]
mod imp {
    use super::{civil_from_unix, format_fields};
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

    /// Secondes entre le 1er janvier 1601 (origine des `FILETIME`) et le 1er janvier 1970.
    const UNIX_TO_FILETIME_SECS: u64 = 11_644_473_600;

    pub fn format_local(since_epoch: Duration) -> String {
        let ticks = (since_epoch.as_secs() + UNIX_TO_FILETIME_SECS) * 10_000_000
            + u64::from(since_epoch.subsec_nanos() / 100);
        let file_time = FILETIME {
            dwLowDateTime: (ticks & 0xFFFF_FFFF) as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut utc = empty_system_time();
        let mut local = empty_system_time();
        // SAFETY: les pointeurs désignent des structures valides et exclusives pour la durée de
        // l'appel ; un fuseau nul signifie « fuseau horaire courant ».
        let converted = unsafe {
            FileTimeToSystemTime(&file_time, &mut utc) != 0
                && SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) != 0
        };
        if converted {
            format_fields(
                i64::from(local.wYear),
                u32::from(local.wMonth),
                u32::from(local.wDay),
                u32::from(local.wHour),
                u32::from(local.wMinute),
            )
        } else {
            // Repli : heure UTC.
            let (y, mo, d, h, mi) = civil_from_unix(since_epoch.as_secs() as i64);
            format_fields(y, mo, d, h, mi)
        }
    }

    fn empty_system_time() -> SYSTEMTIME {
        SYSTEMTIME {
            wYear: 0,
            wMonth: 0,
            wDayOfWeek: 0,
            wDay: 0,
            wHour: 0,
            wMinute: 0,
            wSecond: 0,
            wMilliseconds: 0,
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{civil_from_unix, format_fields};
    use std::time::Duration;

    pub fn format_local(since_epoch: Duration) -> String {
        let (y, mo, d, h, mi) = civil_from_unix(since_epoch.as_secs() as i64);
        format_fields(y, mo, d, h, mi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_are_correct() {
        assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0));
        assert_eq!(civil_from_unix(1_700_000_000), (2023, 11, 14, 22, 13));
        // 29 février d'une année bissextile (2000-02-29 00:00:00 UTC).
        assert_eq!(civil_from_unix(951_782_400), (2000, 2, 29, 0, 0));
        // 1er mars 2100 (2100 n'est pas bissextile).
        assert_eq!(civil_from_unix(4_107_542_400), (2100, 3, 1, 0, 0));
        // Avant 1970.
        assert_eq!(civil_from_unix(-86_400), (1969, 12, 31, 0, 0));
    }

    #[test]
    fn fields_are_zero_padded() {
        assert_eq!(format_fields(2024, 3, 5, 7, 9), "05/03/2024 07:09");
    }

    #[test]
    fn local_format_has_the_expected_shape() {
        let text = format_local(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        // Le fuseau dépend de la machine : on vérifie seulement la forme jj/mm/aaaa hh:mm.
        let bytes = text.as_bytes();
        assert_eq!(text.len(), 16, "{text}");
        assert_eq!(
            (bytes[2], bytes[5], bytes[10], bytes[13]),
            (b'/', b'/', b' ', b':')
        );
    }

    #[test]
    fn dates_before_1970_show_a_dash() {
        assert_eq!(format_local(UNIX_EPOCH - Duration::from_secs(10)), "—");
    }

    #[test]
    fn durations_are_formatted() {
        assert_eq!(format_duration(Duration::from_secs(5)), "0:05");
        assert_eq!(format_duration(Duration::from_secs(65)), "1:05");
        assert_eq!(format_duration(Duration::from_secs(3_661)), "1:01:01");
    }
}
