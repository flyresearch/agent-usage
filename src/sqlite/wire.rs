//! Bounded protobuf wire projection, not a generated schema or a permissive
//! decoder. Only singular fields needed by a reviewed adapter are interpreted.
use crate::{Error, Result};
use std::collections::BTreeMap;
#[derive(Clone, Copy)]
pub enum Value<'a> {
    Integer(u64),
    Bytes(&'a [u8]),
    Fixed,
}
pub struct Message<'a>(BTreeMap<u32, Vec<Value<'a>>>);
fn varint(input: &[u8], offset: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    for n in 0..10 {
        let b = *input.get(*offset).ok_or(Error("invalid_protobuf"))?;
        *offset += 1;
        if n == 9 && b > 1 {
            return Err(Error("protobuf_integer_overflow"));
        }
        value |= u64::from(b & 127) << (7 * n);
        if b & 128 == 0 {
            return Ok(value);
        }
    }
    Err(Error("invalid_protobuf"))
}
impl<'a> Message<'a> {
    pub fn parse(input: &'a [u8]) -> Result<Self> {
        if input.len() > 64 * 1024 * 1024 {
            return Err(Error("protobuf_size_limit"));
        }
        let mut fields: BTreeMap<u32, Vec<Value<'a>>> = BTreeMap::new();
        let mut offset = 0;
        while offset < input.len() {
            let tag = varint(input, &mut offset)?;
            let number = u32::try_from(tag >> 3).map_err(|_| Error("invalid_protobuf"))?;
            if number == 0 || number > 0x1fff_ffff {
                return Err(Error("invalid_protobuf"));
            }
            let value = match tag & 7 {
                0 => Value::Integer(varint(input, &mut offset)?),
                wire @ (1 | 2 | 5) => {
                    let length = match wire {
                        1 => 8,
                        5 => 4,
                        _ => usize::try_from(varint(input, &mut offset)?)
                            .map_err(|_| Error("protobuf_size_limit"))?,
                    };
                    let end = offset
                        .checked_add(length)
                        .ok_or(Error("protobuf_size_limit"))?;
                    let bytes = input.get(offset..end).ok_or(Error("invalid_protobuf"))?;
                    offset = end;
                    if wire == 2 {
                        Value::Bytes(bytes)
                    } else {
                        Value::Fixed
                    }
                }
                _ => return Err(Error("unsupported_protobuf_wire")),
            };
            fields.entry(number).or_default().push(value);
        }
        Ok(Self(fields))
    }
    fn field(&self, n: u32) -> Result<Option<Value<'a>>> {
        match self.0.get(&n) {
            Some(v) if v.len() == 1 => Ok(Some(v[0])),
            Some(_) => Err(Error("duplicate_protobuf_field")),
            None => Ok(None),
        }
    }
    pub fn number(&self, n: u32) -> Result<Option<u64>> {
        match self.field(n)? {
            None => Ok(None),
            Some(Value::Integer(v)) => Ok(Some(v)),
            _ => Err(Error("invalid_protobuf_field_type")),
        }
    }
    pub fn bytes(&self, n: u32) -> Result<Option<&'a [u8]>> {
        match self.field(n)? {
            None => Ok(None),
            Some(Value::Bytes(v)) => Ok(Some(v)),
            _ => Err(Error("invalid_protobuf_field_type")),
        }
    }
    pub fn message(&self, n: u32) -> Result<Option<Self>> {
        self.bytes(n)?.map(Self::parse).transpose()
    }
    pub fn string(&self, n: u32) -> Result<Option<&'a str>> {
        self.bytes(n)?
            .map(|b| std::str::from_utf8(b).map_err(|_| Error("invalid_protobuf_utf8")))
            .transpose()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn unknown_fields(&self, known: &[u32]) -> u64 {
        self.0
            .iter()
            .filter(|(k, _)| !known.contains(k))
            .map(|(_, v)| v.len() as u64)
            .sum()
    }
}
