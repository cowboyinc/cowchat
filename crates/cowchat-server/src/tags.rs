//! Stamps `metadata.tags` on accepted messages: validates author tags and marks
//! them `author`, whatever source the client claimed.

use cowchat_core::tags::{MessageTag, MAX_TAGS_PER_MESSAGE, TAG_SOURCE_AUTHOR};
use serde_json::Value;

pub(crate) fn stamp_tags(metadata: &mut Value, encrypted: bool) -> Result<(), String> {
    let tags = author_tags(metadata)?;
    if encrypted && !tags.is_empty() {
        return Err("tags are not available in end-to-end encrypted rooms".to_owned());
    }
    if let Some(fields) = metadata.as_object_mut() {
        fields.remove("tags");
    }
    if !tags.is_empty() {
        if !metadata.is_object() {
            *metadata = serde_json::json!({});
        }
        metadata["tags"] = serde_json::json!(tags);
    }
    Ok(())
}

fn author_tags(metadata: &Value) -> Result<Vec<MessageTag>, String> {
    let Some(value) = metadata.get("tags") else {
        return Ok(Vec::new());
    };
    let supplied: Vec<MessageTag> = serde_json::from_value(value.clone())
        .map_err(|_| "metadata.tags must be a list of objects with a key".to_owned())?;
    if supplied.len() > MAX_TAGS_PER_MESSAGE {
        return Err(format!(
            "a message can carry at most {MAX_TAGS_PER_MESSAGE} tags"
        ));
    }
    let mut tags = Vec::with_capacity(supplied.len());
    for mut tag in supplied {
        tag.validate()?;
        tag.source = Some(TAG_SOURCE_AUTHOR.to_owned());
        push_unique(&mut tags, tag);
    }
    Ok(tags)
}

fn push_unique(tags: &mut Vec<MessageTag>, tag: MessageTag) {
    if !tags
        .iter()
        .any(|existing| existing.key == tag.key && existing.value == tag.value)
    {
        tags.push(tag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cowchat_core::tags::{metadata_tags, MAX_TAGS_PER_MESSAGE};
    use serde_json::{json, Value};

    fn stamped(mut metadata: Value) -> Value {
        stamp_tags(&mut metadata, false).unwrap();
        metadata
    }

    #[test]
    fn author_tags_are_marked_author_even_when_the_client_claims_otherwise() {
        let metadata = stamped(
            json!({"tags":[{"key":"topic","value":"router","source":"extractor","extra":1}]}),
        );
        assert_eq!(
            metadata["tags"],
            json!([{"key":"topic","value":"router","source":"author"}])
        );
    }

    #[test]
    fn invalid_author_tags_are_rejected() {
        for tags in [
            json!("topic"),
            json!(["topic"]),
            json!([{"key":"Bad Key"}]),
            json!([{"value":"no key"}]),
        ] {
            assert!(stamp_tags(&mut json!({"tags":tags}), false).is_err());
        }
        let too_many: Vec<Value> = (0..=MAX_TAGS_PER_MESSAGE)
            .map(|index| json!({"key":format!("k{index}")}))
            .collect();
        assert!(stamp_tags(&mut json!({"tags":too_many}), false).is_err());
    }

    #[test]
    fn repeated_author_tags_collapse() {
        let metadata = stamped(
            json!({"tags":[{"key":"topic","value":"a"},{"key":"topic","value":"a"},{"key":"topic","value":"b"}]}),
        );
        assert_eq!(metadata_tags(&metadata).len(), 2);
    }

    #[test]
    fn a_message_without_tags_gets_no_tags_field() {
        assert_eq!(stamped(json!({"kind":"fyi"})), json!({"kind":"fyi"}));
        assert_eq!(stamped(json!({"tags":[]})), json!({}));
    }

    #[test]
    fn encrypted_rooms_refuse_author_tags_because_metadata_is_plaintext() {
        let error = stamp_tags(&mut json!({"tags":[{"key":"topic"}]}), true).unwrap_err();
        assert!(error.contains("encrypted"));
    }
}
