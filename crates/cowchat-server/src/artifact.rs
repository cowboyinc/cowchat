//! Published room artifacts. Authored views are declarative OpenUI; data is independent.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_ARTIFACT_BYTES: usize = 512 * 1024;
pub const MAX_VIEW_BYTES: usize = 64 * 1024;
const MAX_DEPTH: usize = 16;
const MAX_NODES: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Artifact {
    pub room_id: String,
    pub title: String,
    pub view: String,
    pub data: Value,
    pub revision: u64,
    pub updated_at: String,
    pub public_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishArtifact {
    pub title: String,
    pub view: String,
    #[serde(
        default,
        deserialize_with = "optional_data",
        skip_serializing_if = "Option::is_none"
    )]
    pub data: Option<Value>,
    pub expected_revision: Option<u64>,
    pub request_id: String,
}

fn optional_data<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchArtifact {
    pub patch: Value,
    pub expected_revision: Option<u64>,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotateArtifact {
    pub request_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("artifact not found")]
    NotFound,
    #[error("artifact revision or request ID conflict")]
    Conflict,
    #[error("artifact mutation rate limit exceeded")]
    RateLimited,
    #[error("invalid artifact: {0}")]
    Invalid(String),
    #[error("artifact storage error")]
    Storage(#[from] super::store::StoreError),
}

pub fn validate_request_id(id: &str) -> Result<(), ArtifactError> {
    if uuid::Uuid::parse_str(id).is_ok_and(|id| id.get_version_num() == 4) {
        Ok(())
    } else {
        Err(ArtifactError::Invalid("request_id must be a UUIDv4".into()))
    }
}

pub fn validate_document(title: &str, view: &str, data: &Value) -> Result<(), ArtifactError> {
    if title.trim().is_empty() || title.chars().count() > 200 || title.chars().any(char::is_control)
    {
        return Err(ArtifactError::Invalid(
            "title must contain 1-200 characters without controls".into(),
        ));
    }
    if view.trim().is_empty() || view.len() > MAX_VIEW_BYTES {
        return Err(ArtifactError::Invalid(
            "view must contain 1-65536 bytes".into(),
        ));
    }
    validate_data(data)?;
    if title.len() + view.len() + serde_json::to_vec(data).unwrap().len() > MAX_ARTIFACT_BYTES {
        return Err(ArtifactError::Invalid("document exceeds 512 KiB".into()));
    }
    Ok(())
}

pub fn validate_data(data: &Value) -> Result<(), ArtifactError> {
    if !data.is_object() {
        return Err(ArtifactError::Invalid(
            "data and patch must be JSON objects".into(),
        ));
    }
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), ArtifactError> {
        *nodes += 1;
        if depth > MAX_DEPTH || *nodes > MAX_NODES {
            return Err(ArtifactError::Invalid(
                "JSON exceeds depth or item limit".into(),
            ));
        }
        match value {
            Value::Object(map) => {
                const AST_KINDS: &[&str] = &[
                    "Comp",
                    "Str",
                    "Num",
                    "Bool",
                    "Null",
                    "Arr",
                    "Obj",
                    "Ref",
                    "Ph",
                    "StateRef",
                    "RuntimeRef",
                    "BinOp",
                    "UnaryOp",
                    "Ternary",
                    "Member",
                    "Index",
                    "Assign",
                ];
                let reactive = map.get("__reactive").is_some_and(|value| match value {
                    Value::Null | Value::Bool(false) => false,
                    Value::Number(number) => number.as_f64() != Some(0.0),
                    Value::String(text) => !text.is_empty(),
                    _ => true,
                });
                if map
                    .get("k")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| AST_KINDS.contains(&kind))
                    || map.get("type").and_then(Value::as_str) == Some("element")
                    || reactive
                    || map.get("steps").is_some_and(Value::is_array)
                {
                    return Err(ArtifactError::Invalid(
                        "data contains a reserved OpenUI control object".into(),
                    ));
                }
                for (key, child) in map {
                    if matches!(key.as_str(), "__proto__" | "prototype" | "constructor") {
                        return Err(ArtifactError::Invalid(
                            "data contains a reserved prototype key".into(),
                        ));
                    }
                    if key.len() > 512 || key.chars().any(char::is_control) {
                        return Err(ArtifactError::Invalid("invalid JSON key".into()));
                    }
                    visit(child, depth + 1, nodes)?;
                }
            }
            Value::Array(array) => {
                for child in array {
                    visit(child, depth + 1, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(data, 0, &mut 0)?;
    if serde_json::to_vec(data).unwrap().len() > MAX_ARTIFACT_BYTES {
        return Err(ArtifactError::Invalid("data exceeds 512 KiB".into()));
    }
    Ok(())
}

/// RFC 7396: null removes object members; arrays and scalar values replace.
/// Callers validate the patch before recursion and the merged document before commit.
pub fn merge_patch(target: &mut Value, patch: &Value) {
    if let Value::Object(patch) = patch {
        if !target.is_object() {
            *target = serde_json::json!({});
        }
        let target = target.as_object_mut().unwrap();
        for (key, value) in patch {
            if value.is_null() {
                target.remove(key);
            } else {
                merge_patch(target.entry(key).or_insert(Value::Null), value);
            }
        }
    } else {
        *target = patch.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_preserves_siblings_and_replaces_arrays() {
        let mut data =
            serde_json::json!({"items":{"a":{"status":"open","owner":"A"},"b":2},"list":[1,2]});
        merge_patch(
            &mut data,
            &serde_json::json!({"items":{"a":{"status":"done","owner":null}},"list":[3]}),
        );
        assert_eq!(
            data,
            serde_json::json!({"items":{"a":{"status":"done"},"b":2},"list":[3]})
        );
    }
    #[test]
    fn reject_renderer_reserved_data_before_publication() {
        for value in [
            serde_json::json!({"deploy":{"steps":["build","test"]}}),
            serde_json::json!({"row":{"type":"element"}}),
            serde_json::json!({"row":{"k":"Comp"}}),
            serde_json::json!({"row":{"__reactive":true}}),
            serde_json::json!({"constructor":{"name":"row"}}),
            serde_json::json!({"nested":{"__proto__":null}}),
            serde_json::json!({"nested":{"prototype":1}}),
        ] {
            assert!(validate_data(&value)
                .unwrap_err()
                .to_string()
                .contains("reserved"));
        }
        assert!(validate_data(
            &serde_json::json!({"steps":"build", "k":"task", "__reactive":false})
        )
        .is_ok());
    }

    #[test]
    fn reject_unbounded_and_nonobject_data() {
        assert!(validate_data(&Value::Null).is_err());
        let mut data = serde_json::json!({});
        for _ in 0..34 {
            data = serde_json::json!({"nested":data});
        }
        assert!(validate_data(&data).is_err());
        assert!(validate_data(&serde_json::json!({"items":vec![0;20_001]})).is_err());
    }
}
