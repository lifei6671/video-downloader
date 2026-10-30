//! R2 transport using the official S3 SDK; errors never include request credentials.
use super::{
    model::Space,
    store::{
        Condition, Object, ObjectInfo, ObjectStore, ObjectStream, Page, Part, StoreFuture, Write,
    },
};
use anyhow::{anyhow, bail, Context, Result};
use aws_sdk_s3::{
    config::{
        BehaviorVersion, Credentials, Region, RequestChecksumCalculation,
        ResponseChecksumValidation,
    },
    error::{ProvideErrorMetadata, SdkError},
    primitives::{ByteStream, SdkBody},
    types::{CompletedMultipartUpload, CompletedPart},
};
use http_body_util::BodyExt;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::io::AsyncReadExt;

const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;

pub struct S3Store {
    client: aws_sdk_s3::Client,
    bucket: String,
}

pub fn validate_target(endpoint: &str, bucket: &str, prefix: &str) -> Result<()> {
    let url = url::Url::parse(endpoint).context("BACKUP_INVALID_ENDPOINT")?;
    let host = url.host_str().context("BACKUP_INVALID_ENDPOINT")?;
    let account = host
        .strip_suffix(".r2.cloudflarestorage.com")
        .context("BACKUP_INVALID_ENDPOINT")?;
    if url.scheme() != "https"
        || account.len() != 32
        || !account.bytes().all(|b| b.is_ascii_hexdigit())
        || url.port().is_some_and(|p| p != 443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        bail!("BACKUP_INVALID_ENDPOINT");
    }
    if bucket.len() < 3
        || bucket.len() > 63
        || !bucket
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || bucket.starts_with('-')
        || bucket.ends_with('-')
    {
        bail!("BACKUP_INVALID_BUCKET");
    }
    let inner = prefix.strip_suffix('/').context("BACKUP_INVALID_PREFIX")?;
    if inner.is_empty()
        || inner.len() > 512
        || inner
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || inner
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && !matches!(b, b'/' | b'-' | b'_'))
    {
        bail!("BACKUP_INVALID_PREFIX");
    }
    Ok(())
}

impl S3Store {
    pub fn new(space: &Space, access_key: &str, secret: &str) -> Result<Self> {
        validate_target(&space.endpoint, &space.bucket, &space.prefix)?;
        if access_key.is_empty() || secret.is_empty() {
            bail!("BACKUP_CREDENTIALS_MISSING");
        }
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("auto"))
            .endpoint_url(&space.endpoint)
            .force_path_style(true)
            .credentials_provider(Credentials::new(
                access_key,
                secret,
                None,
                None,
                "yingqu-r2",
            ))
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            .retry_config(aws_sdk_s3::config::retry::RetryConfig::standard().with_max_attempts(1))
            .timeout_config(
                aws_sdk_s3::config::timeout::TimeoutConfig::builder()
                    .connect_timeout(Duration::from_secs(15))
                    .read_timeout(Duration::from_secs(30))
                    .operation_timeout(Duration::from_secs(180))
                    .build(),
            )
            .build();
        Ok(Self {
            client: aws_sdk_s3::Client::from_conf(config),
            bucket: space.bucket.clone(),
        })
    }
}

fn error<E: ProvideErrorMetadata>(error: SdkError<E>) -> anyhow::Error {
    let code = error.as_service_error().and_then(|e| e.code());
    let status = error.raw_response().map(|r| r.status().as_u16());
    anyhow!(match (code, status) {
        (Some("InvalidAccessKeyId"), _) => "BACKUP_INVALID_ACCESS_KEY",
        (Some("SignatureDoesNotMatch"), _) => "BACKUP_SIGNATURE_MISMATCH",
        (Some("NoSuchKey"), _) => "BACKUP_OBJECT_NOT_FOUND",
        (Some("NoSuchBucket"), _) => "BACKUP_BUCKET_NOT_FOUND",
        (Some("NoSuchUpload"), _) => "BACKUP_UPLOAD_EXPIRED",
        (_, Some(412)) => "BACKUP_PRECONDITION_FAILED",
        (_, Some(403)) => "BACKUP_ACCESS_DENIED",
        (_, Some(429)) | (Some("SlowDown"), _) => "BACKUP_RATE_LIMITED",
        (_, Some(500..=599)) => "BACKUP_SERVICE_UNAVAILABLE",
        (_, Some(404)) => "BACKUP_OBJECT_NOT_FOUND",
        (_, Some(_)) => "BACKUP_REQUEST_REJECTED",
        _ => "BACKUP_NETWORK_ERROR",
    })
}

impl ObjectStore for S3Store {
    fn uploads(&self, key: &str) -> StoreFuture<'_, Vec<String>> {
        let key = key.to_owned();
        Box::pin(async move {
            let mut ids = Vec::new();
            let mut marker = None;
            let mut upload_marker = None;
            loop {
                let page = self
                    .client
                    .list_multipart_uploads()
                    .bucket(&self.bucket)
                    .prefix(&key)
                    .set_key_marker(marker.clone())
                    .set_upload_id_marker(upload_marker.clone())
                    .send()
                    .await
                    .map_err(error)?;
                for item in page
                    .uploads()
                    .iter()
                    .filter(|u| u.key() == Some(key.as_str()))
                {
                    ids.push(
                        item.upload_id()
                            .context("BACKUP_INVALID_UPLOAD")?
                            .to_owned(),
                    );
                }
                if page.is_truncated() != Some(true) {
                    break;
                }
                let next = page.next_key_marker().map(str::to_owned);
                let next_upload = page.next_upload_id_marker().map(str::to_owned);
                if next.is_none() || (next == marker && next_upload == upload_marker) {
                    bail!("BACKUP_INVALID_PAGE");
                }
                marker = next;
                upload_marker = next_upload;
            }
            Ok(ids)
        })
    }

    fn get(&self, key: &str) -> StoreFuture<'_, Option<Object>> {
        let key = key.to_owned();
        Box::pin(async move {
            let result = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(key)
                .send()
                .await
                .map_err(error);
            let result = match result {
                Ok(v) => v,
                Err(e) if e.to_string() == "BACKUP_OBJECT_NOT_FOUND" => return Ok(None),
                Err(e) => return Err(e),
            };
            if result
                .content_length()
                .is_some_and(|v| v < 0 || v as u64 > MAX_METADATA_BYTES)
            {
                bail!("BACKUP_METADATA_TOO_LARGE");
            }
            let etag = result.e_tag().context("BACKUP_ETAG_MISSING")?.to_owned();
            let mut body = Vec::new();
            result
                .body
                .into_async_read()
                .take(MAX_METADATA_BYTES + 1)
                .read_to_end(&mut body)
                .await
                .context("BACKUP_NETWORK_ERROR")?;
            if body.len() as u64 > MAX_METADATA_BYTES {
                bail!("BACKUP_METADATA_TOO_LARGE");
            }
            Ok(Some(Object { body, etag }))
        })
    }
    fn head(&self, key: &str) -> StoreFuture<'_, Option<ObjectInfo>> {
        let key = key.to_owned();
        Box::pin(async move {
            let result = self
                .client
                .head_object()
                .bucket(&self.bucket)
                .key(&key)
                .send()
                .await
                .map_err(error);
            let v = match result {
                Ok(v) => v,
                Err(e) if e.to_string() == "BACKUP_OBJECT_NOT_FOUND" => return Ok(None),
                Err(e) => return Err(e),
            };
            Ok(Some(ObjectInfo {
                key,
                etag: v.e_tag().context("BACKUP_ETAG_MISSING")?.into(),
                bytes: u64::try_from(v.content_length().context("BACKUP_INVALID_SIZE")?)?,
                sha256: v.metadata().and_then(|m| m.get("sha256")).cloned(),
            }))
        })
    }
    fn list(&self, prefix: &str, token: Option<String>) -> StoreFuture<'_, Page> {
        let prefix = prefix.to_owned();
        Box::pin(async move {
            let page = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix)
                .set_continuation_token(token)
                .send()
                .await
                .map_err(error)?;
            let mut objects = Vec::new();
            for v in page.contents() {
                objects.push(ObjectInfo {
                    key: v.key().context("BACKUP_INVALID_KEY")?.into(),
                    etag: v.e_tag().context("BACKUP_ETAG_MISSING")?.into(),
                    bytes: u64::try_from(v.size().context("BACKUP_INVALID_SIZE")?)?,
                    sha256: None,
                });
            }
            let next = page.next_continuation_token().map(str::to_owned);
            if page.is_truncated() == Some(true) && next.is_none() {
                bail!("BACKUP_INVALID_PAGE");
            }
            Ok(Page { objects, next })
        })
    }
    fn stream(
        &self,
        key: &str,
        offset: u64,
        etag: Option<String>,
    ) -> StoreFuture<'_, ObjectStream> {
        let key = key.to_owned();
        Box::pin(async move {
            let mut request = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(&key)
                .set_if_match(etag);
            if offset > 0 {
                request = request.range(format!("bytes={offset}-"));
            }
            let v = request.send().await.map_err(error)?;
            let info = ObjectInfo {
                key,
                etag: v.e_tag().context("BACKUP_ETAG_MISSING")?.into(),
                bytes: u64::try_from(v.content_length().context("BACKUP_INVALID_SIZE")?)?,
                sha256: v.metadata().and_then(|m| m.get("sha256")).cloned(),
            };
            let content_range = v.content_range().map(str::to_owned);
            Ok(ObjectStream {
                info,
                content_range,
                reader: Box::new(v.body.into_async_read()),
            })
        })
    }
    fn parts(&self, key: &str, upload_id: &str) -> StoreFuture<'_, Vec<Part>> {
        let (key, upload_id) = (key.to_owned(), upload_id.to_owned());
        Box::pin(async move {
            let mut marker = None;
            let mut parts = Vec::new();
            loop {
                let page = self
                    .client
                    .list_parts()
                    .bucket(&self.bucket)
                    .key(&key)
                    .upload_id(&upload_id)
                    .set_part_number_marker(marker.clone())
                    .send()
                    .await
                    .map_err(error)?;
                for p in page.parts() {
                    parts.push(Part {
                        number: u32::try_from(p.part_number().context("BACKUP_INVALID_PART")?)?,
                        etag: p.e_tag().context("BACKUP_ETAG_MISSING")?.into(),
                        bytes: u64::try_from(p.size().context("BACKUP_INVALID_SIZE")?)?,
                    });
                }
                if page.is_truncated() != Some(true) {
                    break;
                }
                let next = page
                    .next_part_number_marker()
                    .context("BACKUP_INVALID_PAGE")?
                    .to_owned();
                if marker.as_deref() == Some(&next) {
                    bail!("BACKUP_INVALID_PAGE");
                }
                marker = Some(next);
            }
            Ok(parts)
        })
    }
    fn write(&self, operation: Write) -> StoreFuture<'_, String> {
        Box::pin(async move {
            match operation {
                Write::Put {
                    key,
                    body,
                    condition,
                } => {
                    let mut request = self
                        .client
                        .put_object()
                        .bucket(&self.bucket)
                        .key(key)
                        .body(ByteStream::from(body));
                    request = match condition {
                        Condition::Absent => request.if_none_match("*"),
                        Condition::Match(e) => request.if_match(e),
                    };
                    Ok(request
                        .send()
                        .await
                        .map_err(error)?
                        .e_tag()
                        .context("BACKUP_ETAG_MISSING")?
                        .into())
                }
                Write::PutFile {
                    key,
                    path,
                    bytes,
                    sha256,
                    progress,
                } => {
                    let body = ByteStream::from_path(path)
                        .await
                        .context("BACKUP_SOURCE_UNAVAILABLE")?;
                    let body = observed_body(body, progress);
                    Ok(self
                        .client
                        .put_object()
                        .bucket(&self.bucket)
                        .key(key)
                        .if_none_match("*")
                        .content_length(i64::try_from(bytes)?)
                        .metadata("sha256", sha256)
                        .body(body)
                        .send()
                        .await
                        .map_err(error)?
                        .e_tag()
                        .context("BACKUP_ETAG_MISSING")?
                        .into())
                }
                Write::Delete { key } => {
                    self.client
                        .delete_object()
                        .bucket(&self.bucket)
                        .key(key)
                        .send()
                        .await
                        .map_err(error)?;
                    Ok(String::new())
                }
                Write::CreateMultipart { key, sha256 } => Ok(self
                    .client
                    .create_multipart_upload()
                    .metadata("sha256", sha256)
                    .bucket(&self.bucket)
                    .key(key)
                    .send()
                    .await
                    .map_err(error)?
                    .upload_id()
                    .context("BACKUP_UPLOAD_ID_MISSING")?
                    .into()),
                Write::UploadPart {
                    key,
                    upload_id,
                    part,
                    body,
                } => Ok(self
                    .client
                    .upload_part()
                    .bucket(&self.bucket)
                    .key(key)
                    .upload_id(upload_id)
                    .part_number(i32::try_from(part)?)
                    .body(ByteStream::from(body))
                    .send()
                    .await
                    .map_err(error)?
                    .e_tag()
                    .context("BACKUP_ETAG_MISSING")?
                    .into()),
                Write::UploadFilePart {
                    key,
                    upload_id,
                    part,
                    path,
                    offset,
                    bytes,
                    progress,
                } => {
                    let body = ByteStream::read_from()
                        .path(path)
                        .offset(offset)
                        .length(aws_sdk_s3::primitives::Length::Exact(bytes))
                        .build()
                        .await
                        .context("BACKUP_SOURCE_UNAVAILABLE")?;
                    let body = observed_body(body, progress);
                    Ok(self
                        .client
                        .upload_part()
                        .bucket(&self.bucket)
                        .key(key)
                        .upload_id(upload_id)
                        .part_number(i32::try_from(part)?)
                        .content_length(i64::try_from(bytes)?)
                        .body(body)
                        .send()
                        .await
                        .map_err(error)?
                        .e_tag()
                        .context("BACKUP_ETAG_MISSING")?
                        .into())
                }
                Write::CompleteMultipart {
                    key,
                    upload_id,
                    parts,
                } => {
                    let parts = parts
                        .into_iter()
                        .map(|(n, e)| {
                            Ok(CompletedPart::builder()
                                .part_number(i32::try_from(n)?)
                                .e_tag(e)
                                .build())
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(self
                        .client
                        .complete_multipart_upload()
                        .bucket(&self.bucket)
                        .key(key)
                        .upload_id(upload_id)
                        .multipart_upload(
                            CompletedMultipartUpload::builder()
                                .set_parts(Some(parts))
                                .build(),
                        )
                        .send()
                        .await
                        .map_err(error)?
                        .e_tag()
                        .context("BACKUP_ETAG_MISSING")?
                        .into())
                }
                Write::AbortMultipart { key, upload_id } => {
                    self.client
                        .abort_multipart_upload()
                        .bucket(&self.bucket)
                        .key(key)
                        .upload_id(upload_id)
                        .send()
                        .await
                        .map_err(error)?;
                    Ok(String::new())
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targets_cannot_redirect_credentials_to_an_arbitrary_host_or_prefix() {
        let valid = "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com";
        assert!(validate_target(valid, "test-bucket", "yingqu/v1/").is_ok());
        for endpoint in [
            "http://127.0.0.1",
            "https://example.com",
            "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com.evil.test",
            "https://user:pass@0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com",
        ] {
            assert!(validate_target(endpoint, "test-bucket", "yingqu/v1/").is_err());
        }
        for prefix in ["", "/", "yingqu", "yingqu/../", "yingqu//v1/"] {
            assert!(validate_target(valid, "test-bucket", prefix).is_err());
        }
    }
}

fn observed_body(body: ByteStream, progress: Option<Arc<AtomicU64>>) -> ByteStream {
    let Some(progress) = progress else {
        return body;
    };
    ByteStream::new(body.into_inner().map_preserve_contents(move |body| {
        let progress = progress.clone();
        let mut count = 0u64;
        SdkBody::from_body_1_x(body.map_frame(move |frame| {
            if let Some(data) = frame.data_ref() {
                count += data.len() as u64;
                progress.store(count, Ordering::Relaxed);
            }
            frame
        }))
    }))
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    #[tokio::test]
    async fn http_body_progress_counts_consumed_bytes_without_changing_content() {
        let progress = Arc::new(AtomicU64::new(0));
        let body = observed_body(ByteStream::from(vec![7u8; 1234]), Some(progress.clone()));
        assert_eq!(progress.load(Ordering::Relaxed), 0);
        let bytes = body.collect().await.unwrap().into_bytes();
        assert_eq!(bytes.as_ref(), vec![7u8; 1234]);
        assert_eq!(progress.load(Ordering::Relaxed), 1234);
    }
}
