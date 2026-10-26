//! Following redirects. The client's step derives the request that follows a redirect,
//! applying the method, content, credential and downgrade rules and the chain limit
//! (`Limits::redirects`, 20 by default). The adapter sends that request through the same
//! `Transport::execute` as a separate call, and this module maps the step's answer onto
//! the kinds' outcomes.
use dynamic_openapi_client::{
    Outcome,
    diagnostics::{Code, DiagnosticReason, RedirectRefusal},
    transport::PreparedRequest,
};

/// How a received outcome ends the chain of requests when no redirect is followed.
pub(crate) enum End {
    /// The outcome is the final response, read under the response rules.
    Final,
    /// The invocation completes unsuccessfully with no value.
    Failed,
}

/// The request that follows `outcome`, which `sent` produced, or how the chain ends.
pub(crate) fn step(sent: &PreparedRequest, outcome: &Outcome) -> Result<PreparedRequest, End> {
    let refusal = match sent.follow_redirect(outcome) {
        Ok(Some(next)) => return Ok(next),
        // No response, a status that is not a redirect to follow (300, 304 and 305
        // included), or no `Location`.
        Ok(None) => return Err(End::Final),
        Err(refusal) => refusal,
    };
    // A browser's manual redirect is opaque: it exposes neither a `Location` to follow
    // nor a status to read as the final response. Letting the platform follow it would
    // skip the credential and downgrade rules, so a host that cannot observe a redirect
    // makes the call unsuccessful.
    if refusal.code() == Code::OpaqueRedirect {
        return Err(End::Failed);
    }
    Err(match refusal.reason() {
        // An https to http hop is not followed: the 3xx is the final response.
        Some(DiagnosticReason::Redirect(RedirectRefusal::Downgrade)) => End::Final,
        // More redirects than the limit, an unusable `Location`, incomplete header
        // evidence, or evidence the client cannot admit.
        _ => End::Failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dynamic_openapi_client::{
        Cancellation, Document, Header, Limits,
        transport::{
            BodyState, HeaderDelivery, HostCapabilities, Provenance, ResponseInput,
            TransportResult, UploadState,
        },
    };

    fn request(server: &str, limits: Limits) -> PreparedRequest {
        let text = format!(
            r#"{{"openapi":"3.1.2","info":{{"title":"Redirects","version":"1"}},"servers":[{{"url":"{server}"}}],"paths":{{"/x":{{"get":{{"responses":{{"200":{{"description":"ok"}}}}}}}}}}}}"#
        );
        let document = Document::parse(text.as_bytes(), None, limits).unwrap();
        document
            .operations()
            .next()
            .unwrap()
            .request()
            .prepare()
            .unwrap()
    }

    async fn outcome(
        sent: &PreparedRequest,
        status: u16,
        location: Option<&str>,
        delivery: HeaderDelivery,
        opaque: bool,
    ) -> Outcome {
        let location = location.map(str::to_owned);
        sent.invoke(
            Cancellation::default(),
            &HostCapabilities::programmable(),
            move |_, _| async move {
                let headers = location
                    .map(|l| vec![Header::new("location", l)])
                    .unwrap_or_default();
                let mut response = ResponseInput::new(
                    status,
                    headers,
                    std::sync::Arc::from([]),
                    BodyState::Complete,
                    Provenance::Wire,
                );
                response.header_delivery = delivery;
                response.opaque_redirect = opaque;
                TransportResult {
                    response: Some(response),
                    upload: UploadState::Complete,
                    error_detail: None,
                }
            },
        )
        .await
    }

    fn name(step: &Result<PreparedRequest, End>) -> &'static str {
        match step {
            Ok(_) => "follow",
            Err(End::Final) => "final",
            Err(End::Failed) => "failed",
        }
    }

    #[tokio::test]
    async fn each_answer_of_the_client_step_maps_to_one_outcome() {
        let complete = HeaderDelivery::Complete;
        let https = request("https://api.example.test", Limits::default());
        let http = request("http://api.example.test", Limits::default());
        let none = request(
            "https://api.example.test",
            Limits {
                redirects: 0,
                ..Limits::default()
            },
        );
        for (sent, status, location, delivery, opaque, expected) in [
            (&https, 302, Some("/y"), complete, false, "follow"),
            (
                &http,
                307,
                Some("https://api.example.test/y"),
                complete,
                false,
                "follow",
            ),
            (&https, 200, None, complete, false, "final"),
            (&https, 302, None, complete, false, "final"),
            (&https, 300, Some("/y"), complete, false, "final"),
            (&https, 304, Some("/y"), complete, false, "final"),
            (&https, 305, Some("/y"), complete, false, "final"),
            // An https to http hop is not followed, and the 3xx is the final response.
            (
                &https,
                301,
                Some("http://api.example.test/y"),
                complete,
                false,
                "final",
            ),
            (
                &https,
                308,
                Some("ftp://api.example.test/y"),
                complete,
                false,
                "failed",
            ),
            (
                &https,
                308,
                Some("https://user@api.example.test/y"),
                complete,
                false,
                "failed",
            ),
            (
                &https,
                303,
                Some("/y"),
                HeaderDelivery::Unspecified,
                false,
                "failed",
            ),
            (
                &https,
                303,
                Some("/y"),
                HeaderDelivery::Incomplete,
                false,
                "failed",
            ),
            (&none, 302, Some("/y"), complete, false, "failed"),
            (&https, 0, None, HeaderDelivery::Unspecified, true, "failed"),
        ] {
            let outcome = outcome(sent, status, location, delivery, opaque).await;
            assert_eq!(
                name(&step(sent, &outcome)),
                expected,
                "{status} {location:?} {delivery:?} {opaque}"
            );
        }
    }
}
