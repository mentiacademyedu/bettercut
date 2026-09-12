//! The bundled templates are held to the same validator as any other.

use std::collections::HashSet;

use bettercut_templates::starters::FILES;
use bettercut_templates::{parse, starters};

#[test]
fn every_starter_is_valid() {
    for (name, json) in FILES {
        if let Err(problems) = parse(json) {
            let listed: Vec<String> = problems.iter().map(ToString::to_string).collect();
            panic!("{name} is not a valid template:\n{}", listed.join("\n"));
        }
    }
    assert_eq!(starters().len(), FILES.len());
}

#[test]
fn starter_ids_are_unique_and_match_their_file_names() {
    let mut seen = HashSet::new();
    for (template, (file, _)) in starters().iter().zip(FILES) {
        assert!(seen.insert(template.id.clone()), "{} twice", template.id);
        assert_eq!(format!("{}.json", template.id), file);
    }
}
