//! Google Vertex AI adapter.
//!
//! Vertex uses the same `generateContent` shapes as the Gemini API but with a
//! different URL scheme and OAuth2 bearer tokens:
//! `{base}/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent`
//! Auth: `Authorization: Bearer <access_token>` (OAuth2 service account).
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::adaptor::{AdaptedResponse, Adaptor, UpstreamResponse};
use crate::error::RelayError;
use crate::sse::gemini::GeminiToOpenAiStream;
use crate::usage::extract_gemini_usage;
use crate::value::{RelayInfo, RelayValue};

/// A Vertex channel credential: `project|location|access_token` or just a token.
#[derive(Debug, Clone)]
pub struct VertexCredential {
    pub project: String,
    pub location: String,
    pub access_token: String,
}

pub fn parse_credential(raw: &str) -> VertexCredential {
    let parts: Vec<&str> = raw.split('|').map(|s| s.trim()).collect();
    match parts.as_slice() {
        [project, location, token] => VertexCredential {
            project: project.to_string(),
            location: location.to_string(),
            access_token: token.to_string(),
        },
        [token] => VertexCredential {
            project: String::new(),
            location: "us-central1".to_string(),
            access_token: token.to_string(),
        },
        _ => VertexCredential {
            project: String::new(),
            location: "us-central1".to_string(),
            access_token: String::new(),
        },
    }
}

pub struct VertexAdaptor;

#[async_trait]
impl Adaptor for VertexAdaptor {
    fn name(&self) -> &'static str {
        "vertex"
    }

    fn request_url(&self, info: &RelayInfo) -> Result<String, RelayError> {
        let base = info.base_url.trim_end_matches('/');
        let action = if info.is_stream {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        Ok(format!("{}/{}", base, action))
    }

    fn setup_headers(&self, headers: &mut HeaderMap, _info: &RelayInfo) -> Result<(), RelayError> {
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        Ok(())
    }

    fn convert_request(
        &self,
        info: &RelayInfo,
        body: &serde_json::Value,
    ) -> Result<RelayValue, RelayError> {
        let g = crate::convert::openai_to_gemini::convert(body, info.is_stream)?;
        Ok(RelayValue::Raw(g))
    }

    fn convert_response(
        &self,
        info: &RelayInfo,
        resp: &UpstreamResponse,
    ) -> Result<AdaptedResponse, RelayError> {
        if info.is_stream && resp.is_stream {
            let mut conv = GeminiToOpenAiStream::new(&info.origin_model);
            let (body, usage) = conv.run(&resp.body);
            return Ok(AdaptedResponse {
                body: body.into(),
                usage,
            });
        }
        let usage = extract_gemini_usage(&resp.body);
        let body =
            crate::convert::gemini_to_openai::convert_response(&resp.body, &info.origin_model)?;
        Ok(AdaptedResponse {
            body: body.into(),
            usage,
        })
    }
}
