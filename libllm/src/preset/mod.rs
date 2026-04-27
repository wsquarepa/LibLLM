//! Instruct, reasoning, and context template presets with file and builtin resolution.

mod context;
mod instruct;
pub mod matching;
mod reasoning;

use std::path::{Path, PathBuf};

pub use context::{
    ContextPreset, ContextVars, list_template_preset_names, resolve_template_preset,
};
pub use instruct::{
    InstructPreset, StopSequence, list_instruct_preset_names, resolve_instruct_preset,
};
pub use reasoning::{ReasoningPreset, list_reasoning_preset_names, resolve_reasoning_preset};

pub(crate) const DEFAULT_INSTRUCT_PRESET: &str = "Mistral V3-Tekken";
pub(crate) const REASONING_OFF: &str = "OFF";

pub(crate) const BUILTIN_INSTRUCT: &[(&str, &str)] = &[
    (
        "Mistral V3-Tekken",
        include_str!("../presets/instruct/mistral_v3_tekken.json"),
    ),
    (
        "Llama 3 Instruct",
        include_str!("../presets/instruct/llama3_instruct.json"),
    ),
    ("ChatML", include_str!("../presets/instruct/chatml.json")),
    ("Phi", include_str!("../presets/instruct/phi.json")),
    ("Alpaca", include_str!("../presets/instruct/alpaca.json")),
];

pub(crate) const BUILTIN_REASONING: &[(&str, &str)] = &[(
    "DeepSeek",
    include_str!("../presets/reasoning/deepseek.json"),
)];

pub(crate) const DEFAULT_TEMPLATE_PRESET: &str = "Default";

pub(crate) const BUILTIN_TEMPLATE: &[(&str, &str)] =
    &[("Default", include_str!("../presets/template/default.json"))];

pub fn instruct_presets_dir() -> PathBuf {
    crate::config::data_dir().join("presets").join("instruct")
}

pub fn reasoning_presets_dir() -> PathBuf {
    crate::config::data_dir().join("presets").join("reasoning")
}

pub fn template_presets_dir() -> PathBuf {
    crate::config::data_dir().join("presets").join("template")
}

pub fn ensure_default_presets() {
    write_defaults_if_dir_missing(&instruct_presets_dir(), BUILTIN_INSTRUCT);
    write_defaults_if_dir_missing(&reasoning_presets_dir(), BUILTIN_REASONING);
    write_defaults_if_dir_missing(&template_presets_dir(), BUILTIN_TEMPLATE);
}

pub(crate) fn write_defaults_if_dir_missing(dir: &Path, builtins: &[(&str, &str)]) {
    let already_existed = dir.exists();
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if already_existed {
        return;
    }
    for (name, json) in builtins {
        let path = dir.join(format!("{name}.json"));
        let _ = std::fs::write(&path, json);
    }
}

pub(crate) fn backward_compat_alias(name: &str) -> Option<&'static str> {
    match name {
        "llama2" | "mistral" => Some("Mistral V3-Tekken"),
        "chatml" => Some("ChatML"),
        "phi" => Some("Phi"),
        "alpaca" => Some("Alpaca"),
        _ => None,
    }
}

pub(crate) fn load_json_from_dir<T: serde::de::DeserializeOwned>(
    dir: &Path,
    name: &str,
) -> Option<T> {
    let entries = std::fs::read_dir(dir).ok()?;
    let name_lower = name.to_lowercase();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if stem == name_lower
                && let Ok(contents) = std::fs::read_to_string(&path)
            {
                return serde_json::from_str(&contents).ok();
            }
        }
    }

    None
}

pub(crate) fn list_json_names_in_dir(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json")
            && let Some(stem) = path.file_stem()
        {
            names.push(stem.to_string_lossy().to_string());
        }
    }
    names.sort();
    names
}
