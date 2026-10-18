use crate::{Error, ErrorKind, Kind, OpenApi, check};
use openbindings::{Binding, Document, Edit, EditError, JsonValue, Operation, WorkControl};
use openbindings_interface_synthesis::{
    CancellationToken, CoverageClaim, CoverageEntry, CoverageLimitation, CoverageScope,
    CoverageStatus, InterfaceSynthesizer, SupportedKind, SynthesisCandidate, SynthesisError,
    SynthesisErrorKind, SynthesisRequest,
};
use std::{
    collections::BTreeMap,
    future::{Ready, ready},
};
impl Error {
    fn synthesis(&self) -> SynthesisError {
        let kind = match self.kind() {
            ErrorKind::InvalidInput => SynthesisErrorKind::InvalidInput,
            ErrorKind::Unsupported => SynthesisErrorKind::UnsupportedRequest,
            ErrorKind::Unreadable => SynthesisErrorKind::UnreadableSource,
            ErrorKind::Limit => SynthesisErrorKind::ProviderFailure,
            ErrorKind::Cancelled => SynthesisErrorKind::Cancelled,
        };
        SynthesisError::new(
            kind,
            format!("openbindings.openapi.{}", self.code()),
            self.to_string(),
        )
    }
}
impl<T> InterfaceSynthesizer for OpenApi<T> {
    type Synthesis<'a>
        = Ready<Result<SynthesisCandidate, SynthesisError>>
    where
        Self: 'a;
    fn supports_kind(&self, kind: &str) -> bool {
        Kind::from_label(kind).is_some()
    }
    fn supported_kinds(&self) -> Vec<SupportedKind> {
        Kind::ALL
            .into_iter()
            .map(|kind| {
                SupportedKind::new(kind.label()).with_description(
                    "Private JSON experiment, with disclosed schema projection and capability limits",
                )
            })
            .collect()
    }
    fn synthesize<'a>(
        &'a self,
        request: &'a SynthesisRequest<'a>,
        control: CancellationToken,
    ) -> Self::Synthesis<'a> {
        ready(self.derive(request, &control))
    }
}
impl<T> OpenApi<T> {
    fn derive(
        &self,
        request: &SynthesisRequest<'_>,
        control: &dyn WorkControl,
    ) -> Result<SynthesisCandidate, SynthesisError> {
        check(control).map_err(|e| e.synthesis())?;
        if request.sources.len() > 1 {
            return Err(SynthesisError::new(
                SynthesisErrorKind::UnsupportedRequest,
                "openbindings.openapi.composition-unimplemented",
                "This experiment accepts at most one supplied source",
            ));
        }
        let mut document = Document::new();
        let mut operations = BTreeMap::new();
        let mut sources = None;
        let mut bindings = BTreeMap::new();
        let mut coverage = CoverageClaim::complete(Vec::new());
        if let Some(input) = request.sources.first() {
            let prepared = self
                .prepare(input.source, control)
                .map_err(|e| e.synthesis())?;
            let source_key = input.name.unwrap_or("api").to_owned();
            sources = Some(BTreeMap::from([(source_key.clone(), input.source.clone())]));
            for (index, unit) in prepared.units.iter().enumerate() {
                check(control).map_err(|e| e.synthesis())?;
                let mut entry = CoverageEntry::new(
                    0,
                    &unit.source_ref,
                    CoverageScope::Target,
                    CoverageStatus::Represented,
                );
                match &unit.profile {
                    Ok(profile) => {
                        // Core keys have an ASCII grammar. The ordinal guarantees
                        // uniqueness; the short path hint is authoring convenience.
                        let hint: String = unit
                            .operation
                            .path()
                            .chars()
                            .take(48)
                            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                            .collect();
                        let key = format!(
                            "{}_{}_{}",
                            unit.operation.method().as_str().to_ascii_lowercase(),
                            index + 1,
                            hint
                        );
                        operations.insert(
                            key.clone(),
                            Operation {
                                input: Some(profile.input_schema.clone()),
                                ..Default::default()
                            },
                        );
                        bindings.insert(
                            key.clone(),
                            Binding {
                                operation: key.clone(),
                                source: source_key.clone(),
                                content: Some(unit.binding.clone()),
                                ..Default::default()
                            },
                        );
                        entry.source_key = Some(source_key.clone());
                        entry.operation_key = Some(key.clone());
                        entry.binding_key = Some(key.clone());
                        if profile.security.iter().any(|a| !a.is_empty()) {
                            entry.requirements = Some(vec![prepared.kind.security_point().into()]);
                            entry.details = Some(BTreeMap::from([(
                                "security".into(),
                                crate::json(&crate::security::details(profile))
                                    .map_err(|e| e.synthesis())?,
                            )]));
                        }
                        coverage.entries.push(entry);
                        if profile.schema_loss {
                            let mut loss = CoverageEntry::new(
                                0,
                                &unit.source_ref,
                                CoverageScope::Projection,
                                CoverageStatus::Lossy,
                            );
                            loss.source_key = Some(source_key.clone());
                            loss.operation_key = Some(key.clone());
                            loss.binding_key = Some(key);
                            loss.reason_code =
                                Some("openbindings.openapi.schema-contract-not-projected".into());
                            loss.message=Some("The input schema describes the adapter envelope; source value constraints and response schemas are not fully projected".into());
                            coverage.entries.push(loss);
                        }
                    }
                    Err(reason) => {
                        entry.status = CoverageStatus::ImplementationUnsupported;
                        entry.reason_code = Some(format!("openbindings.openapi.{reason}"));
                        entry.message=Some("This implementation cannot faithfully represent the declared operation profile; the target is not declared invalid or excluded by a kind rule".into());
                        coverage.entries.push(entry);
                    }
                }
            }
            coverage.exhaustive = prepared.exhaustive && !prepared.extra_interactions;
            if !coverage.exhaustive {
                let resources: Vec<_> = prepared
                    .document
                    .graph_issues()
                    .filter_map(|d| {
                        d.resource_need()
                            .and_then(|r| r.requested_uri())
                            .map(str::to_owned)
                    })
                    .collect();
                let details =
                    crate::json(&serde_json_compat::json!({"requiredResources":resources}))
                        .map_err(|e| e.synthesis())?;
                coverage.limitation = Some(
                    CoverageLimitation::new(
                        "openbindings.openapi.inventory-incomplete",
                        "The supplied resources and implemented interaction profile do not establish every source unit; no empty/full inventory claim is made",
                    )
                    .with_details(BTreeMap::from([("inventory".into(), details)])),
                );
            }
        }
        check(control).map_err(|e| e.synthesis())?;
        // Build typed parts first; commit metadata and all namespaces in one batch.
        let metadata: Vec<_> = [
            ("/openbindings", request.openbindings_version),
            ("/name", request.name),
            ("/version", request.version),
            ("/description", request.description),
        ]
        .into_iter()
        .filter_map(|(pointer, value)| value.map(|value| (pointer, value)))
        .map(|(pointer, value)| {
            // A value too large to admit is an editing failure, like the batch below.
            JsonValue::string(value)
                .map(|value| (pointer, value))
                .map_err(|error| SynthesisError::from(EditError::from(error)))
        })
        .collect::<Result<_, _>>()?;
        let mut edits: Vec<_> = metadata
            .iter()
            .map(|(pointer, value)| Edit::Set {
                pointer: (*pointer).into(),
                value,
            })
            .collect();
        edits.push(Edit::SetOperations { value: &operations });
        if let Some(sources) = &sources {
            edits.push(Edit::SetSources { value: sources });
        }
        if !bindings.is_empty() {
            edits.push(Edit::SetBindings { value: &bindings });
        }
        document.edit(&edits)?;
        if document.value().bytes().len()
            > self
                .options
                .source_bytes
                .saturating_add(self.options.result_bytes)
        {
            return Err(Error::new(ErrorKind::Limit, "synthesis-result-limit").synthesis());
        }
        Ok(SynthesisCandidate::new(document, coverage))
    }
}
