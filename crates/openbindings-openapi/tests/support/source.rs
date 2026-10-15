use openbindings::{Source, SourceView};

/// Retain a source's typed metadata and exact opaque fields for provider calls.
pub fn source(view: SourceView) -> Source {
    Source {
        kind: view.kind().unwrap().as_str().unwrap().to_owned(),
        content: view.content().map(|value| value.to_owned()),
        description: view
            .description()
            .unwrap()
            .map(|value| value.as_str().unwrap().to_owned()),
        additional_fields: view
            .value()
            .members()
            .unwrap()
            .filter_map(|member| {
                let key = member.name.as_str().unwrap();
                (!matches!(key, "kind" | "content" | "description"))
                    .then(|| (key.to_owned(), member.value.to_owned()))
            })
            .collect(),
    }
}
