//! RESP3 parser/encoder (minimal subset for commands).

use tokio::io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, AsyncWrite, AsyncBufRead};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum RespValue {
    Simple(String),
    Error(String),
    Integer(i64),
    Blob(Arc<[u8]>),
    Null,
    Array(Vec<RespValue>),
    /// Zero-alloc simple string response (e.g. "OK", "PONG")
    StaticSimple(&'static str),
    /// Zero-alloc error response (e.g. "WRONGTYPE ...")
    StaticError(&'static str),
}

fn push_usize(buf: &mut Vec<u8>, mut n: usize) {
    if n == 0 {
        buf.push(b'0');
        return;
    }
    let mut tmp = [0u8; 20];
    let mut i = tmp.len();
    while n > 0 {
        i -= 1;
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    buf.extend_from_slice(&tmp[i..]);
}

fn push_i64(buf: &mut Vec<u8>, n: i64) {
    if n == 0 {
        buf.push(b'0');
        return;
    }
    let mut v: u64 = if n < 0 {
        buf.push(b'-');
        (-(n as i128)) as u64
    } else {
        n as u64
    };
    let mut tmp = [0u8; 20];
    let mut i = tmp.len();
    while v > 0 {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    buf.extend_from_slice(&tmp[i..]);
}

impl RespValue {
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            RespValue::Simple(s) => {
                out.extend_from_slice(b"+");
                out.extend_from_slice(s.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            RespValue::StaticSimple(s) => {
                out.extend_from_slice(b"+");
                out.extend_from_slice(s.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            RespValue::Error(s) => {
                out.extend_from_slice(b"-");
                out.extend_from_slice(s.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            RespValue::StaticError(s) => {
                out.extend_from_slice(b"-");
                out.extend_from_slice(s.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            RespValue::Integer(n) => {
                out.extend_from_slice(b":");
                push_i64(out, *n);
                out.extend_from_slice(b"\r\n");
            }
            RespValue::Blob(bytes) => {
                out.extend_from_slice(b"$");
                push_usize(out, bytes.len());
                out.extend_from_slice(b"\r\n");
                out.extend_from_slice(bytes.as_ref());
                out.extend_from_slice(b"\r\n");
            }
            RespValue::Null => {
                out.extend_from_slice(b"_\r\n");
            }
            RespValue::Array(items) => {
                out.extend_from_slice(b"*");
                push_usize(out, items.len());
                out.extend_from_slice(b"\r\n");
                for item in items {
                    item.encode(out);
                }
            }
        }
    }
}

pub async fn write_value<W: AsyncWrite + Unpin>(writer: &mut W, val: &RespValue) -> io::Result<()> {
    let mut buf = Vec::with_capacity(estimate_encoded_len(val));
    val.encode(&mut buf);
    writer.write_all(&buf).await
}

pub async fn write_value_buf<W: AsyncWrite + Unpin>(
    writer: &mut W,
    val: &RespValue,
    buf: &mut Vec<u8>,
) -> io::Result<()> {
    buf.clear();
    buf.reserve(estimate_encoded_len(val));
    val.encode(buf);
    writer.write_all(buf).await
}

pub async fn write_array_of_blobs_buf<W: AsyncWrite + Unpin>(
    writer: &mut W,
    items: &[Arc<[u8]>],
    buf: &mut Vec<u8>,
) -> io::Result<()> {
    buf.clear();
    encode_array_of_blobs(items, buf);
    writer.write_all(buf).await
}

fn encode_array_of_blobs(items: &[Arc<[u8]>], out: &mut Vec<u8>) {
    let payload_len = items
        .iter()
        .map(|item| 16 + item.len())
        .sum::<usize>();
    out.reserve(16 + payload_len);
    out.extend_from_slice(b"*");
    push_usize(out, items.len());
    out.extend_from_slice(b"\r\n");
    for item in items {
        out.extend_from_slice(b"$");
        push_usize(out, item.len());
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(item.as_ref());
        out.extend_from_slice(b"\r\n");
    }
}

fn estimate_encoded_len(value: &RespValue) -> usize {
    match value {
        RespValue::Simple(s) | RespValue::Error(s) => 3 + s.len(),
        RespValue::StaticSimple(s) | RespValue::StaticError(s) => 3 + s.len(),
        RespValue::Integer(_) => 24,
        RespValue::Blob(bytes) => 16 + bytes.len(),
        RespValue::Null => 3,
        RespValue::Array(items) => 16 + items.iter().map(estimate_encoded_len).sum::<usize>(),
    }
}

pub async fn read_value<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> io::Result<Option<RespValue>> {
    let mut line_buf = Vec::with_capacity(128);
    let tag = match read_prefix(reader).await? {
        Some(tag) => tag,
        None => return Ok(None),
    };
    read_value_from_tag(reader, tag, &mut line_buf).await
}

struct ArrayFrame {
    remaining: usize,
    items: Vec<RespValue>,
}

impl ArrayFrame {
    fn new(len: usize) -> Self {
        Self {
            remaining: len,
            items: Vec::with_capacity(len),
        }
    }
}

async fn read_value_from_tag<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    tag: u8,
    line_buf: &mut Vec<u8>,
) -> io::Result<Option<RespValue>> {
    if tag != b'*' {
        return read_single_element(reader, tag, line_buf, true).await;
    }

    let len = read_array_len(reader, line_buf).await?;
    if len < 0 {
        return Ok(Some(RespValue::Null));
    }

    // Iterative array parser with explicit stack, so nested arrays avoid recursion.
    let mut current = ArrayFrame::new(len as usize);
    let mut stack: Vec<ArrayFrame> = Vec::new();
    loop {
        if current.remaining == 0 {
            let completed = RespValue::Array(current.items);
            if let Some(mut parent) = stack.pop() {
                parent.items.push(completed);
                parent.remaining -= 1;
                current = parent;
                continue;
            }
            return Ok(Some(completed));
        }

        let tag = match read_prefix(reader).await? {
            Some(tag) => tag,
            None => return Ok(None),
        };

        if tag == b'*' {
            let len = read_array_len(reader, line_buf).await?;
            if len < 0 {
                current.items.push(RespValue::Null);
                current.remaining -= 1;
            } else {
                stack.push(current);
                current = ArrayFrame::new(len as usize);
            }
            continue;
        }

        let value = match read_single_element(reader, tag, line_buf, false).await? {
            Some(value) => value,
            None => return Ok(None),
        };
        current.items.push(value);
        current.remaining -= 1;
    }
}

async fn read_prefix<R: AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<Option<u8>> {
    let mut prefix = [0u8; 1];
    let n = reader.read(&mut prefix).await?;
    if n == 0 {
        Ok(None)
    } else {
        Ok(Some(prefix[0]))
    }
}

async fn read_array_len<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    line_buf: &mut Vec<u8>,
) -> io::Result<i64> {
    let line = read_line_bytes(reader, line_buf).await?;
    Ok(parse_i64_bytes(line).unwrap_or(-1))
}

async fn read_single_element<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    tag: u8,
    line_buf: &mut Vec<u8>,
    allow_inline: bool,
) -> io::Result<Option<RespValue>> {
    match tag {
        b'+' => {
            let line = read_line_bytes(reader, line_buf).await?;
            Ok(Some(RespValue::Simple(String::from_utf8_lossy(line).into_owned())))
        }
        b'-' => {
            let line = read_line_bytes(reader, line_buf).await?;
            Ok(Some(RespValue::Error(String::from_utf8_lossy(line).into_owned())))
        }
        b':' => {
            let line = read_line_bytes(reader, line_buf).await?;
            let val = parse_i64_bytes(line).unwrap_or(0);
            Ok(Some(RespValue::Integer(val)))
        }
        b'_' => {
            let _ = read_line_bytes(reader, line_buf).await?;
            Ok(Some(RespValue::Null))
        }
        b'$' => {
            let line = read_line_bytes(reader, line_buf).await?;
            let len = parse_i64_bytes(line).unwrap_or(-1);
            if len < 0 {
                return Ok(Some(RespValue::Null));
            }
            let mut buf = vec![0u8; len as usize];
            reader.read_exact(&mut buf).await?;
            let mut crlf = [0u8; 2];
            reader.read_exact(&mut crlf).await?;
            Ok(Some(RespValue::Blob(Arc::from(buf))))
        }
        _ if allow_inline => read_inline_command(reader, tag, line_buf).await,
        _ => Ok(Some(RespValue::StaticError("ERR unknown RESP type"))),
    }
}

async fn read_inline_command<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    first_byte: u8,
    line_buf: &mut Vec<u8>,
) -> io::Result<Option<RespValue>> {
    line_buf.clear();
    line_buf.push(first_byte);
    reader.read_until(b'\n', line_buf).await?;
    if line_buf.ends_with(b"\r\n") {
        line_buf.truncate(line_buf.len().saturating_sub(2));
    } else if line_buf.ends_with(b"\n") {
        line_buf.truncate(line_buf.len().saturating_sub(1));
    }

    let mut args = Vec::new();
    for part in line_buf.split(|b| b.is_ascii_whitespace()) {
        if !part.is_empty() {
            args.push(RespValue::Blob(Arc::from(part.to_vec())));
        }
    }
    Ok(Some(RespValue::Array(args)))
}

async fn read_line_bytes<'a, R: AsyncBufRead + Unpin>(
    reader: &'a mut R,
    line_buf: &'a mut Vec<u8>,
) -> io::Result<&'a [u8]> {
    line_buf.clear();
    reader.read_until(b'\n', line_buf).await?;
    if line_buf.ends_with(b"\r\n") {
        line_buf.truncate(line_buf.len().saturating_sub(2));
    } else if line_buf.ends_with(b"\n") {
        line_buf.truncate(line_buf.len().saturating_sub(1));
    }
    Ok(line_buf.as_slice())
}

fn parse_i64_bytes(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() {
        return None;
    }
    let mut index = 0usize;
    let mut negative = false;
    if bytes[0] == b'-' {
        negative = true;
        index = 1;
    }
    if index >= bytes.len() {
        return None;
    }
    let mut value: i64 = 0;
    while index < bytes.len() {
        let c = bytes[index];
        if !c.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?;
        value = value.checked_add((c - b'0') as i64)?;
        index += 1;
    }
    if negative {
        Some(-value)
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    async fn parse(input: &[u8]) -> RespValue {
        let mut reader = BufReader::new(input);
        read_value(&mut reader)
            .await
            .expect("RESP read should succeed")
            .expect("RESP frame should exist")
    }

    fn expect_blob(value: &RespValue, expected: &[u8]) {
        match value {
            RespValue::Blob(bytes) => assert_eq!(bytes.as_ref(), expected),
            _ => panic!("expected blob"),
        }
    }

    #[tokio::test]
    async fn parses_nested_arrays_without_recursion() {
        let input = b"*2\r\n$4\r\nECHO\r\n*2\r\n$3\r\nfoo\r\n$3\r\nbar\r\n";
        let value = parse(input).await;
        match value {
            RespValue::Array(items) => {
                assert_eq!(items.len(), 2);
                expect_blob(&items[0], b"ECHO");
                match &items[1] {
                    RespValue::Array(nested) => {
                        assert_eq!(nested.len(), 2);
                        expect_blob(&nested[0], b"foo");
                        expect_blob(&nested[1], b"bar");
                    }
                    _ => panic!("expected nested array"),
                }
            }
            _ => panic!("expected array"),
        }
    }

    #[tokio::test]
    async fn parses_inline_command_into_blob_array() {
        let value = parse(b"SET mykey value PX 10\r\n").await;
        match value {
            RespValue::Array(items) => {
                assert_eq!(items.len(), 5);
                expect_blob(&items[0], b"SET");
                expect_blob(&items[1], b"mykey");
                expect_blob(&items[2], b"value");
                expect_blob(&items[3], b"PX");
                expect_blob(&items[4], b"10");
            }
            _ => panic!("expected array"),
        }
    }

    #[tokio::test]
    async fn parses_null_bulk_string() {
        let value = parse(b"$-1\r\n").await;
        assert!(matches!(value, RespValue::Null));
    }

    #[tokio::test]
    async fn parses_null_bulk_inside_array() {
        let value = parse(b"*2\r\n$3\r\nGET\r\n$-1\r\n").await;
        match value {
            RespValue::Array(items) => {
                assert_eq!(items.len(), 2);
                expect_blob(&items[0], b"GET");
                assert!(matches!(items[1], RespValue::Null));
            }
            _ => panic!("expected array"),
        }
    }
}
