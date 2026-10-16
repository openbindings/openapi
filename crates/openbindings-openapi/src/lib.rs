//! Experimental OpenAPI inspection, synthesis and finite JSON invocation.
//!
//! This optional adapter implements private 3.1 and 3.0 JSON experiments. Their meaning
//! is documented in the package README; it is not a published binding standard.
//! Core remains unaware of OpenAPI, transport and operation effects.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod analysis;
mod invocation;
mod security;
mod synthesis;

use dynamic_openapi_client::graph::DocumentBuilder;
pub use dynamic_openapi_client::{
    Limits,
    graph::{GraphLimits, ResourceInput, ResourceLayout},
};
pub use invocation::{InvocationFuture, Transport};
use openbindings::{JsonValue, Source, WorkControl};
use openbindings_source_inspection::{
    CancellationToken, InspectionError, InspectionErrorKind, SourceInspection, SourceInspector,
    SupportedKind,
};
use std::{
    future::{Ready, ready},
    sync::{Arc, Mutex},
};

/// Exact private experiment identifier. Saved r1 documents retain this meaning.
pub const EXPERIMENTAL_KIND: &str = "local.openapi-json-probe.r1";
/// Private per-attempt security selection and caller-approved credential origins.
pub const SECURITY_POINT: &str = "local.openapi-json-probe.r1.security";
/// Configuration point containing a declared absolute server URL selector.
pub const SERVER_POINT: &str = "local.openapi-json-probe.r1.server";
/// Kind-defined unsuccessful HTTP status, with application data `{ "status": n }`.
pub const HTTP_STATUS_ERROR: &str = "local.openapi-json-probe.r1.http-status";
/// Exact private OpenAPI 3.0 experiment identifier; independent of the 3.1 r1 kind.
pub const EXPERIMENTAL_KIND_30: &str = "local.openapi30-json-probe.r1";
/// OpenAPI 3.0 private per-attempt security selection and credential origins.
pub const SECURITY_POINT_30: &str = "local.openapi30-json-probe.r1.security";
/// OpenAPI 3.0 experiment's declared absolute server URL configuration point.
pub const SERVER_POINT_30: &str = "local.openapi30-json-probe.r1.server";
/// OpenAPI 3.0 experiment's unsuccessful HTTP status, with data `{ "status": n }`.
pub const HTTP_STATUS_ERROR_30: &str = "local.openapi30-json-probe.r1.http-status";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    OpenApi31,
    OpenApi30,
}
impl Kind {
    const ALL: [Self; 2] = [Self::OpenApi31, Self::OpenApi30];
    fn from_label(label: &str) -> Option<Self> {
        match label {
            EXPERIMENTAL_KIND => Some(Self::OpenApi31),
            EXPERIMENTAL_KIND_30 => Some(Self::OpenApi30),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::OpenApi31 => EXPERIMENTAL_KIND,
            Self::OpenApi30 => EXPERIMENTAL_KIND_30,
        }
    }
    fn edition(self) -> dynamic_openapi_client::inspect::Edition {
        match self {
            Self::OpenApi31 => dynamic_openapi_client::inspect::Edition::OpenApi31,
            Self::OpenApi30 => dynamic_openapi_client::inspect::Edition::OpenApi30,
        }
    }
    fn server_point(self) -> &'static str {
        match self {
            Self::OpenApi31 => SERVER_POINT,
            Self::OpenApi30 => SERVER_POINT_30,
        }
    }
    fn security_point(self) -> &'static str {
        match self {
            Self::OpenApi31 => SECURITY_POINT,
            Self::OpenApi30 => SECURITY_POINT_30,
        }
    }
    fn status_error(self) -> &'static str {
        match self {
            Self::OpenApi31 => HTTP_STATUS_ERROR,
            Self::OpenApi30 => HTTP_STATUS_ERROR_30,
        }
    }
}

/// Fixed resources and finite admission budgets. Changing these requires a new provider.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Options {
    /// Client per-resource, operation and HTTP budgets.
    pub limits: Limits,
    /// Aggregate resource graph budgets.
    pub graph_limits: GraphLimits,
    /// Explicit resource bytes and identities; no automatic acquisition occurs.
    pub resources: Vec<ResourceInput>,
    /// Maximum encoded source content, including its document text.
    pub source_bytes: usize,
    /// Maximum charged adapter inventory bytes, including retained server/parameter metadata.
    /// This is a logical work budget, not an allocator or process RSS measurement.
    pub result_bytes: usize,
    /// Maximum encoded per-attempt context for a profile containing authenticated alternatives.
    /// Unsecured profiles ignore authentication context. No retained-arena/RSS guarantee.
    pub context_bytes: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            graph_limits: GraphLimits::default(),
            resources: vec![],
            source_bytes: 16 << 20,
            result_bytes: 8 << 20,
            context_bytes: 64 << 10,
        }
    }
}

/// Configuration/source admission category. Messages never include source values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Invalid carrier or malformed supplied data.
    InvalidInput,
    /// A different kind or an unimplemented source/profile.
    Unsupported,
    /// The available resources cannot establish the source.
    Unreadable,
    /// A configured budget was exhausted.
    Limit,
    /// Cooperative cancellation.
    Cancelled,
}
/// Safe adapter admission error; interface facades map it into their own error vocabulary.
#[derive(Clone, Debug)]
pub struct Error {
    kind: ErrorKind,
    code: &'static str,
}
impl Error {
    /// Typed admission category.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
    /// Stable implementation diagnostic, not a binding-spec requirement.
    pub fn code(&self) -> &'static str {
        self.code
    }
    pub(crate) fn new(kind: ErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }
    pub(crate) fn inspection(&self) -> InspectionError {
        let kind = match self.kind {
            ErrorKind::InvalidInput => InspectionErrorKind::InvalidInput,
            ErrorKind::Unsupported if self.code == "unsupported-kind" => {
                InspectionErrorKind::UnsupportedKind
            }
            ErrorKind::Unsupported => InspectionErrorKind::UnreadableSource,
            ErrorKind::Unreadable => InspectionErrorKind::UnreadableSource,
            ErrorKind::Limit => InspectionErrorKind::ProviderFailure,
            ErrorKind::Cancelled => InspectionErrorKind::Cancelled,
        };
        InspectionError::new(
            kind,
            format!("openbindings.openapi.{}", self.code),
            self.to_string(),
        )
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpenAPI adapter admission: {}", self.code)
    }
}
impl std::error::Error for Error {}

/// Reusable adapter. Inspection and synthesis need no transport; bind one once
/// with [`Self::with_transport`] to enable the existing BindingInvoker interface.
///
/// One immutable prepared source is cached. No mutex is held during source
/// analysis or host I/O. Concurrent misses may compute duplicate results;
/// the last completed analysis replaces the single cache entry.
/// Supplied resources must be reconstructed for an independent cold reload.
pub struct OpenApi<T = ()> {
    options: Options,
    transport: T,
    cached: Mutex<Option<Arc<analysis::Prepared>>>,
    #[cfg(test)]
    preparations: std::sync::atomic::AtomicUsize,
}
impl Default for OpenApi<()> {
    fn default() -> Self {
        Self {
            options: Options::default(),
            transport: (),
            cached: Mutex::new(None),
            #[cfg(test)]
            preparations: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}
impl OpenApi<()> {
    /// Check a fixed supplied-resource collection without performing I/O.
    pub fn new(options: Options) -> Result<Self, Error> {
        let mut builder =
            DocumentBuilder::new(options.limits.clone(), options.graph_limits.clone());
        for resource in &options.resources {
            builder
                .add(resource.clone())
                .map_err(|_| Error::new(ErrorKind::InvalidInput, "resource-admission"))?;
        }
        Ok(Self {
            options,
            ..Self::default()
        })
    }
}
impl<T> OpenApi<T> {
    /// Bind application-owned HTTP execution. Native and local non-Send hosts are supported.
    pub fn with_transport<H: Transport>(self, transport: H) -> OpenApi<H> {
        OpenApi {
            options: self.options,
            transport,
            cached: self.cached,
            #[cfg(test)]
            preparations: self.preparations,
        }
    }
    fn prepare(
        &self,
        source: &Source,
        control: &dyn WorkControl,
    ) -> Result<Arc<analysis::Prepared>, Error> {
        self.prepare_with(source, control, |kind, content| {
            analysis::prepare(kind, content, &self.options, control)
        })
    }
    fn prepare_with(
        &self,
        source: &Source,
        control: &dyn WorkControl,
        analyze: impl FnOnce(Kind, &JsonValue) -> Result<analysis::Prepared, Error>,
    ) -> Result<Arc<analysis::Prepared>, Error> {
        check(control)?;
        let kind = Kind::from_label(&source.kind)
            .ok_or_else(|| Error::new(ErrorKind::Unsupported, "unsupported-kind"))?;
        let content = source
            .content
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "missing-source-content"))?;
        if content.bytes().len() > self.options.source_bytes {
            return Err(Error::new(ErrorKind::Limit, "source-limit"));
        }
        let evicted = {
            let mut cache = self
                .cached
                .lock()
                .map_err(|_| Error::new(ErrorKind::Unreadable, "poisoned-cache"))?;
            check(control)?;
            if let Some(prepared) = cache.as_ref().filter(|p| {
                p.kind == kind
                    && (std::ptr::eq(p.content.text(), content.text())
                        || p.content.text() == content.text())
            }) {
                return Ok(prepared.clone());
            }
            cache.take()
        };
        // Drop the old graph and analyze the new source without the cache mutex.
        drop(evicted);
        let prepared = Arc::new(analyze(kind, content)?);
        #[cfg(test)]
        self.preparations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut cache = self
            .cached
            .lock()
            .map_err(|_| Error::new(ErrorKind::Unreadable, "poisoned-cache"))?;
        check(control)?;
        *cache = Some(prepared.clone());
        Ok(prepared)
    }
}
impl<T> SourceInspector for OpenApi<T> {
    type Inspection<'a>
        = Ready<Result<SourceInspection, InspectionError>>
    where
        Self: 'a;
    fn supports_kind(&self, kind: &str) -> bool {
        Kind::from_label(kind).is_some()
    }
    fn supported_kinds(&self) -> Vec<SupportedKind> {
        Kind::ALL
            .into_iter()
            .map(|kind| SupportedKind {
                kind: kind.label().into(),
                description: Some(
                    "Private finite JSON experiment; consult its qualified capability profile"
                        .into(),
                ),
            })
            .collect()
    }
    fn inspect<'a>(
        &'a self,
        source: &'a Source,
        control: CancellationToken,
    ) -> Self::Inspection<'a> {
        ready(
            self.prepare(source, &control)
                .map(|p| p.inspection())
                .map_err(|e| e.inspection()),
        )
    }
}
fn check(control: &dyn WorkControl) -> Result<(), Error> {
    if control.is_cancelled() {
        Err(Error::new(ErrorKind::Cancelled, "cancelled"))
    } else {
        Ok(())
    }
}
fn json(value: &serde_json_compat::Value) -> Result<JsonValue, Error> {
    JsonValue::parse(value.to_string()).map_err(|_| Error::new(ErrorKind::Limit, "result-limit"))
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use std::sync::atomic::Ordering;
    fn source(uri: &str) -> Source {
        Source {kind:EXPERIMENTAL_KIND.into(),content:Some(json(&serde_json_compat::json!({"document":include_str!("../tests/fixtures/r1/parcels.json"),"retrievalUri":uri})).unwrap()),..Default::default()}
    }
    #[test]
    fn one_source_cache_checks_content_base_and_fixed_options() {
        let provider = OpenApi::default();
        let control = CancellationToken::new();
        let a = source("https://a.invalid/doc.json");
        let first = provider.prepare(&a, &control).unwrap();
        let same = provider.prepare(&a.clone(), &control).unwrap();
        assert!(Arc::ptr_eq(&first, &same));
        let equivalent = source("https://a.invalid/doc.json");
        assert!(Arc::ptr_eq(
            &first,
            &provider.prepare(&equivalent, &control).unwrap()
        ));
        assert_eq!(provider.preparations.load(Ordering::Relaxed), 1);
        let b = source("https://b.invalid/doc.json");
        assert!(!Arc::ptr_eq(
            &first,
            &provider.prepare(&b, &control).unwrap()
        ));
        assert_eq!(provider.preparations.load(Ordering::Relaxed), 2);
        drop(same);
        drop(first);
        let weak = Arc::downgrade(provider.cached.lock().unwrap().as_ref().unwrap());
        provider.prepare(&a, &control).unwrap();
        assert!(weak.upgrade().is_none());
        let options = Options {
            source_bytes: 0,
            ..Default::default()
        };
        assert!(
            OpenApi::new(options)
                .unwrap()
                .prepare(&a, &control)
                .is_err()
        );
    }

    #[test]
    fn different_documents_analyze_concurrently_without_the_cache_lock() {
        let provider = OpenApi::default();
        let sources = [
            source("https://a.invalid/doc.json"),
            Source {
                kind: EXPERIMENTAL_KIND.into(),
                content: Some(
                    json(&serde_json_compat::json!({
                        "document": include_str!("../tests/fixtures/r1/destinations.json")
                    }))
                    .unwrap(),
                ),
                ..Default::default()
            },
        ];
        let analyzing = std::sync::Barrier::new(2);
        let (started, entered) = std::sync::mpsc::channel();
        let analyze = |source: &Source, started: Option<std::sync::mpsc::Sender<()>>| {
            let control = CancellationToken::new();
            provider
                .prepare_with(source, &control, |kind, content| {
                    // No other thread can be in cache lookup at this checkpoint.
                    // Fail before the barrier if analysis holds the mutex.
                    assert!(provider.cached.try_lock().is_ok());
                    if let Some(started) = started {
                        started.send(()).unwrap();
                    }
                    // Neither analysis can finish until both have entered.
                    analyzing.wait();
                    analysis::prepare(kind, content, &provider.options, &control)
                })
                .unwrap()
        };
        let prepared = std::thread::scope(|scope| {
            let analyze = &analyze;
            let first_source = &sources[0];
            let first = scope.spawn(move || analyze(first_source, Some(started)));
            // Start the second lookup only after the first analysis is waiting.
            entered.recv().unwrap();
            let second = scope.spawn(|| analyze(&sources[1], None));
            vec![first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(provider.preparations.load(Ordering::Relaxed), 2);
        for (source, prepared) in sources.iter().zip(&prepared) {
            let expected = analysis::prepare(
                Kind::OpenApi31,
                source.content.as_ref().unwrap(),
                &provider.options,
                &CancellationToken::new(),
            )
            .unwrap();
            assert_eq!(
                prepared.inspection().to_json().unwrap().text(),
                expected.inspection().to_json().unwrap().text()
            );
        }
        let weak: Vec<_> = prepared.iter().map(Arc::downgrade).collect();
        drop(prepared);
        // Exactly one result is retained, whichever computation completed last.
        assert_eq!(weak.iter().filter(|p| p.upgrade().is_some()).count(), 1);
    }

    #[test]
    fn failed_analysis_evicts_previous_entry_and_preserves_result_budget() {
        let provider = OpenApi::default();
        let control = CancellationToken::new();
        let valid = source("https://a.invalid/doc.json");
        let weak = Arc::downgrade(&provider.prepare(&valid, &control).unwrap());
        let malformed = Source {
            content: Some(json(&serde_json_compat::json!({"document": "{"})).unwrap()),
            ..valid.clone()
        };
        assert!(provider.prepare(&malformed, &control).is_err());
        assert!(weak.upgrade().is_none());
        assert!(provider.cached.lock().unwrap().is_none());

        let bounded = OpenApi::new(Options {
            result_bytes: 1,
            ..Default::default()
        })
        .unwrap();
        let error = bounded.prepare(&valid, &control).err().unwrap();
        assert_eq!(error.kind(), ErrorKind::Limit);
        assert_eq!(error.code(), "result-limit");
        assert!(bounded.cached.lock().unwrap().is_none());
    }
}
