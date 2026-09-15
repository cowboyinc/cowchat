use crate::{Error, Result};
use ciborium::value::Value;
use std::collections::BTreeMap;

pub(crate) struct Fields(BTreeMap<String, Value>);
impl Fields {
    pub fn new(value: Value, keys: &[&str]) -> Result<Self> {
        let Value::Map(items) = value else {
            return Err(Error::Schema);
        };
        let mut map = BTreeMap::new();
        for (key, value) in items {
            let Value::Text(key) = key else {
                return Err(Error::Schema);
            };
            if map.insert(key, value).is_some() {
                return Err(Error::Encoding);
            }
        }
        if map.len() != keys.len() || keys.iter().any(|k| !map.contains_key(*k)) {
            return Err(Error::Schema);
        }
        Ok(Self(map))
    }
    pub fn get(&self, key: &str) -> Result<&Value> {
        self.0.get(key).ok_or(Error::Schema)
    }
    pub fn text(&self, key: &str) -> Result<String> {
        match self.get(key)? {
            Value::Text(s) if !s.is_empty() => Ok(s.clone()),
            _ => Err(Error::Schema),
        }
    }
    pub fn nullable_text(&self, key: &str) -> Result<Option<String>> {
        match self.get(key)? {
            Value::Null => Ok(None),
            _ => self.text(key).map(Some),
        }
    }
    pub fn uint(&self, key: &str) -> Result<u64> {
        match self.get(key)? {
            Value::Integer(i) => u64::try_from(*i).map_err(|_| Error::Schema),
            _ => Err(Error::Schema),
        }
    }
    pub fn nullable_uint(&self, key: &str) -> Result<Option<u64>> {
        match self.get(key)? {
            Value::Null => Ok(None),
            _ => self.uint(key).map(Some),
        }
    }
    pub fn bytes<const N: usize>(&self, key: &str) -> Result<[u8; N]> {
        match self.get(key)? {
            Value::Bytes(b) => b.as_slice().try_into().map_err(|_| Error::Schema),
            _ => Err(Error::Schema),
        }
    }
    pub fn strings(&self, key: &str) -> Result<Vec<String>> {
        let Value::Array(items) = self.get(key)? else {
            return Err(Error::Schema);
        };
        items
            .iter()
            .map(|v| match v {
                Value::Text(s) if !s.is_empty() => Ok(s.clone()),
                _ => Err(Error::Schema),
            })
            .collect()
    }
}
