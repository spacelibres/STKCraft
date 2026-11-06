//! Shared application-boundary knowledge loading policy.

/// The user's knowledge base: `--knowledge <dir>`, or `STKCRAFT_KNOWLEDGE`, or the configuration
/// directory's `knowledge/`.
///
/// A `--knowledge` path that does not exist is an error with the path in it: a person who pointed the
/// platform at their documents deserves to know it read nothing, rather than getting answers that
/// silently came from memory.
pub(crate) fn library() -> Result<stkcraft_agent_knowledge::Library, String> {
    Ok(opened()?.library)
}

/// Open the configured library, retaining cache and skipped-file diagnostics.
pub(crate) fn opened() -> Result<stkcraft_agent_knowledge::Opened, String> {
    let explicit = std::env::var("STKCRAFT_KNOWLEDGE")
        .ok()
        .map(std::path::PathBuf::from);
    let directory = match explicit {
        Some(directory) => directory,
        None => crate::prefs::knowledge_dir().ok_or_else(|| {
            "this platform has no configuration directory for a knowledge base".to_owned()
        })?,
    };
    let cache = stkcraft_framework_storage::Paths::for_user()
        .ok()
        .map(|paths| stkcraft_framework_storage::Cache::at(paths.cache().join("knowledge")));
    open_at(
        &directory,
        std::env::var_os("STKCRAFT_KNOWLEDGE").is_some(),
        cache.as_ref(),
    )
}

/// Apply the same policy without reading process configuration.
fn open_at(
    directory: &std::path::Path,
    explicit: bool,
    cache: Option<&stkcraft_framework_storage::Cache>,
) -> Result<stkcraft_agent_knowledge::Opened, String> {
    if !directory.is_dir() {
        if explicit {
            return Err(format!(
                "STKCRAFT_KNOWLEDGE points at {}, which is not a directory",
                directory.display()
            ));
        }
        return Ok(stkcraft_agent_knowledge::Opened {
            library: stkcraft_agent_knowledge::Library::default(),
            reuse: stkcraft_agent_knowledge::Reuse::default(),
            skipped: Vec::new(),
        });
    }
    stkcraft_agent_knowledge::Library::open_cached(
        directory,
        stkcraft_agent_knowledge::Limits::default(),
        cache,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn missing_explicit_errors_and_missing_default_is_empty() {
        let path = std::path::Path::new("nonexistent-shared-knowledge-test-root");
        assert_eq!(
            open_at(path, true, None).err().unwrap(),
            format!(
                "STKCRAFT_KNOWLEDGE points at {}, which is not a directory",
                path.display()
            )
        );
        assert!(open_at(path, false, None).unwrap().library.is_empty());
    }
    #[test]
    fn markdown_has_exact_citation() {
        let path =
            std::env::temp_dir().join(format!("stkcraft-shared-knowledge-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("sensor.md"),
            "Maximum off-nadir angle is 45 degrees.\n",
        )
        .unwrap();
        let library = open_at(&path, true, None).unwrap().library;
        let search = library.search("maximum off-nadir angle", 4);
        assert_eq!(search.hits.first().unwrap().cite(), "sensor.md:lines 1-1");
        assert!(search.hits.first().unwrap().text.contains("45 degrees"));
        std::fs::remove_dir_all(path).unwrap();
    }
}
