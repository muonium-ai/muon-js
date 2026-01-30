//! JSON parsing and stringification module
//! 
//! Provides JSON.parse() and JSON.stringify() functionality

use crate::context::Context as JSContextImpl;
use crate::types::*;
use crate::value::Value;

/// Parse JSON string into JSValue
pub fn parse_json(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let mut parser = JsonParser::new(src.as_bytes());
    let val = parser.parse_value(ctx)?;
    parser.skip_ws();
    if parser.pos != parser.input.len() {
        return None;
    }
    Some(val)
}

/// Stringify JSValue into JSON string
pub fn json_stringify_value(ctx: &mut JSContextImpl, value: JSValue) -> String {
    use crate::api::{js_is_string, js_get_property_uint32, js_get_property_str};
    
    // Check string first
    if js_is_string(ctx, value) != 0 {
        // Escape and quote strings
        if let Some(bytes) = ctx.string_bytes(value) {
            let s = core::str::from_utf8(bytes).unwrap_or("");
            let mut result = String::from("\"");
            for ch in s.chars() {
                match ch {
                    '\"' => result.push_str("\\\""),
                    '\\' => result.push_str("\\\\"),
                    '\n' => result.push_str("\\n"),
                    '\r' => result.push_str("\\r"),
                    '\t' => result.push_str("\\t"),
                    _ => result.push(ch),
                }
            }
            result.push('\"');
            return result;
        } else {
            return String::from("\"\"");
        }
    }
    
    // Check number
    if value.is_int() {
        return value.int32().unwrap_or(0).to_string();
    }
    
    // Check bool
    if value.is_bool() {
        return if value == Value::TRUE {
            String::from("true")
        } else {
            String::from("false")
        };
    }
    
    // Check null
    if value.is_null() {
        return String::from("null");
    }
    
    // Check undefined
    if value.is_undefined() {
        return String::from("undefined");
    }
    
    // Check if it's an array
    if let Some(class_id) = ctx.object_class_id(value) {
        if class_id == JSObjectClassEnum::Array as u32 {
            // Stringify as array
            let mut result = String::from("[");
            if let Some(arr_len_val) = ctx.get_property_str(value, b"length") {
                if let Some(len) = arr_len_val.int32() {
                    for i in 0..len {
                        if i > 0 {
                            result.push(',');
                        }
                        let elem = js_get_property_uint32(ctx, value, i as u32);
                        result.push_str(&json_stringify_value(ctx, elem));
                    }
                }
            }
            result.push(']');
            return result;
        }
    }
    
    // Stringify as object
    let mut result = String::from("{");
    if let Some(keys) = ctx.object_keys(value) {
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                result.push(',');
            }
            result.push('\"');
            result.push_str(key);
            result.push_str("\":");
            let prop_val = js_get_property_str(ctx, value, key);
            result.push_str(&json_stringify_value(ctx, prop_val));
        }
    }
    result.push('}');
    result
}

struct JsonParser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\n' || b == b'\r' || b == b'\t' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn expect(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_value(&mut self, ctx: &mut JSContextImpl) -> Option<JSValue> {
        use crate::api::{js_new_string, js_new_array, js_new_object, js_set_property_str, js_array_push};
        use crate::helpers::number_to_value;
        
        self.skip_ws();
        match self.peek()? {
            b'{' => self.parse_object(ctx),
            b'[' => self.parse_array(ctx),
            b'"' => {
                let bytes = self.parse_string_bytes()?;
                let s = core::str::from_utf8(&bytes).ok()?;
                Some(js_new_string(ctx, s))
            }
            b'-' | b'0'..=b'9' => self.parse_number(ctx),
            b't' => {
                if self.consume_literal(b"true") {
                    Some(Value::TRUE)
                } else {
                    None
                }
            }
            b'f' => {
                if self.consume_literal(b"false") {
                    Some(Value::FALSE)
                } else {
                    None
                }
            }
            b'n' => {
                if self.consume_literal(b"null") {
                    Some(Value::NULL)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn consume_literal(&mut self, lit: &[u8]) -> bool {
        if self.input.len().saturating_sub(self.pos) < lit.len() {
            return false;
        }
        if &self.input[self.pos..self.pos + lit.len()] == lit {
            self.pos += lit.len();
            true
        } else {
            false
        }
    }

    fn parse_array(&mut self, ctx: &mut JSContextImpl) -> Option<JSValue> {
        use crate::api::{js_new_array, js_array_push};
        
        if !self.expect(b'[') {
            return None;
        }
        self.skip_ws();
        let arr = js_new_array(ctx, 0);
        if arr.is_exception() {
            return None;
        }
        if self.expect(b']') {
            return Some(arr);
        }
        loop {
            let val = self.parse_value(ctx)?;
            let res = js_array_push(ctx, arr, val);
            if res.is_exception() {
                return None;
            }
            self.skip_ws();
            if self.expect(b',') {
                self.skip_ws();
                continue;
            }
            if self.expect(b']') {
                break;
            }
            return None;
        }
        Some(arr)
    }

    fn parse_object(&mut self, ctx: &mut JSContextImpl) -> Option<JSValue> {
        use crate::api::{js_new_object, js_set_property_str};
        
        if !self.expect(b'{') {
            return None;
        }
        self.skip_ws();
        let obj = js_new_object(ctx);
        if obj.is_exception() {
            return None;
        }
        if self.expect(b'}') {
            return Some(obj);
        }
        loop {
            self.skip_ws();
            let key_bytes = self.parse_string_bytes()?;
            let key = core::str::from_utf8(&key_bytes).ok()?;
            self.skip_ws();
            if !self.expect(b':') {
                return None;
            }
            let val = self.parse_value(ctx)?;
            let res = js_set_property_str(ctx, obj, key, val);
            if res.is_exception() {
                return None;
            }
            self.skip_ws();
            if self.expect(b',') {
                continue;
            }
            if self.expect(b'}') {
                break;
            }
            return None;
        }
        Some(obj)
    }

    fn parse_string_bytes(&mut self) -> Option<Vec<u8>> {
        if !self.expect(b'"') {
            return None;
        }
        let mut out = Vec::new();
        while let Some(b) = self.next() {
            match b {
                b'"' => return Some(out),
                b'\\' => {
                    let esc = self.next()?;
                    match esc {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            use crate::helpers::{is_high_surrogate, is_low_surrogate};
                            
                            let code = self.parse_hex4()? as u32;
                            let code = if is_high_surrogate(code) {
                                if self.next() != Some(b'\\') || self.next() != Some(b'u') {
                                    return None;
                                }
                                let low = self.parse_hex4()? as u32;
                                if !is_low_surrogate(low) {
                                    return None;
                                }
                                0x10000 + (((code - 0xD800) << 10) | (low - 0xDC00))
                            } else {
                                if is_low_surrogate(code) {
                                    return None;
                                }
                                code
                            };
                            let ch = char::from_u32(code)?;
                            let mut buf = [0u8; 4];
                            let s = ch.encode_utf8(&mut buf);
                            out.extend_from_slice(s.as_bytes());
                        }
                        _ => return None,
                    }
                }
                b if b < 0x20 => return None,
                _ => out.push(b),
            }
        }
        None
    }

    fn parse_hex4(&mut self) -> Option<u16> {
        let mut val = 0u16;
        for _ in 0..4 {
            let b = self.next()?;
            let digit = hex_val(b)? as u16;
            val = (val << 4) | digit;
        }
        Some(val)
    }

    fn parse_number(&mut self, ctx: &mut JSContextImpl) -> Option<JSValue> {
        use crate::helpers::number_to_value;
        
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek()? {
            b'0' => {
                self.pos += 1;
            }
            b'1'..=b'9' => {
                self.pos += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return None;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return None;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let s = core::str::from_utf8(&self.input[start..self.pos]).ok()?;
        let num = s.parse::<f64>().ok()?;
        Some(number_to_value(ctx, num))
    }
}

fn hex_val(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u32),
        b'a'..=b'f' => Some((b - b'a' + 10) as u32),
        b'A'..=b'F' => Some((b - b'A' + 10) as u32),
        _ => None,
    }
}
