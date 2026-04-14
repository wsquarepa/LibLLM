use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{
    BUILTIN_REASONING, REASONING_OFF, list_json_names_in_dir, load_json_from_dir,
    reasoning_presets_dir,
};

/// A reasoning-mode wrapper that adds think-aloud prefix/suffix around assistant output.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReasoningPreset {
    pub name: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub separator: String,
}

fn load_builtin_reasoning(name: &str) -> Option<ReasoningPreset> {
    let name_lower = name.to_lowercase();
    for (builtin_name, json) in BUILTIN_REASONING {
        if builtin_name.to_lowercase() == name_lower {
            return serde_json::from_str(json).ok();
        }
    }
    None
}

/// Resolves a reasoning preset by name, returning `None` for "OFF" or an empty string.
pub fn resolve_reasoning_preset(name: &str) -> Option<ReasoningPreset> {
    if name.eq_ignore_ascii_case(REASONING_OFF) || name.is_empty() {
        return None;
    }
    if let Some(preset) = load_json_from_dir(&reasoning_presets_dir(), name) {
        return Some(preset);
    }
    load_builtin_reasoning(name)
}

/// Returns all available reasoning preset names, always starting with "OFF".
pub fn list_reasoning_preset_names() -> Vec<String> {
    let mut names = vec![REASONING_OFF.to_owned()];
    let mut seen: HashSet<String> = HashSet::from([REASONING_OFF.to_lowercase()]);

    for dir_name in list_json_names_in_dir(&reasoning_presets_dir()) {
        if seen.insert(dir_name.to_lowercase()) {
            names.push(dir_name);
        }
    }

    for (builtin_name, _) in BUILTIN_REASONING {
        if seen.insert(builtin_name.to_lowercase()) {
            names.push((*builtin_name).to_owned());
        }
    }

    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_data_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        crate::config::set_data_dir(dir.path().to_path_buf()).ok();
        dir
    }

    #[test]
    fn resolve_reasoning_preset_off() {
        let _dir = setup_data_dir();
        let result = resolve_reasoning_preset("OFF");
        assert!(result.is_none(), "\"OFF\" should resolve to None");
    }

    #[test]
    fn resolve_reasoning_preset_empty() {
        let _dir = setup_data_dir();
        let result = resolve_reasoning_preset("");
        assert!(result.is_none(), "empty string should resolve to None");
    }

    #[test]
    fn list_reasoning_preset_names_includes_off() {
        let _dir = setup_data_dir();
        let names = list_reasoning_preset_names();
        assert!(
            !names.is_empty(),
            "reasoning preset list should not be empty"
        );
        assert_eq!(
            names[0], "OFF",
            "first reasoning preset name should be \"OFF\", got: {names:?}"
        );
    }
}
