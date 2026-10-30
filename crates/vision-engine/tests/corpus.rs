//! Corpus compatibility baseline.

use std::error::Error;
use std::path::PathBuf;

use serde_json::Value;
use vision::{Config, Engine, Snapshot, Viewport};

#[test]
fn corpus_matches_the_golden_manifest() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(
        root.join("corpus/goldens/manifest.json"),
    )?)?;
    let engine = Engine::native(Config::default())?;
    let pages = manifest
        .get("pages")
        .and_then(Value::as_array)
        .ok_or("manifest has no pages")?;

    for page in pages {
        let path = page
            .get("path")
            .and_then(Value::as_str)
            .ok_or("page has no path")?;
        let viewport = page
            .get("viewport")
            .and_then(Value::as_array)
            .ok_or("page has no viewport")?;
        let width = viewport
            .first()
            .and_then(Value::as_u64)
            .ok_or("viewport has no width")? as u32;
        let height = viewport
            .get(1)
            .and_then(Value::as_u64)
            .ok_or("viewport has no height")? as u32;

        let snapshot =
            Snapshot::from_file(root.join(path))?.with_viewport(Viewport::new(width, height));
        assert!(
            snapshot.warnings.is_empty(),
            "{path} has resource warnings: {:?}",
            snapshot.warnings
        );
        let capture = engine.render(&snapshot)?;
        assert!(capture.compat.warnings.is_empty(), "{path} has warnings");

        for tag in page
            .get("tags")
            .and_then(Value::as_array)
            .ok_or("page has no tags")?
        {
            let tag = tag.as_str().ok_or("tag is not a string")?;
            assert!(
                capture.elements.iter().any(|element| element.tag == tag),
                "{path} is missing <{tag}>"
            );
        }
        for id in page
            .get("ids")
            .and_then(Value::as_array)
            .ok_or("page has no ids")?
        {
            let id = id.as_str().ok_or("id is not a string")?;
            assert!(
                capture
                    .elements
                    .iter()
                    .any(|element| element.id.as_deref() == Some(id)),
                "{path} is missing id {id}"
            );
        }

        let expected = page
            .get("unsupported")
            .and_then(Value::as_array)
            .ok_or("page has no unsupported list")?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "unsupported entry is not a string".into())
            })
            .collect::<Result<Vec<String>, Box<dyn Error>>>()?;
        let actual = capture
            .compat
            .unsupported
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{path} compatibility baseline changed");
    }
    Ok(())
}
