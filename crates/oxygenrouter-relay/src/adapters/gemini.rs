//! Gemini adapter — Google `generateContent` / `streamGenerateContent`.
//!
//! URL: `{base}/{version}/models/{model}:generateContent`
//!      `{base}/{version}/models/{model}:streamGenerateContent?alt=sse`
//! Auth: `x-goog-api-key: <key>` (not Bearer).
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::adaptor::{AdaptedResponse, Adaptor, UpstreamResponse};
use crate::convert::openai_to_gemini;
use crate::error::RelayError;
use crate::sse::gemini::GeminiToOpenAiStream;
use crate::usage::extract_gemini_usage;
use crate::value::{RelayInfo, RelayValue};

pub struct GeminiAdaptor;

#[async_trait]
impl Adaptor for GeminiAdaptor {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn request_url(&self, info: &RelayInfo) -> Result<String, RelayError> {
        let base = info.base_url.trim_end_matches('/');
        let model = &info.upstream_model;
        if info.is_stream {
            Ok(format!(
                "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
                base, model
            ))
        } else {
            Ok(format!("{}/v1beta/models/{}:generateContent", base, model))
        }
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
        let g = openai_to_gemini::convert(body, info.is_stream)?;
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
