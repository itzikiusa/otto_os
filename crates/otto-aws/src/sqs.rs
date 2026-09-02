//! SQS — list / attributes / peek (View), send / delete-message / purge /
//! redrive (Edit) (§2.3).

use otto_core::{Error, Result};
use otto_state::AwsAccountRow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accounts::AwsService;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SqsQueue {
    pub url: String,
    pub name: String,
    pub fifo: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueuesResp {
    pub queues: Vec<SqsQueue>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AttributesResp {
    pub attributes: serde_json::Map<String, Value>,
    pub approx_messages: u64,
    pub approx_not_visible: u64,
    pub approx_delayed: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dlq_target_arn: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SqsMessage {
    pub message_id: String,
    pub receipt_handle: String,
    pub body: String,
    pub attributes: Value,
    pub message_attributes: Value,
    pub md5: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PeekResp {
    pub messages: Vec<SqsMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueuesQuery {
    pub prefix: Option<String>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UrlQuery {
    pub url: String,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PeekReq {
    pub url: String,
    pub max: Option<u8>,
    pub visibility_timeout: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendReq {
    pub url: String,
    pub body: String,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<String>,
    pub dedup_id: Option<String>,
    pub message_attributes: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendResp {
    pub message_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteMessageReq {
    pub url: String,
    pub receipt_handle: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PurgeReq {
    pub url: String,
    pub confirm_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedriveReq {
    pub source_arn: String,
    pub destination_arn: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RedriveResp {
    pub task_handle: String,
}

/// `?region=` for the POST routes.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RegionQuery {
    pub region: Option<String>,
}

// ---------------------------------------------------------------------------
// Pure
// ---------------------------------------------------------------------------

pub fn queue_name(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

pub fn normalize_queues(v: &Value) -> Vec<SqsQueue> {
    v.get("QueueUrls")
        .and_then(|u| u.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|u| u.as_str())
                .map(|url| {
                    let name = queue_name(url);
                    SqsQueue {
                        url: url.to_string(),
                        fifo: name.ends_with(".fifo"),
                        name,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn attr_u64(m: &serde_json::Map<String, Value>, k: &str) -> u64 {
    m.get(k)
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

pub fn normalize_attributes(v: &Value) -> AttributesResp {
    let attributes = v
        .get("Attributes")
        .and_then(|a| a.as_object())
        .cloned()
        .unwrap_or_default();
    let dlq_target_arn = attributes
        .get("RedrivePolicy")
        .and_then(|r| r.as_str())
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|p| {
            p.get("deadLetterTargetArn")
                .and_then(|a| a.as_str())
                .map(str::to_string)
        });
    AttributesResp {
        approx_messages: attr_u64(&attributes, "ApproximateNumberOfMessages"),
        approx_not_visible: attr_u64(&attributes, "ApproximateNumberOfMessagesNotVisible"),
        approx_delayed: attr_u64(&attributes, "ApproximateNumberOfMessagesDelayed"),
        dlq_target_arn,
        attributes,
    }
}

pub fn normalize_messages(v: &Value) -> Vec<SqsMessage> {
    v.get("Messages")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let s = |k: &str| m.get(k).and_then(|x| x.as_str()).map(str::to_string);
                    Some(SqsMessage {
                        message_id: s("MessageId")?,
                        receipt_handle: s("ReceiptHandle").unwrap_or_default(),
                        body: s("Body").unwrap_or_default(),
                        attributes: m
                            .get("Attributes")
                            .cloned()
                            .unwrap_or(Value::Object(Default::default())),
                        message_attributes: m
                            .get("MessageAttributes")
                            .cloned()
                            .unwrap_or(Value::Object(Default::default())),
                        md5: s("MD5OfBody"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn validate_url(url: &str) -> Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://"))
        || url.len() > 2048
        || url.chars().any(char::is_whitespace)
    {
        return Err(Error::Invalid("queue url must be an http(s) URL".into()));
    }
    Ok(())
}

pub fn validate_arn(arn: &str) -> Result<()> {
    if !arn.starts_with("arn:") || arn.len() > 2048 || arn.chars().any(char::is_whitespace) {
        return Err(Error::Invalid(format!("invalid ARN '{arn}'")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

pub async fn list_queues(
    svc: &AwsService,
    a: &AwsAccountRow,
    q: &QueuesQuery,
) -> Result<QueuesResp> {
    let mut args = vec!["sqs", "list-queues"];
    let prefix = q.prefix.clone().unwrap_or_default();
    if !prefix.is_empty() {
        args.extend(["--queue-name-prefix", prefix.as_str()]);
    }
    let v = svc.run_json(a, q.region.as_deref(), &args).await?;
    Ok(QueuesResp {
        queues: normalize_queues(&v),
    })
}

pub async fn attributes(
    svc: &AwsService,
    a: &AwsAccountRow,
    url: &str,
    region: Option<&str>,
) -> Result<AttributesResp> {
    validate_url(url)?;
    let v = svc
        .run_json(
            a,
            region,
            &[
                "sqs",
                "get-queue-attributes",
                "--queue-url",
                url,
                "--attribute-names",
                "All",
            ],
        )
        .await?;
    Ok(normalize_attributes(&v))
}

pub async fn peek(
    svc: &AwsService,
    a: &AwsAccountRow,
    req: &PeekReq,
    region: Option<&str>,
) -> Result<PeekResp> {
    validate_url(&req.url)?;
    let max = req.max.unwrap_or(10).clamp(1, 10).to_string();
    let vis = req.visibility_timeout.unwrap_or(0).min(43200).to_string();
    let v = svc
        .run_json(
            a,
            region,
            &[
                "sqs",
                "receive-message",
                "--queue-url",
                &req.url,
                "--max-number-of-messages",
                &max,
                "--visibility-timeout",
                &vis,
                "--wait-time-seconds",
                "1",
                "--attribute-names",
                "All",
                "--message-attribute-names",
                "All",
            ],
        )
        .await?;
    Ok(PeekResp {
        messages: normalize_messages(&v),
    })
}

pub async fn send(
    svc: &AwsService,
    a: &AwsAccountRow,
    req: &SendReq,
    region: Option<&str>,
) -> Result<SendResp> {
    validate_url(&req.url)?;
    if req.body.is_empty() || req.body.len() > 256 * 1024 {
        return Err(Error::Invalid(
            "message body must be 1 byte .. 256 KiB".into(),
        ));
    }
    let delay;
    let attrs;
    let mut args: Vec<&str> = vec![
        "sqs",
        "send-message",
        "--queue-url",
        &req.url,
        "--message-body",
        &req.body,
    ];
    if let Some(d) = req.delay_seconds {
        delay = d.min(900).to_string();
        args.extend(["--delay-seconds", delay.as_str()]);
    }
    if let Some(g) = req.group_id.as_deref().filter(|s| !s.is_empty()) {
        args.extend(["--message-group-id", g]);
    }
    if let Some(d) = req.dedup_id.as_deref().filter(|s| !s.is_empty()) {
        args.extend(["--message-deduplication-id", d]);
    }
    if let Some(ma) = &req.message_attributes {
        if ma.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
            attrs = ma.to_string();
            args.extend(["--message-attributes", attrs.as_str()]);
        }
    }
    let v = svc.run_json(a, region, &args).await?;
    let message_id = v
        .get("MessageId")
        .and_then(|m| m.as_str())
        .ok_or_else(|| Error::Upstream("send-message returned no MessageId".into()))?
        .to_string();
    Ok(SendResp { message_id })
}

pub async fn delete_message(
    svc: &AwsService,
    a: &AwsAccountRow,
    req: &DeleteMessageReq,
    region: Option<&str>,
) -> Result<()> {
    validate_url(&req.url)?;
    if req.receipt_handle.is_empty() {
        return Err(Error::Invalid("receipt_handle is required".into()));
    }
    svc.run(
        a,
        region,
        &[
            "sqs",
            "delete-message",
            "--queue-url",
            &req.url,
            "--receipt-handle",
            &req.receipt_handle,
        ],
    )
    .await?;
    Ok(())
}

pub async fn purge(
    svc: &AwsService,
    a: &AwsAccountRow,
    req: &PurgeReq,
    region: Option<&str>,
) -> Result<()> {
    validate_url(&req.url)?;
    let name = queue_name(&req.url);
    if req.confirm_name != name {
        return Err(Error::Invalid(format!(
            "confirm_name must equal the queue name '{name}'"
        )));
    }
    svc.run(a, region, &["sqs", "purge-queue", "--queue-url", &req.url])
        .await?;
    Ok(())
}

pub async fn redrive(
    svc: &AwsService,
    a: &AwsAccountRow,
    req: &RedriveReq,
    region: Option<&str>,
) -> Result<RedriveResp> {
    validate_arn(&req.source_arn)?;
    let mut args = vec![
        "sqs",
        "start-message-move-task",
        "--source-arn",
        req.source_arn.as_str(),
    ];
    if let Some(d) = req.destination_arn.as_deref().filter(|s| !s.is_empty()) {
        validate_arn(d)?;
        args.extend(["--destination-arn", d]);
    }
    let v = svc.run_json(a, region, &args).await?;
    let task_handle = v
        .get("TaskHandle")
        .and_then(|t| t.as_str())
        .ok_or_else(|| Error::Upstream("start-message-move-task returned no TaskHandle".into()))?
        .to_string();
    Ok(RedriveResp { task_handle })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queues_normalize() {
        let v: Value = serde_json::from_str(
            r#"{"QueueUrls": ["https://sqs.eu-west-1.amazonaws.com/123456789012/orders", "https://sqs.eu-west-1.amazonaws.com/123456789012/orders-dlq.fifo"]}"#,
        )
        .unwrap();
        let q = normalize_queues(&v);
        assert_eq!(q[0].name, "orders");
        assert!(!q[0].fifo);
        assert_eq!(q[1].name, "orders-dlq.fifo");
        assert!(q[1].fifo);
        // Empty account: the CLI prints nothing / `{}`.
        assert!(normalize_queues(&Value::Null).is_empty());
    }

    #[test]
    fn attributes_normalize_with_redrive_policy() {
        let v: Value = serde_json::from_str(
            r#"{"Attributes": {"QueueArn": "arn:aws:sqs:eu-west-1:123456789012:orders", "ApproximateNumberOfMessages": "12", "ApproximateNumberOfMessagesNotVisible": "3", "ApproximateNumberOfMessagesDelayed": "0", "RedrivePolicy": "{\"deadLetterTargetArn\":\"arn:aws:sqs:eu-west-1:123456789012:orders-dlq\",\"maxReceiveCount\":5}", "VisibilityTimeout": "30"}}"#,
        )
        .unwrap();
        let a = normalize_attributes(&v);
        assert_eq!(a.approx_messages, 12);
        assert_eq!(a.approx_not_visible, 3);
        assert_eq!(a.approx_delayed, 0);
        assert_eq!(
            a.dlq_target_arn.as_deref(),
            Some("arn:aws:sqs:eu-west-1:123456789012:orders-dlq")
        );
        assert_eq!(a.attributes["VisibilityTimeout"], "30");
    }

    #[test]
    fn messages_normalize() {
        let v: Value = serde_json::from_str(
            r#"{"Messages": [{"MessageId": "m1", "ReceiptHandle": "rh", "MD5OfBody": "d41d8", "Body": "{\"a\":1}", "Attributes": {"SentTimestamp": "1700000000000"}, "MessageAttributes": {"k": {"StringValue": "v", "DataType": "String"}}}]}"#,
        )
        .unwrap();
        let m = normalize_messages(&v);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].message_id, "m1");
        assert_eq!(m[0].body, "{\"a\":1}");
        assert_eq!(m[0].attributes["SentTimestamp"], "1700000000000");
        assert_eq!(m[0].message_attributes["k"]["StringValue"], "v");
        assert!(normalize_messages(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn validators() {
        assert!(validate_url("https://sqs.us-east-1.amazonaws.com/1/q").is_ok());
        assert!(validate_url("sqs.us-east-1.amazonaws.com/1/q").is_err());
        assert!(validate_url("https://x y").is_err());
        assert!(validate_arn("arn:aws:sqs:us-east-1:1:q").is_ok());
        assert!(validate_arn("q").is_err());
        assert_eq!(queue_name("https://sqs.us-east-1.amazonaws.com/1/q/"), "q");
    }
}
