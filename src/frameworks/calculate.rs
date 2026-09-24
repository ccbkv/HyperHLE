/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `Calculate.framework` compatibility for legacy Calculator.app.

use crate::dyld::{export_c_func, FunctionExports, HostDylib};
use crate::mem::{ConstPtr, MutPtr};
use crate::Environment;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CalcError {
    Parse = 1,
    Underflow = 2,
    Overflow = 3,
    DivideByZero = 4,
}

fn CalculatePerformExpression(
    env: &mut Environment,
    expression: ConstPtr<u8>,
    significant_digits: i32,
    _flags: i32,
    answer: MutPtr<u8>,
) -> i32 {
    let Ok(source) = env.mem.cstr_at_utf8(expression) else {
        write_guest_cstring(env, answer, "Error");
        return CalcError::Parse as i32;
    };

    let normalized = source.trim().replace(',', ".");
    let mut parser = Parser::new(&normalized);
    let result = parser
        .parse_expression()
        .and_then(|value| parser.finish(value))
        .and_then(check_value);

    match result {
        Ok(value) => {
            write_guest_cstring(env, answer, &format_result(value, significant_digits));
            0
        }
        Err(CalcError::DivideByZero) => {
            write_guest_cstring(env, answer, "divByZero");
            CalcError::DivideByZero as i32
        }
        Err(CalcError::Underflow) => {
            write_guest_cstring(env, answer, "underflow");
            CalcError::Underflow as i32
        }
        Err(CalcError::Overflow) => {
            write_guest_cstring(env, answer, "overflow");
            CalcError::Overflow as i32
        }
        Err(CalcError::Parse) => {
            write_guest_cstring(env, answer, "Error");
            CalcError::Parse as i32
        }
    }
}

fn write_guest_cstring(env: &mut Environment, ptr: MutPtr<u8>, s: &str) {
    if ptr.is_null() {
        return;
    }
    let bytes = s.as_bytes();
    let total = bytes.len() as u32 + 1;
    let dst = env.mem.bytes_at_mut(ptr, total);
    dst[..bytes.len()].copy_from_slice(bytes);
    dst[bytes.len()] = 0;
}

fn check_value(value: f64) -> Result<f64, CalcError> {
    if value.is_nan() {
        return Err(CalcError::Parse);
    }
    if value.is_infinite() {
        return Err(CalcError::Overflow);
    }
    if value != 0.0 && value.abs() < f64::MIN_POSITIVE {
        return Err(CalcError::Underflow);
    }
    Ok(value)
}

fn format_result(value: f64, significant_digits: i32) -> String {
    if value == 0.0 {
        return "0".into();
    }

    let digits = significant_digits.max(1) as usize;
    let abs = value.abs();
    let exponent = abs.log10().floor() as i32;

    let formatted = if exponent >= digits as i32 || exponent <= -4 {
        format!("{:.*e}", digits.saturating_sub(1), value)
    } else {
        let decimals = (digits as i32 - exponent - 1).max(0) as usize;
        format!("{:.*}", decimals, value)
    };

    trim_number_string(formatted)
}

fn trim_number_string(mut s: String) -> String {
    if let Some(exp_index) = s.find(['e', 'E']) {
        let exponent = s.split_off(exp_index);
        s = trim_plain_number(s);
        return format!("{s}{exponent}");
    }
    trim_plain_number(s)
}

fn trim_plain_number(mut s: String) -> String {
    if let Some(dot_index) = s.find('.') {
        let mut end = s.len();
        while end > dot_index + 1 && s.as_bytes()[end - 1] == b'0' {
            end -= 1;
        }
        if end == dot_index + 1 {
            end -= 1;
        }
        s.truncate(end);
    }
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

struct Parser<'a> {
    input: &'a [u8],
    index: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            index: 0,
        }
    }

    fn finish(&mut self, value: f64) -> Result<f64, CalcError> {
        self.skip_ws();
        if self.index == self.input.len() {
            Ok(value)
        } else {
            Err(CalcError::Parse)
        }
    }

    fn parse_expression(&mut self) -> Result<f64, CalcError> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            if self.consume_byte(b'+') {
                value = check_value(value + self.parse_term()?)?;
            } else if self.consume_byte(b'-') {
                value = check_value(value - self.parse_term()?)?;
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_term(&mut self) -> Result<f64, CalcError> {
        let mut value = self.parse_power()?;
        loop {
            self.skip_ws();
            if self.consume_byte(b'*') {
                value = check_value(value * self.parse_power()?)?;
            } else if self.consume_byte(b'/') {
                let rhs = self.parse_power()?;
                if rhs == 0.0 {
                    return Err(CalcError::DivideByZero);
                }
                value = check_value(value / rhs)?;
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_power(&mut self) -> Result<f64, CalcError> {
        let value = self.parse_unary()?;
        self.skip_ws();
        if self.consume_byte(b'^') {
            let rhs = self.parse_power()?;
            return check_value(value.powf(rhs));
        }
        Ok(value)
    }

    fn parse_unary(&mut self) -> Result<f64, CalcError> {
        self.skip_ws();
        if self.consume_byte(b'+') {
            return self.parse_unary();
        }
        if self.consume_byte(b'-') {
            return check_value(-self.parse_unary()?);
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<f64, CalcError> {
        let mut value = self.parse_primary()?;
        loop {
            self.skip_ws();
            if self.consume_byte(b'!') {
                value = factorial(value)?;
            } else if self.consume_byte(b'%') {
                value = check_value(value / 100.0)?;
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_primary(&mut self) -> Result<f64, CalcError> {
        self.skip_ws();
        if self.consume_byte(b'(') {
            let value = self.parse_expression()?;
            self.skip_ws();
            if !self.consume_byte(b')') {
                return Err(CalcError::Parse);
            }
            return Ok(value);
        }

        if self.peek_is_ident() {
            let ident = self.parse_ident();
            self.skip_ws();
            if self.consume_byte(b'(') {
                let mut args = Vec::new();
                self.skip_ws();
                if !self.consume_byte(b')') {
                    loop {
                        args.push(self.parse_expression()?);
                        self.skip_ws();
                        if self.consume_byte(b',') {
                            continue;
                        }
                        if self.consume_byte(b')') {
                            break;
                        }
                        return Err(CalcError::Parse);
                    }
                }
                return apply_function(&ident, &args);
            }
            return constant_value(&ident).ok_or(CalcError::Parse);
        }

        self.parse_number()
    }

    fn parse_number(&mut self) -> Result<f64, CalcError> {
        self.skip_ws();
        let start = self.index;
        let mut seen_digit = false;
        let mut seen_dot = false;

        while let Some(byte) = self.peek_byte() {
            match byte {
                b'0'..=b'9' => {
                    seen_digit = true;
                    self.index += 1;
                }
                b'.' if !seen_dot => {
                    seen_dot = true;
                    self.index += 1;
                }
                b'e' | b'E' if seen_digit => {
                    let exp_start = self.index;
                    self.index += 1;
                    if matches!(self.peek_byte(), Some(b'+') | Some(b'-')) {
                        self.index += 1;
                    }
                    let mut exp_digits = 0usize;
                    while matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                        self.index += 1;
                        exp_digits += 1;
                    }
                    if exp_digits == 0 {
                        self.index = exp_start;
                    }
                    break;
                }
                _ => break,
            }
        }

        if !seen_digit {
            return Err(CalcError::Parse);
        }

        let slice = std::str::from_utf8(&self.input[start..self.index]).map_err(|_| CalcError::Parse)?;
        let value = slice.parse::<f64>().map_err(|_| CalcError::Parse)?;
        check_value(value)
    }

    fn parse_ident(&mut self) -> String {
        let start = self.index;
        while let Some(byte) = self.peek_byte() {
            if byte.is_ascii_alphanumeric() || byte == b'_' {
                self.index += 1;
            } else {
                break;
            }
        }
        String::from_utf8_lossy(&self.input[start..self.index]).to_ascii_lowercase()
    }

    fn peek_is_ident(&self) -> bool {
        matches!(self.peek_byte(), Some(b'a'..=b'z') | Some(b'A'..=b'Z') | Some(b'_'))
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek_byte(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.index += 1;
        }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.input.get(self.index).copied()
    }

    fn consume_byte(&mut self, expected: u8) -> bool {
        if self.peek_byte() == Some(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }
}

fn constant_value(name: &str) -> Option<f64> {
    match name {
        "pi" => Some(std::f64::consts::PI),
        "e" => Some(std::f64::consts::E),
        "random" | "rand" => Some(0.5),
        _ => None,
    }
}

fn factorial(value: f64) -> Result<f64, CalcError> {
    if value < 0.0 {
        return Err(CalcError::Parse);
    }
    let rounded = value.round();
    if (rounded - value).abs() > 1e-9 {
        return Err(CalcError::Parse);
    }
    if rounded > 170.0 {
        return Err(CalcError::Overflow);
    }
    let mut result = 1.0;
    let mut n = rounded as u32;
    while n > 1 {
        result *= n as f64;
        n -= 1;
    }
    check_value(result)
}

fn apply_function(name: &str, args: &[f64]) -> Result<f64, CalcError> {
    let unary = |f: fn(f64) -> f64| -> Result<f64, CalcError> {
        if args.len() != 1 {
            return Err(CalcError::Parse);
        }
        check_value(f(args[0]))
    };

    match name {
        "pow" => {
            if args.len() != 2 {
                return Err(CalcError::Parse);
            }
            check_value(args[0].powf(args[1]))
        }
        "sqrt" => unary(f64::sqrt),
        "sin" => unary(f64::sin),
        "cos" => unary(f64::cos),
        "tan" => unary(f64::tan),
        "asin" => unary(f64::asin),
        "acos" => unary(f64::acos),
        "atan" => unary(f64::atan),
        "sinh" => unary(f64::sinh),
        "cosh" => unary(f64::cosh),
        "tanh" => unary(f64::tanh),
        "asinh" => unary(f64::asinh),
        "acosh" => unary(f64::acosh),
        "atanh" => unary(f64::atanh),
        "exp" => unary(f64::exp),
        "ln" => unary(f64::ln),
        "log" => unary(f64::log10),
        "log2" => unary(f64::log2),
        "abs" => unary(f64::abs),
        "deg" => unary(f64::to_degrees),
        "rad" => unary(f64::to_radians),
        _ => Err(CalcError::Parse),
    }
}

pub const FUNCTIONS: FunctionExports = &[export_c_func!(CalculatePerformExpression(_, _, _, _))];

pub const DYLIB: HostDylib = HostDylib {
    path: "/System/Library/PrivateFrameworks/Calculate.framework/Calculate",
    aliases: &[],
    class_exports: &[],
    constant_exports: &[],
    function_exports: &[FUNCTIONS],
};
