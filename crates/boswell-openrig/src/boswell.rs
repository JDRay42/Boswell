//! A small client for the Boswell gateway's HTTP API: just the routes the
//! hooks and the bridge use.

use crate::config::BoswellConfig;
use serde_json::{json, Value};
use std::time::Duration;

/// What can go wrong talking to the gateway.
#[derive(Debug, thiserror::Error)]
pub enum BoswellError {
    /// The request did not complete.
    #[error("request to Boswell failed: {0}")]
    Transport(#[from] reqwest::Error),
    /// The gateway answered with an error status.
    #[error("Boswell answered {status}: {body}")]
    Status {
        /// The HTTP status.
        status: u16,
        /// The response body, for the log.
        body: String,
    },
}

/// The outcome of a feed read.
#[derive(Debug, Clone, PartialEq)]
pub enum FeedRead {
    /// A page of events and the cursor to send next.
    Page {
        /// The events, oldest first, as the gateway rendered them.
        events: Vec<Value>,
        /// The next cursor.
        next: String,
    },
    /// The cursor fell behind the retention window; resume from `head` after
    /// re-syncing from a dossier.
    Gone {
        /// The current head.
        head: String,
    },
}

/// A gateway client bound to one agent label.
#[derive(Debug, Clone)]
pub struct Boswell {
    http: reqwest::Client,
    config: BoswellConfig,
}

impl Boswell {
    /// A client with `timeout` on every request. Hooks run on the prompt path
    /// and use a short one.
    pub fn new(config: BoswellConfig, timeout: Duration) -> Result<Self, BoswellError> {
        Ok(Self {
            http: reqwest::Client::builder().timeout(timeout).build()?,
            config,
        })
    }

    /// The configured namespace, if any.
    pub fn namespace(&self) -> Option<&str> {
        self.config.namespace.as_deref()
    }

    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        agent: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let mut req = self
            .http
            .request(method, format!("{}{}", self.config.url, path))
            .bearer_auth(&self.config.token);
        if let Some(label) = agent {
            req = req.header("x-boswell-agent", label);
        }
        req
    }

    async fn json(resp: reqwest::Response) -> Result<Value, BoswellError> {
        let status = resp.status();
        if !status.is_success() {
            return Err(BoswellError::Status {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(resp.json().await?)
    }

    /// `POST /v1/dossier` in the width view for one subject or topic.
    pub async fn dossier_width(
        &self,
        subject: Option<&str>,
        topic: Option<&str>,
    ) -> Result<Value, BoswellError> {
        let mut body = json!({ "view": "width" });
        if let Some(s) = subject {
            body["subject"] = json!(s);
        }
        if let Some(t) = topic {
            body["topic"] = json!(t);
        }
        if let Some(ns) = self.namespace() {
            body["namespace"] = json!(ns);
        }
        let resp = self
            .request(reqwest::Method::POST, "/v1/dossier", None)
            .json(&body)
            .send()
            .await?;
        Self::json(resp).await
    }

    /// `GET /v1/feed`. `since: None` with `from_head` returns only the head.
    ///
    /// `exclude_self` leaves out every event by the token's principal, which is
    /// right for a process with a key of its own (the bridge) and wrong for a
    /// seat whose key its teammates share (the hooks), who would lose each
    /// other's news along with their own.
    pub async fn feed(
        &self,
        since: Option<&str>,
        from_head: bool,
        changes: &[String],
        exclude_self: bool,
        limit: u32,
    ) -> Result<FeedRead, BoswellError> {
        let mut query: Vec<(&str, String)> = vec![
            ("limit", limit.to_string()),
            ("exclude_self", exclude_self.to_string()),
        ];
        if let Some(s) = since {
            query.push(("since", s.to_string()));
        }
        if from_head {
            query.push(("from", "head".to_string()));
        }
        if !changes.is_empty() {
            query.push(("changes", changes.join(",")));
        }
        if let Some(ns) = self.namespace() {
            query.push(("namespace", ns.to_string()));
        }
        let resp = self
            .request(reqwest::Method::GET, "/v1/feed", None)
            .query(&query)
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::GONE {
            let body: Value = resp.json().await.unwrap_or_default();
            return Ok(FeedRead::Gone {
                head: body["head"].as_str().unwrap_or("0").to_string(),
            });
        }
        let body = Self::json(resp).await?;
        Ok(FeedRead::Page {
            events: body["events"].as_array().cloned().unwrap_or_default(),
            next: body["next"].as_str().unwrap_or("0").to_string(),
        })
    }

    /// `POST /v1/claims` on behalf of `agent`, returning the claim id.
    pub async fn assert(
        &self,
        agent: Option<&str>,
        subject: &str,
        predicate: &str,
        object: &str,
    ) -> Result<String, BoswellError> {
        let namespace = self.namespace().unwrap_or("openrig");
        let resp = self
            .request(reqwest::Method::POST, "/v1/claims", agent)
            .json(&json!({
                "namespace": namespace,
                "subject": subject,
                "predicate": predicate,
                "object": object,
                "tier": "task",
            }))
            .send()
            .await?;
        let body = Self::json(resp).await?;
        Ok(body["id"].as_str().unwrap_or_default().to_string())
    }

    /// `DELETE /v1/claims/{id}`. A claim already gone is not an error.
    pub async fn forget(&self, claim_id: &str) -> Result<(), BoswellError> {
        let resp = self
            .request(
                reqwest::Method::DELETE,
                &format!("/v1/claims/{}", claim_id),
                None,
            )
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        Self::json(resp).await.map(|_| ())
    }
}
