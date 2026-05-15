/// Collection of well-tested regex patterns used across the codebase.
/// Each pattern is compiled once and exposed as a lazy_static.

use once_cell::sync::Lazy;
use regex::Regex;

pub static EMAIL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}$").unwrap()
});

pub static SEMVER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\.(?P<patch>0|[1-9]\d*)(?:-(?P<pre>[0-9A-Za-z\-]+(?:\.[0-9A-Za-z\-]+)*))?(?:\+(?P<build>[0-9A-Za-z\-]+(?:\.[0-9A-Za-z\-]+)*))?$").unwrap()
});

pub static ISO_DATE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\d{4}-(0[1-9]|1[0-2])-(0[1-9]|[12]\d|3[01])$").unwrap()
});

pub static IPV4: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?:(?:25[0-5]|2[0-4]\d|[01]?\d\d?)\.){3}(?:25[0-5]|2[0-4]\d|[01]?\d\d?)$").unwrap()
});

pub static SLUG: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)*$").unwrap()
});

pub static JWT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+$").unwrap()
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn valid_email()   { assert!(EMAIL.is_match("user@example.com")); }
    #[test] fn invalid_email() { assert!(!EMAIL.is_match("not-an-email")); }
    #[test] fn semver()        { assert!(SEMVER.is_match("1.2.3-alpha.1+build.42")); }
    #[test] fn iso_date()      { assert!(ISO_DATE.is_match("2026-05-15")); }
    #[test] fn bad_date()      { assert!(!ISO_DATE.is_match("2026-13-01")); }
    #[test] fn ipv4()          { assert!(IPV4.is_match("192.168.1.255")); }
    #[test] fn slug()          { assert!(SLUG.is_match("hello-world-123")); }
}
