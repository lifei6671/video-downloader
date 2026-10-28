use super::Adapter;
use crate::{core_ipc::MAX_REQUEST_BYTES, mcp_read};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use rmcp::model::{
    ErrorData, ListResourceTemplatesResult, PaginatedRequestParams, ReadResourceResult,
    ResourceContents, ResourceTemplate,
};
use serde_json::{json, Value};
use std::collections::HashMap;

// RFC 3986 unreserved characters are the only unescaped logical-ID bytes.
const ID_ESCAPE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub(super) fn check_cursor(request: Option<PaginatedRequestParams>) -> Result<(), ErrorData> {
    if request.is_some_and(|request| request.cursor.is_some()) {
        return Err(ErrorData::invalid_params("INVALID_RESOURCE_CURSOR", None));
    }
    Ok(())
}

pub(super) fn templates() -> ListResourceTemplatesResult {
    let resource_templates = [
        ("yingqu://assets/{asset_id}{?document_cursor,limit}", "asset", "Asset metadata and paged summaries. Same scope as get_asset."),
        ("yingqu://assets/{asset_id}/documents/{document_id}{?version,offset,length}", "document", "Unicode scalar slice of an immutable document. Same scope as read_transcript."),
        ("yingqu://assets/{asset_id}/notes/{note_id}{?revision,offset,length,citation_offset,citation_limit}", "note", "Revision-pinned note slice and paged citations. Same scope as read_note."),
    ].into_iter().map(|(uri, name, description)| ResourceTemplate::new(uri,name).with_description(description).with_mime_type("application/json")).collect();
    ListResourceTemplatesResult {
        resource_templates,
        ..Default::default()
    }
}

#[derive(Debug)]
enum Read {
    Asset(mcp_read::AssetInput),
    Document(mcp_read::TranscriptInput),
    Note(mcp_read::NoteInput),
}
fn invalid() -> ErrorData {
    ErrorData::invalid_params("INVALID_RESOURCE_URI", None)
}

fn decode(value: &str) -> Result<String, ErrorData> {
    // percent_decode deliberately leaves malformed '%' unchanged. Reject that
    // before decoding, and reject invalid UTF-8 rather than replacing it.
    let bytes = value.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && (index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit())
        {
            return Err(invalid());
        }
    }
    percent_decode_str(value)
        .decode_utf8()
        .map(|value| value.into_owned())
        .map_err(|_| invalid())
}
fn segment(value: &str) -> Result<String, ErrorData> {
    let value_decoded = decode(value)?;
    if value_decoded.is_empty()
        || utf8_percent_encode(&value_decoded, ID_ESCAPE).to_string() != value
    {
        return Err(invalid());
    }
    Ok(value_decoded)
}
fn parameters(raw: Option<&str>, allowed: &[&str]) -> Result<HashMap<String, Value>, ErrorData> {
    let mut values = HashMap::new();
    if let Some(raw) = raw {
        if raw.is_empty() {
            return Err(invalid());
        }
        for pair in raw.split('&') {
            let (key, value) = pair.split_once('=').ok_or_else(invalid)?;
            let key = decode(&key.replace('+', " "))?;
            let value = decode(&value.replace('+', " "))?;
            if !allowed.contains(&key.as_str()) || values.contains_key(&key) || value.is_empty() {
                return Err(invalid());
            }
            let value = if key == "document_cursor" {
                Value::String(value)
            } else {
                if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(invalid());
                }
                Value::from(value.parse::<u64>().map_err(|_| invalid())?)
            };
            values.insert(key, value);
        }
    }
    Ok(values)
}
fn parse(uri: &str) -> Result<Read, ErrorData> {
    if uri.len() > MAX_REQUEST_BYTES || !uri.starts_with("yingqu://assets/") {
        return Err(invalid());
    }
    let url = url::Url::parse(uri).map_err(|_| invalid())?;
    // Deny implicit normalization, credentials, ports and fragments. Encoded ID
    // separators remain logical ID text; no URI ever becomes a filesystem path.
    if url.as_str() != uri
        || url.host_str() != Some("assets")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let path: Vec<_> = url
        .path()
        .strip_prefix('/')
        .ok_or_else(invalid)?
        .split('/')
        .collect();
    let (method, mut params) = match path.as_slice() {
        [_] => (
            "asset",
            parameters(url.query(), &["document_cursor", "limit"])?,
        ),
        [_, "documents", _] => (
            "document",
            parameters(url.query(), &["version", "offset", "length"])?,
        ),
        [_, "notes", _] => (
            "note",
            parameters(
                url.query(),
                &[
                    "revision",
                    "offset",
                    "length",
                    "citation_offset",
                    "citation_limit",
                ],
            )?,
        ),
        _ => return Err(invalid()),
    };
    params.insert("asset_id".into(), Value::String(segment(path[0])?));
    match method {
        "asset" => serde_json::from_value(json!(params))
            .map(Read::Asset)
            .map_err(|_| invalid()),
        "document" => {
            params.insert("document_id".into(), Value::String(segment(path[2])?));
            serde_json::from_value(json!(params))
                .map(Read::Document)
                .map_err(|_| invalid())
        }
        "note" => {
            params.insert("note_id".into(), Value::String(segment(path[2])?));
            serde_json::from_value(json!(params))
                .map(Read::Note)
                .map_err(|_| invalid())
        }
        _ => unreachable!("route was selected above"),
    }
}

impl Adapter {
    pub(super) async fn read_public_resource(
        &self,
        uri: &str,
    ) -> Result<ReadResourceResult, ErrorData> {
        let text = self.resource_text(uri).await.map_err(|error| {
            if matches!(
                error.message.as_ref(),
                "ASSET_NOT_FOUND" | "DOCUMENT_NOT_FOUND" | "NOTE_NOT_FOUND"
            ) {
                ErrorData::resource_not_found(error.message, None)
            } else {
                error
            }
        })?;
        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            text, uri,
        )
        .with_mime_type("application/json")]))
    }

    async fn resource_text(&self, uri: &str) -> Result<String, ErrorData> {
        match parse(uri)? {
            Read::Asset(input) => serde_json::to_string(
                &self
                    .read::<_, mcp_read::AssetDetail>("mcp_get_asset", input)
                    .await?
                    .0,
            ),
            Read::Document(input) => serde_json::to_string(
                &self
                    .read::<_, mcp_read::Transcript>("mcp_read_transcript", input)
                    .await?
                    .0,
            ),
            Read::Note(input) => serde_json::to_string(
                &self
                    .read::<_, mcp_read::Note>("mcp_read_note", input)
                    .await?
                    .0,
            ),
        }
        .map_err(|_| ErrorData::internal_error("IPC_PROTOCOL_ERROR", None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_and_uri_parameters_preserve_public_read_inputs() {
        assert_eq!(templates().resource_templates.len(), 3);
        let Read::Document(input) =
            parse("yingqu://assets/asset-a/documents/doc-a1?version=1&offset=2&length=3").unwrap()
        else {
            panic!("wrong route")
        };
        assert_eq!(
            serde_json::to_value(input).unwrap(),
            json!({"asset_id":"asset-a","document_id":"doc-a1","version":1,"offset":2,"length":3})
        );
        let Read::Note(input) = parse(
            "yingqu://assets/asset-a/notes/note-a?revision=2&citation_offset=20&citation_limit=1",
        )
        .unwrap() else {
            panic!("wrong route")
        };
        assert_eq!(input.asset_id.as_deref(), Some("asset-a"));
        assert_eq!(input.revision, Some(2));
        assert_eq!(input.length, 4000);
        let Read::Asset(input) =
            parse("yingqu://assets/%E4%B8%AD%E6%96%87%F0%9F%98%80?document_cursor=doc%2B1&limit=1")
                .unwrap()
        else {
            panic!("wrong route")
        };
        assert_eq!(input.asset_id, "中文😀");
        assert_eq!(input.document_cursor.as_deref(), Some("doc+1"));
        let Read::Asset(input) = parse("yingqu://assets/asset-a").unwrap() else {
            panic!("wrong route")
        };
        assert_eq!(input.limit, 20);
    }

    #[test]
    fn malformed_or_unbounded_resource_routes_never_become_reads() {
        for uri in [
            "file:///C:/private",
            "https://example.invalid",
            "yingqu://settings/private",
            "yingqu://assets/",
            "yingqu://assets/asset-a/files/file-a",
            "yingqu://assets/asset-a/documents/doc-a1/extra",
            "yingqu://assets/asset-a/../asset-b",
            "yingqu://assets/asset-a/%2e%2e/asset-b",
            "yingqu://assets/asset-a#secret",
            "yingqu://assets@remote/asset-a",
            "yingqu://assets:80/asset-a",
            "yingqu://assets/asset-a?path=C:/private",
            "yingqu://assets/asset-a?limit=1&limit=100",
            "yingqu://assets/asset-a?limit=18446744073709551616",
            "yingqu://assets/asset-a?limit=-1",
            "yingqu://assets/asset-a?limit=1.5",
            "yingqu://assets/asset-a?",
            "yingqu://assets/asset-a?document_cursor=%FF",
            "yingqu://assets/asset-a?document_cursor=%ZZ",
            "yingqu://assets/%FF",
            "yingqu://assets/%ZZ",
            "yingqu://assets/%61sset-a",
            "yingqu://assets/asset-a/notes/note-a?asset_id=asset-b",
            "yingqu://assets/asset-a/documents/doc-a1?length=all",
        ] {
            assert!(parse(uri).is_err(), "accepted {uri}");
        }
    }
}
