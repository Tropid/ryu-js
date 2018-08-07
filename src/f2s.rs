// Translated from C to Rust. The original C code can be found at
// https://github.com/ulfjack/ryu and carries the following license:
//
// Copyright 2018 Ulf Adams
//
// The contents of this file may be used under the terms of the Apache License,
// Version 2.0.
//
//    (See accompanying file LICENSE-Apache or copy at
//     http://www.apache.org/licenses/LICENSE-2.0)
//
// Alternatively, the contents of this file may be used under the terms of
// the Boost Software License, Version 1.0.
//    (See accompanying file LICENSE-Boost or copy at
//     https://www.boost.org/LICENSE_1_0.txt)
//
// Unless required by applicable law or agreed to in writing, this software
// is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.

use core::{mem, ptr};

use common::*;
use digit_table::*;

#[cfg(feature = "no-panic")]
use no_panic::no_panic;

pub const FLOAT_MANTISSA_BITS: u32 = 23;
pub const FLOAT_EXPONENT_BITS: u32 = 8;

const FLOAT_POW5_INV_BITCOUNT: i32 = 59;
const FLOAT_POW5_BITCOUNT: i32 = 61;

// This table is generated by PrintFloatLookupTable.
static FLOAT_POW5_INV_SPLIT: [u64; 32] = [
    576460752303423489,
    461168601842738791,
    368934881474191033,
    295147905179352826,
    472236648286964522,
    377789318629571618,
    302231454903657294,
    483570327845851670,
    386856262276681336,
    309485009821345069,
    495176015714152110,
    396140812571321688,
    316912650057057351,
    507060240091291761,
    405648192073033409,
    324518553658426727,
    519229685853482763,
    415383748682786211,
    332306998946228969,
    531691198313966350,
    425352958651173080,
    340282366920938464,
    544451787073501542,
    435561429658801234,
    348449143727040987,
    557518629963265579,
    446014903970612463,
    356811923176489971,
    570899077082383953,
    456719261665907162,
    365375409332725730,
    1 << 63,
];

static FLOAT_POW5_SPLIT: [u64; 47] = [
    1152921504606846976,
    1441151880758558720,
    1801439850948198400,
    2251799813685248000,
    1407374883553280000,
    1759218604441600000,
    2199023255552000000,
    1374389534720000000,
    1717986918400000000,
    2147483648000000000,
    1342177280000000000,
    1677721600000000000,
    2097152000000000000,
    1310720000000000000,
    1638400000000000000,
    2048000000000000000,
    1280000000000000000,
    1600000000000000000,
    2000000000000000000,
    1250000000000000000,
    1562500000000000000,
    1953125000000000000,
    1220703125000000000,
    1525878906250000000,
    1907348632812500000,
    1192092895507812500,
    1490116119384765625,
    1862645149230957031,
    1164153218269348144,
    1455191522836685180,
    1818989403545856475,
    2273736754432320594,
    1421085471520200371,
    1776356839400250464,
    2220446049250313080,
    1387778780781445675,
    1734723475976807094,
    2168404344971008868,
    1355252715606880542,
    1694065894508600678,
    2117582368135750847,
    1323488980084844279,
    1654361225106055349,
    2067951531382569187,
    1292469707114105741,
    1615587133892632177,
    2019483917365790221,
];

#[cfg_attr(feature = "no-panic", inline)]
fn pow5_factor(mut value: u32) -> u32 {
    let mut count = 0u32;
    loop {
        if value == 0 {
            return 0;
        }
        if value % 5 != 0 {
            return count;
        }
        value /= 5;
        count += 1;
    }
}

// Returns true if value is divisible by 5^p.
#[cfg_attr(feature = "no-panic", inline)]
fn multiple_of_power_of_5(value: u32, p: u32) -> bool {
    pow5_factor(value) >= p
}

// Returns true if value is divisible by 2^p.
#[cfg_attr(feature = "no-panic", inline)]
fn multiple_of_power_of_2(value: u32, p: u32) -> bool {
    // return __builtin_ctz(value) >= p;
    (value & ((1u32 << p) - 1)) == 0
}

// It seems to be slightly faster to avoid uint128_t here, although the
// generated code for uint128_t looks slightly nicer.
#[cfg_attr(feature = "no-panic", inline)]
fn mul_shift(m: u32, factor: u64, shift: i32) -> u32 {
    debug_assert!(shift > 32);

    // The casts here help MSVC to avoid calls to the __allmul library
    // function.
    let factor_lo = factor as u32;
    let factor_hi = (factor >> 32) as u32;
    let bits0 = m as u64 * factor_lo as u64;
    let bits1 = m as u64 * factor_hi as u64;

    let sum = (bits0 >> 32) + bits1;
    let shifted_sum = sum >> (shift - 32);
    debug_assert!(shifted_sum <= u32::max_value() as u64);
    shifted_sum as u32
}

#[cfg_attr(feature = "no-panic", inline)]
fn mul_pow5_inv_div_pow2(m: u32, q: u32, j: i32) -> u32 {
    debug_assert!(q < FLOAT_POW5_INV_SPLIT.len() as u32);
    unsafe { mul_shift(m, *FLOAT_POW5_INV_SPLIT.get_unchecked(q as usize), j) }
}

#[cfg_attr(feature = "no-panic", inline)]
fn mul_pow5_div_pow2(m: u32, i: u32, j: i32) -> u32 {
    debug_assert!(i < FLOAT_POW5_SPLIT.len() as u32);
    unsafe { mul_shift(m, *FLOAT_POW5_SPLIT.get_unchecked(i as usize), j) }
}

#[cfg_attr(feature = "no-panic", inline)]
pub fn decimal_length(v: u32) -> u32 {
    // Function precondition: v is not a 10-digit number.
    // (9 digits are sufficient for round-tripping.)
    debug_assert!(v < 1000000000);

    if v >= 100000000 {
        9
    } else if v >= 10000000 {
        8
    } else if v >= 1000000 {
        7
    } else if v >= 100000 {
        6
    } else if v >= 10000 {
        5
    } else if v >= 1000 {
        4
    } else if v >= 100 {
        3
    } else if v >= 10 {
        2
    } else {
        1
    }
}

// A floating decimal representing m * 10^e.
pub struct FloatingDecimal32 {
    pub mantissa: u32,
    pub exponent: i32,
}

#[cfg_attr(feature = "no-panic", inline)]
pub fn f2d(ieee_mantissa: u32, ieee_exponent: u32) -> FloatingDecimal32 {
    let bias = (1u32 << (FLOAT_EXPONENT_BITS - 1)) - 1;

    let (e2, m2) = if ieee_exponent == 0 {
        (
            // We subtract 2 so that the bounds computation has 2 additional bits.
            1 - bias as i32 - FLOAT_MANTISSA_BITS as i32 - 2,
            ieee_mantissa,
        )
    } else {
        (
            ieee_exponent as i32 - bias as i32 - FLOAT_MANTISSA_BITS as i32 - 2,
            (1u32 << FLOAT_MANTISSA_BITS) | ieee_mantissa,
        )
    };
    let even = (m2 & 1) == 0;
    let accept_bounds = even;

    // Step 2: Determine the interval of legal decimal representations.
    let mv = 4 * m2;
    let mp = 4 * m2 + 2;
    // Implicit bool -> int conversion. True is 1, false is 0.
    let mm_shift = (ieee_mantissa != 0 || ieee_exponent <= 1) as u32;
    let mm = 4 * m2 - 1 - mm_shift;

    // Step 3: Convert to a decimal power base using 64-bit arithmetic.
    let mut vr: u32;
    let mut vp: u32;
    let mut vm: u32;
    let e10: i32;
    let mut vm_is_trailing_zeros = false;
    let mut vr_is_trailing_zeros = false;
    let mut last_removed_digit = 0u8;
    if e2 >= 0 {
        let q = log10_pow2(e2) as u32;
        e10 = q as i32;
        let k = FLOAT_POW5_INV_BITCOUNT + pow5bits(q as i32) as i32 - 1;
        let i = -e2 + q as i32 + k;
        vr = mul_pow5_inv_div_pow2(mv, q, i);
        vp = mul_pow5_inv_div_pow2(mp, q, i);
        vm = mul_pow5_inv_div_pow2(mm, q, i);
        if q != 0 && (vp - 1) / 10 <= vm / 10 {
            // We need to know one removed digit even if we are not going to loop below. We could use
            // q = X - 1 above, except that would require 33 bits for the result, and we've found that
            // 32-bit arithmetic is faster even on 64-bit machines.
            let l = FLOAT_POW5_INV_BITCOUNT + pow5bits(q as i32 - 1) as i32 - 1;
            last_removed_digit =
                (mul_pow5_inv_div_pow2(mv, q - 1, -e2 + q as i32 - 1 + l) % 10) as u8;
        }
        if q <= 9 {
            // The largest power of 5 that fits in 24 bits is 5^10, but q<=9 seems to be safe as well.
            // Only one of mp, mv, and mm can be a multiple of 5, if any.
            if mv % 5 == 0 {
                vr_is_trailing_zeros = multiple_of_power_of_5(mv, q);
            } else if accept_bounds {
                vm_is_trailing_zeros = multiple_of_power_of_5(mm, q);
            } else {
                vp -= multiple_of_power_of_5(mp, q) as u32;
            }
        }
    } else {
        let q = log10_pow5(-e2) as u32;
        e10 = q as i32 + e2;
        let i = -e2 - q as i32;
        let k = pow5bits(i) as i32 - FLOAT_POW5_BITCOUNT;
        let mut j = q as i32 - k;
        vr = mul_pow5_div_pow2(mv, i as u32, j);
        vp = mul_pow5_div_pow2(mp, i as u32, j);
        vm = mul_pow5_div_pow2(mm, i as u32, j);
        if q != 0 && (vp - 1) / 10 <= vm / 10 {
            j = q as i32 - 1 - (pow5bits(i + 1) as i32 - FLOAT_POW5_BITCOUNT);
            last_removed_digit = (mul_pow5_div_pow2(mv, (i + 1) as u32, j) % 10) as u8;
        }
        if q <= 1 {
            // {vr,vp,vm} is trailing zeros if {mv,mp,mm} has at least q trailing 0 bits.
            // mv = 4 * m2, so it always has at least two trailing 0 bits.
            vr_is_trailing_zeros = true;
            if accept_bounds {
                // mm = mv - 1 - mm_shift, so it has 1 trailing 0 bit iff mm_shift == 1.
                vm_is_trailing_zeros = mm_shift == 1;
            } else {
                // mp = mv + 2, so it always has at least one trailing 0 bit.
                vp -= 1;
            }
        } else if q < 31 {
            // TODO(ulfjack): Use a tighter bound here.
            vr_is_trailing_zeros = multiple_of_power_of_2(mv, q - 1);
        }
    }

    // Step 4: Find the shortest decimal representation in the interval of legal representations.
    let mut removed = 0u32;
    let output = if vm_is_trailing_zeros || vr_is_trailing_zeros {
        // General case, which happens rarely.
        while vp / 10 > vm / 10 {
            vm_is_trailing_zeros &= vm - (vm / 10) * 10 == 0;
            vr_is_trailing_zeros &= last_removed_digit == 0;
            last_removed_digit = (vr % 10) as u8;
            vr /= 10;
            vp /= 10;
            vm /= 10;
            removed += 1;
        }
        if vm_is_trailing_zeros {
            while vm % 10 == 0 {
                vr_is_trailing_zeros &= last_removed_digit == 0;
                last_removed_digit = (vr % 10) as u8;
                vr /= 10;
                vp /= 10;
                vm /= 10;
                removed += 1;
            }
        }
        if vr_is_trailing_zeros && last_removed_digit == 5 && vr % 2 == 0 {
            // Round even if the exact number is .....50..0.
            last_removed_digit = 4;
        }
        // We need to take vr+1 if vr is outside bounds or we need to round up.
        vr + ((vr == vm && (!accept_bounds || !vm_is_trailing_zeros)) || (last_removed_digit >= 5))
            as u32
    } else {
        // Common case.
        while vp / 10 > vm / 10 {
            last_removed_digit = (vr % 10) as u8;
            vr /= 10;
            vp /= 10;
            vm /= 10;
            removed += 1;
        }
        // We need to take vr+1 if vr is outside bounds or we need to round up.
        vr + ((vr == vm) || (last_removed_digit >= 5)) as u32
    };
    let exp = e10 + removed as i32;

    FloatingDecimal32 {
        exponent: exp,
        mantissa: output,
    }
}

#[cfg_attr(feature = "no-panic", inline)]
unsafe fn to_chars(v: FloatingDecimal32, sign: bool, result: *mut u8) -> usize {
    // Step 5: Print the decimal representation.
    let mut index = 0isize;
    if sign {
        *result.offset(index) = b'-';
        index += 1;
    }

    let mut output = v.mantissa;
    let olength = decimal_length(output);

    // Print the decimal digits.
    // The following code is equivalent to:
    // for (uint32_t i = 0; i < olength - 1; ++i) {
    //   const uint32_t c = output % 10; output /= 10;
    //   result[index + olength - i] = (char) ('0' + c);
    // }
    // result[index] = '0' + output % 10;
    let mut i = 0isize;
    while output >= 10000 {
        let c = output - 10000 * (output / 10000);
        output /= 10000;
        let c0 = (c % 100) << 1;
        let c1 = (c / 100) << 1;
        ptr::copy_nonoverlapping(
            DIGIT_TABLE.get_unchecked(c0 as usize),
            result.offset(index + olength as isize - i - 1),
            2,
        );
        ptr::copy_nonoverlapping(
            DIGIT_TABLE.get_unchecked(c1 as usize),
            result.offset(index + olength as isize - i - 3),
            2,
        );
        i += 4;
    }
    if output >= 100 {
        let c = (output % 100) << 1;
        output /= 100;
        ptr::copy_nonoverlapping(
            DIGIT_TABLE.get_unchecked(c as usize),
            result.offset(index + olength as isize - i - 1),
            2,
        );
        i += 2;
    }
    if output >= 10 {
        let c = output << 1;
        // We can't use memcpy here: the decimal dot goes between these two digits.
        *result.offset(index + olength as isize - i) = *DIGIT_TABLE.get_unchecked(c as usize + 1);
        *result.offset(index) = *DIGIT_TABLE.get_unchecked(c as usize);
    } else {
        *result.offset(index) = b'0' + output as u8;
    }

    // Print decimal point if needed.
    if olength > 1 {
        *result.offset(index + 1) = b'.';
        index += olength as isize + 1;
    } else {
        index += 1;
    }

    // Print the exponent.
    *result.offset(index) = b'E';
    index += 1;
    let mut exp = v.exponent + olength as i32 - 1;
    if exp < 0 {
        *result.offset(index) = b'-';
        index += 1;
        exp = -exp;
    }

    if exp >= 10 {
        ptr::copy_nonoverlapping(
            DIGIT_TABLE.get_unchecked((2 * exp) as usize),
            result.offset(index),
            2,
        );
        index += 2;
    } else {
        *result.offset(index) = b'0' + exp as u8;
        index += 1;
    }

    debug_assert!(index <= 15);
    index as usize
}

/// Print f32 to the given buffer and return number of bytes written.
///
/// At most 15 bytes will be written.
///
/// ## Special cases
///
/// This function represents any NaN as `NaN`, positive infinity as `Infinity`,
/// and negative infinity as `-Infinity`.
///
/// ## Safety
///
/// The `result` pointer argument must point to sufficiently many writable bytes
/// to hold Ryū's representation of `f`.
///
/// ## Example
///
/// ```rust
/// let f = 1.234f32;
///
/// unsafe {
///     let mut buffer: [u8; 15] = std::mem::uninitialized();
///     let n = ryu::raw::f2s_buffered_n(f, &mut buffer[0]);
///     let s = std::str::from_utf8_unchecked(&buffer[..n]);
///     assert_eq!(s, "1.234E0");
/// }
/// ```
#[cfg_attr(must_use_return, must_use)]
#[cfg_attr(feature = "no-panic", no_panic)]
pub unsafe fn f2s_buffered_n(f: f32, result: *mut u8) -> usize {
    // Step 1: Decode the floating-point number, and unify normalized and subnormal cases.
    let bits = mem::transmute::<f32, u32>(f).to_le();

    // Decode bits into sign, mantissa, and exponent.
    let ieee_sign = ((bits >> (FLOAT_MANTISSA_BITS + FLOAT_EXPONENT_BITS)) & 1) != 0;
    let ieee_mantissa = bits & ((1u32 << FLOAT_MANTISSA_BITS) - 1);
    let ieee_exponent =
        ((bits >> FLOAT_MANTISSA_BITS) & ((1u32 << FLOAT_EXPONENT_BITS) - 1)) as u32;

    // Case distinction; exit early for the easy cases.
    if ieee_exponent == ((1u32 << FLOAT_EXPONENT_BITS) - 1)
        || (ieee_exponent == 0 && ieee_mantissa == 0)
    {
        return copy_special_str(result, ieee_sign, ieee_exponent != 0, ieee_mantissa != 0);
    }

    let v = f2d(ieee_mantissa, ieee_exponent);
    to_chars(v, ieee_sign, result)
}
