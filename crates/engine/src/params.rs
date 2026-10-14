//! Typed access to JSON command params. Every accessor returns an error on a missing or
//! wrong-typed value; params are hostile input.

use dbcraft_core::{Error, Result};
use serde_json::Value as Json;

pub fn str_req<'a>(p: &'a Json, key: &str) -> Result<&'a str> {
    p.get(key).and_then(Json::as_str).ok_or_else(|| Error::InvalidParam(format!("`{key}` (string) is required")))
}

pub fn str_opt<'a>(p: &'a Json, key: &str) -> Result<Option<&'a str>> {
    match p.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => v.as_str().map(Some).ok_or_else(|| Error::InvalidParam(format!("`{key}` must be a string"))),
    }
}

pub fn u64_opt(p: &Json, key: &str) -> Result<Option<u64>> {
    match p.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| Error::InvalidParam(format!("`{key}` must be a non-negative integer"))),
    }
}
