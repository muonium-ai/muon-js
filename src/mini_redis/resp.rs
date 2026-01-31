//! RESP3 parser/encoder (minimal subset for commands).

use async_std::io::{self, BufReadExt, ReadExt, WriteExt};

#[derive(Debug, Clone)]
pub enum RespValue {
    Simple(String),
    Error(String),
    Integer(i64),
    Blob(Vec<u8>),
    Null,
    Array(Vec<RespValue>),
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
                out.extend_from_slice(n.to_string().as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            RespValue::Blob(bytes) => {
                out.extend_from_slice(b"$");
                out.extend_from_slice(bytes.len().to_string().as_bytes());
                out.extend_from_slice(b"\r\n");
                out.extend_from_slice(bytes);
                out.extend_from_slice(b"\r\n");
            }
            RespValue::Null => {
                out.extend_from_slice(b"_\r\n");
            }
            RespValue::Array(items) => {
                out.extend_from_slice(b"*");
                out.extend_from_slice(items.len().to_string().as_bytes());
                out.extend_from_slice(b"\r\n");
                for item in items {
                    item.encode(out);
                }
            }
        }
    }
}

pub async fn write_value<W: io::Write + Unpin>(writer: &mut W, val: &RespValue) -> io::Result<()> {
    let mut buf = Vec::new();
    val.encode(&mut buf);
    writer.write_all(&buf).await
}

pub async fn read_value<R: io::BufRead + Unpin>(reader: &mut R) -> io::Result<Option<RespValue>> {
    let mut prefix = [0u8; 1];
    let n = reader.read(&mut prefix).await?;
    if n == 0 {
        return Ok(None);
    }
    let tag = prefix[0];
    match tag {
        b'+' => Ok(Some(RespValue::Simple(read_line(reader).await?))),
        b'-' => Ok(Some(RespValue::Error(read_line(reader).await?))),
        b':' => {
            let line = read_line(reader).await?;
            let val = line.trim().parse::<i64>().unwrap_or(0);
            Ok(Some(RespValue::Integer(val)))
        }
        b'_' => {
            let _ = read_line(reader).await?;
            Ok(Some(RespValue::Null))
        }
        b'$' => {
            let line = read_line(reader).await?;
            let len = line.trim().parse::<isize>().unwrap_or(-1);
            if len < 0 {
                return Ok(Some(RespValue::Null));
            }
            let mut buf = vec![0u8; len as usize];
            reader.read_exact(&mut buf).await?;
            let mut crlf = [0u8; 2];
            reader.read_exact(&mut crlf).await?;
            Ok(Some(RespValue::Blob(buf)))
        }
        b'*' => {
            let line = read_line(reader).await?;
            let len = line.trim().parse::<isize>().unwrap_or(-1);
            if len < 0 {
                return Ok(Some(RespValue::Null));
            }
            let mut items = Vec::with_capacity(len as usize);
            for _ in 0..len {
                if let Some(v) = read_value(reader).await? {
                    items.push(v);
                } else {
                    return Ok(None);
                }
            }
            Ok(Some(RespValue::Array(items)))
        }
        _ => Ok(Some(RespValue::Error("ERR unknown RESP type".to_string()))),
    }
}

async fn read_line<R: io::BufRead + Unpin>(reader: &mut R) -> io::Result<String> {
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    if line.ends_with("\r\n") {
        line.truncate(line.len().saturating_sub(2));
    } else if line.ends_with('\n') {
        line.truncate(line.len().saturating_sub(1));
    }
    Ok(line)
}
