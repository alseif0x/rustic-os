// SPDX-License-Identifier: Apache-2.0
pub(super) fn crc(bytes: &[u8]) -> u32 {
    let mut value = !0u32;
    crc_update(&mut value, bytes);
    !value
}
/// Running form: `crc(a ++ b) == !crc_update_final` when started from `!0` and
/// updated with `a` then `b`. It exists so a 32 KiB table needs no temporary.
pub(super) fn crc_update(value: &mut u32, bytes: &[u8]) {
    for byte in bytes {
        *value ^= u32::from(*byte);
        for _ in 0..8 {
            *value = (*value >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(*value & 1)));
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn standard_check_value() {
        assert_eq!(super::crc(b"123456789"), 0xcbf4_3926);
    }
}
