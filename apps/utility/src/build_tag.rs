// SPDX-License-Identifier: Apache-2.0
//! Compile-time build tag, so separately built utility images are observably distinct.
//!
//! A build sets `RUSTIC_UTILITY_TAG` to a decimal number; the default build sets
//! nothing and reports `0`. The tag is reported in word 1 of the `FINISH`
//! report and in nothing else. It identifies a build invocation for acceptance
//! evidence only: it is not a version, an authority or a publisher identity.

/// This build's tag; an invalid `RUSTIC_UTILITY_TAG` fails the build.
pub const TAG: u64 = parse(option_env!("RUSTIC_UTILITY_TAG"));

/// Decode a tag of one to nine decimal digits; no tag is `0`. Any other text
/// panics, which in the constant above is a compile-time error.
pub const fn parse(text: Option<&str>) -> u64 {
    let Some(text) = text else {
        return 0;
    };
    let bytes = text.as_bytes();
    assert!(
        !bytes.is_empty() && bytes.len() <= 9,
        "RUSTIC_UTILITY_TAG must have one to nine decimal digits"
    );
    let mut value = 0;
    let mut i = 0;
    while i < bytes.len() {
        assert!(
            bytes[i].is_ascii_digit(),
            "RUSTIC_UTILITY_TAG must be decimal"
        );
        value = value * 10 + (bytes[i] - b'0') as u64;
        i += 1;
    }
    value
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn absent_tag_is_zero_and_digits_decode() {
        assert_eq!(parse(None), 0);
        assert_eq!(parse(Some("0")), 0);
        assert_eq!(parse(Some("1")), 1);
        assert_eq!(parse(Some("002")), 2);
        assert_eq!(parse(Some("999999999")), 999_999_999);
    }

    #[test]
    #[should_panic(expected = "decimal")]
    fn non_decimal_tag_is_refused() {
        parse(Some("1a"));
    }

    #[test]
    #[should_panic(expected = "one to nine")]
    fn empty_tag_is_refused() {
        parse(Some(""));
    }

    #[test]
    #[should_panic(expected = "one to nine")]
    fn overlong_tag_is_refused() {
        parse(Some("1234567890"));
    }
}
