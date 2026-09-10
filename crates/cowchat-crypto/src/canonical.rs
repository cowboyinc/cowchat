//! Bounded RFC8949 core deterministic profile. Preflight checks run before a
//! general-purpose decoder can allocate based on attacker-supplied lengths.
use crate::{Error, Result};
use ciborium::value::Value;

pub const MAX_BYTES: usize = 65_536;
pub const MAX_DEPTH: usize = 16;
pub const MAX_VALUES: usize = 4096;

struct Scan<'a> {
    raw: &'a [u8],
    pos: usize,
    values: usize,
}
impl Scan<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::Limit)?;
        let bytes = self.raw.get(self.pos..end).ok_or(Error::Encoding)?;
        self.pos = end;
        Ok(bytes)
    }
    fn item(&mut self, depth: usize) -> Result<()> {
        self.values += 1;
        if depth > MAX_DEPTH || self.values > MAX_VALUES {
            return Err(Error::Limit);
        }
        let first = self.take(1)?[0];
        let major = first >> 5;
        let ai = first & 31;
        if major == 7 {
            return if matches!(ai, 20..=22) {
                Ok(())
            } else {
                Err(Error::Encoding)
            };
        }
        if !matches!(major, 0 | 2 | 3 | 4 | 5) {
            return Err(Error::Encoding);
        }
        let size = match ai {
            0..=23 => ai as u64,
            24..=27 => {
                let n = 1usize << (ai - 24);
                let mut number = 0u64;
                for b in self.take(n)? {
                    number = (number << 8) | u64::from(*b);
                }
                let minimum = [24u64, 256, 65_536, 4_294_967_296][(ai - 24) as usize];
                if number < minimum {
                    return Err(Error::Encoding);
                }
                number
            }
            _ => return Err(Error::Encoding),
        };
        match major {
            0 => (),
            2 | 3 => {
                let length = usize::try_from(size).map_err(|_| Error::Limit)?;
                let text = self.take(length)?;
                if major == 3 {
                    std::str::from_utf8(text).map_err(|_| Error::Encoding)?;
                }
            }
            4 | 5 => {
                let n = size
                    .checked_mul(if major == 5 { 2 } else { 1 })
                    .ok_or(Error::Limit)?;
                if n > (MAX_VALUES - self.values) as u64 {
                    return Err(Error::Limit);
                }
                for _ in 0..n {
                    self.item(depth + 1)?;
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}

fn parse(raw: &[u8]) -> Result<Value> {
    if raw.len() > MAX_BYTES {
        return Err(Error::Limit);
    }
    let mut scan = Scan {
        raw,
        pos: 0,
        values: 0,
    };
    scan.item(0)?;
    if scan.pos != raw.len() {
        return Err(Error::Encoding);
    }
    ciborium::from_reader(raw).map_err(|_| Error::Encoding)
}

fn sorted(value: Value) -> Result<Value> {
    Ok(match value {
        Value::Map(items) => {
            let mut entries = Vec::with_capacity(items.len());
            for (key, value) in items {
                if !matches!(key, Value::Text(_)) {
                    return Err(Error::Encoding);
                }
                entries.push((encode_raw(&key)?, key, sorted(value)?));
            }
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            if entries.windows(2).any(|p| p[0].0 == p[1].0) {
                return Err(Error::Encoding);
            }
            Value::Map(entries.into_iter().map(|(_, k, v)| (k, v)).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect::<Result<_>>()?),
        other => other,
    })
}

fn encode_raw(value: &Value) -> Result<Vec<u8>> {
    let mut raw = Vec::new();
    ciborium::into_writer(value, &mut raw).map_err(|_| Error::Encoding)?;
    Ok(raw)
}

/// Convert this bounded CBOR profile to deterministic key order. Duplicate
/// keys and non-preferred encodings are rejected, never silently repaired.
pub fn canonicalize(raw: &[u8]) -> Result<Vec<u8>> {
    encode_raw(&sorted(parse(raw)?)?)
}

/// Validate one already deterministic, bounded CBOR object.
pub fn validate(raw: &[u8]) -> Result<()> {
    if canonicalize(raw)? != raw {
        return Err(Error::Encoding);
    }
    Ok(())
}

pub(crate) fn decode(raw: &[u8]) -> Result<Value> {
    let value = sorted(parse(raw)?)?;
    if encode_raw(&value)? != raw {
        return Err(Error::Encoding);
    }
    Ok(value)
}

pub(crate) fn encode(value: Value) -> Result<Vec<u8>> {
    let raw = encode_raw(&sorted(value)?)?;
    validate(&raw)?;
    Ok(raw)
}
