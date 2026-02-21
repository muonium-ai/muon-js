//! RESP3 parser/encoder (minimal subset for commands).

use tokio::io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, AsyncWrite, AsyncBufRead};
use std::sync::Arc;
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Clone)]
pub enum RespValue {
    Simple(String),
    Error(String),
    Integer(i64),
    Blob(Arc<[u8]>),
    Null,
    Array(Vec<RespValue>),
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
            RespValue::Error(s) => {
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
        RespValue::Integer(_) => 24,
        RespValue::Blob(bytes) => 16 + bytes.len(),
        RespValue::Null => 3,
        RespValue::Array(items) => 16 + items.iter().map(estimate_encoded_len).sum::<usize>(),
    }
}

pub fn read_value<'a, R>(
    reader: &'a mut R,
) -> Pin<Box<dyn Future<Output = io::Result<Option<RespValue>>> + Send + 'a>>
where
    R: AsyncBufRead + Unpin + Send + 'a,
{
    Box::pin(async move {
        let mut line_buf = Vec::with_capacity(128);
        read_value_inner(reader, &mut line_buf).await
    })
}

fn read_value_inner<'a, R>(
    reader: &'a mut R,
    line_buf: &'a mut Vec<u8>,
) -> Pin<Box<dyn Future<Output = io::Result<Option<RespValue>>> + Send + 'a>>
where
    R: AsyncBufRead + Unpin + Send + 'a,
{
    Box::pin(async move {
        let mut prefix = [0u8; 1];
        let n = reader.read(&mut prefix).await?;
        if n == 0 {
            return Ok(None);
        }
        let tag = prefix[0];
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
            b'*' => {
                let line = read_line_bytes(reader, line_buf).await?;
                let len = parse_i64_bytes(line).unwrap_or(-1);
                if len < 0 {
                    return Ok(Some(RespValue::Null));
                }
                let mut items = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    if let Some(v) = read_value_inner(reader, line_buf).await? {
                        items.push(v);
                    } else {
                        return Ok(None);
                    }
                }
                Ok(Some(RespValue::Array(items)))
            }
            _ => Ok(Some(RespValue::Error("ERR unknown RESP type".to_string()))),
        }
    })
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
