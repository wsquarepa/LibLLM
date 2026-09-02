//! Factory functions for constructing field editor dialogs with validation rules.

use super::FieldDialog;
use super::tabbed_field::{TabSection, TabbedFieldDialog};
use super::validation::FieldValidation;

pub(crate) const DIALOG_WIDTH_RATIO: f32 = 0.7;
pub(crate) const DIALOG_HEIGHT_RATIO: f32 = 0.6;
pub(crate) const LIST_DIALOG_WIDTH: u16 = 64;
pub(crate) const LIST_DIALOG_TALL_PADDING: u16 = 4;
pub(crate) const FIELD_DIALOG_DEFAULT_WIDTH: u16 = 60;

const GENERAL_LABELS: &[&str] = &[
    "API URL",
    "Authentication",
    "Template preset",
    "Instruct preset",
    "Reasoning preset",
    "TLS Skip Verify",
];
const GENERAL_BOOLEAN: &[usize] = &[5];
const GENERAL_SELECTOR: &[usize] = &[1, 2, 3, 4];

const SAMPLING_LABELS: &[&str] = &[
    "Temperature",
    "Top-K",
    "Top-P",
    "Min-P",
    "Repeat Last N",
    "Repeat Penalty",
    "Max Tokens",
];

const BACKUP_LABELS: &[&str] = &[
    "Enabled",
    "Keep All Days",
    "Keep Daily Days",
    "Keep Weekly Days",
    "Rebase Threshold %",
    "Rebase Hard Ceiling",
];
const BACKUP_BOOLEAN: &[usize] = &[0];

const SUMMARIZATION_LABELS: &[&str] = &[
    "Enabled",
    "API URL",
    "Max Context Size",
    "Trigger Threshold",
    "Keep Last Messages",
    "Prompt",
];
const SUMMARIZATION_BOOLEAN: &[usize] = &[0];
const SUMMARIZATION_MULTILINE: &[usize] = &[5];
const SUMMARIZATION_PLACEHOLDER: &[usize] = &[1];

const FILES_LABELS: &[&str] = &[
    "Enabled",
    "Per-file bytes",
    "Per-message bytes",
    "Summarize mode",
    "Summary prompt",
];
const FILES_BOOLEAN: &[usize] = &[0];
const FILES_MULTILINE: &[usize] = &[4];

const TEMPLATE_EDITOR_FIELDS: &[&str] =
    &["Name", "Story String", "Example Separator", "Chat Start"];
const TEMPLATE_EDITOR_MULTILINE: &[usize] = &[1];

const INSTRUCT_EDITOR_FIELDS: &[&str] = &[
    "Name",
    "Input Sequence",
    "Output Sequence",
    "System Sequence",
    "Input Suffix",
    "Output Suffix",
    "System Suffix",
    "Stop Sequence",
    "Separator Sequence",
    "Wrap",
    "System Same As User",
    "Seq. As Stop Strings",
];
const INSTRUCT_EDITOR_BOOLEAN: &[usize] = &[9, 10, 11];

const REASONING_EDITOR_FIELDS: &[&str] = &["Name", "Prefix", "Suffix", "Separator"];

const PERSONA_FIELDS: &[&str] = &["Name", "Persona"];
const PERSONA_MULTILINE: &[usize] = &[1];

const AUTHOR_NOTE_FIELDS: &[&str] = &["Note", "Depth", "Pin to top"];
const AUTHOR_NOTE_MULTILINE: &[usize] = &[0];
const AUTHOR_NOTE_BOOLEAN: &[usize] = &[2];

const CHARACTER_EDITOR_FIELDS: &[&str] = &[
    "Name",
    "Description",
    "Personality",
    "Scenario",
    "First Message",
    "Examples",
    "System Prompt",
    "Post-History",
    "Author's Note",
    "Author's Note Depth",
    "Pin Note to Top",
];
const CHARACTER_EDITOR_MULTILINE: &[usize] = &[1, 2, 3, 4, 5, 6, 7, 8];
const CHARACTER_EDITOR_BOOLEAN: &[usize] = &[10];

const SYSTEM_PROMPT_FIELDS: &[&str] = &["Name", "Content"];
const SYSTEM_PROMPT_MULTILINE: &[usize] = &[1];

const ENTRY_EDITOR_FIELDS: &[&str] = &[
    "Keys [OR]",
    "Content",
    "Selective",
    "Keys [AND]",
    "Constant",
    "Enabled",
    "Order",
    "Depth",
    "Case Sensitive",
];
const ENTRY_EDITOR_MULTILINE: &[usize] = &[1];
const ENTRY_EDITOR_PLACEHOLDER_FIELDS: &[usize] = &[0, 3];

#[expect(
    clippy::expect_used,
    reason = "sections and locked are built from exactly six config sections above"
)]
pub fn open_config_editor(
    sections: Vec<Vec<String>>,
    locked: Vec<Vec<usize>>,
) -> TabbedFieldDialog<'static> {
    let [
        general_vals,
        sampling_vals,
        backup_vals,
        summarization_vals,
        files_vals,
        _danger_vals,
    ]: [Vec<String>; 6] = sections.try_into().expect("expected 6 section vectors");
    let [
        general_locked,
        sampling_locked,
        backup_locked,
        summarization_locked,
        files_locked,
        _danger_locked,
    ]: [Vec<usize>; 6] = locked.try_into().expect("expected 6 lock vectors");

    let general = TabSection::new("General", GENERAL_LABELS, general_vals)
        .with_boolean_fields(GENERAL_BOOLEAN)
        .with_selector_fields(GENERAL_SELECTOR)
        .with_locked_fields(general_locked);

    let sampling = TabSection::new("Sampling", SAMPLING_LABELS, sampling_vals)
        .with_locked_fields(sampling_locked)
        .with_validated_fields(vec![
            (0, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (1, FieldValidation::Int { min: 1, max: 100 }),
            (2, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (3, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (
                4,
                FieldValidation::Int {
                    min: -1,
                    max: 32767,
                },
            ),
            (5, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (
                6,
                FieldValidation::Int {
                    min: -1,
                    max: 32767,
                },
            ),
        ]);

    let backup = TabSection::new("Backup", BACKUP_LABELS, backup_vals)
        .with_boolean_fields(BACKUP_BOOLEAN)
        .with_locked_fields(backup_locked)
        .with_validated_fields(vec![
            (1, FieldValidation::Int { min: 0, max: 3650 }),
            (2, FieldValidation::Int { min: 0, max: 3650 }),
            (3, FieldValidation::Int { min: 0, max: 3650 }),
            (4, FieldValidation::Int { min: 0, max: 100 }),
            (5, FieldValidation::Int { min: 0, max: 100 }),
        ]);

    let summarization = TabSection::new("Summarization", SUMMARIZATION_LABELS, summarization_vals)
        .with_multiline_fields(SUMMARIZATION_MULTILINE)
        .with_boolean_fields(SUMMARIZATION_BOOLEAN)
        .with_placeholder(SUMMARIZATION_PLACEHOLDER, "(inherit main api_url)")
        .with_locked_fields(summarization_locked)
        .with_validated_fields(vec![
            (
                2,
                FieldValidation::Int {
                    min: 512,
                    max: 131072,
                },
            ),
            (3, FieldValidation::Int { min: 1, max: 100 }),
            (4, FieldValidation::Int { min: 1, max: 100 }),
        ]);

    let files = TabSection::new("Files", FILES_LABELS, files_vals)
        .with_boolean_fields(FILES_BOOLEAN)
        .with_multiline_fields(FILES_MULTILINE)
        .with_locked_fields(files_locked)
        .with_validated_fields(vec![
            (
                1,
                FieldValidation::Int {
                    min: 0,
                    max: 134217728,
                },
            ),
            (
                2,
                FieldValidation::Int {
                    min: 0,
                    max: 134217728,
                },
            ),
        ]);

    let danger = TabSection::new("Danger", &[], vec![])
        .with_danger_style()
        .with_body_lines(9);

    TabbedFieldDialog::new(
        " Configuration ",
        vec![general, sampling, backup, summarization, files, danger],
    )
}

pub fn open_persona_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(" Edit Persona ", PERSONA_FIELDS, values, PERSONA_MULTILINE)
        .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_author_note_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Author's Note ",
        AUTHOR_NOTE_FIELDS,
        values,
        AUTHOR_NOTE_MULTILINE,
    )
    .with_boolean_fields(AUTHOR_NOTE_BOOLEAN)
    .with_validated_fields(vec![(1, FieldValidation::Int { min: 0, max: 999 })])
}

pub fn open_character_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Character ",
        CHARACTER_EDITOR_FIELDS,
        values,
        CHARACTER_EDITOR_MULTILINE,
    )
    .with_boolean_fields(CHARACTER_EDITOR_BOOLEAN)
    .with_validated_fields(vec![
        (0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH)),
        (9, FieldValidation::Int { min: 0, max: 999 }),
    ])
}

pub fn open_template_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Template Preset ",
        TEMPLATE_EDITOR_FIELDS,
        values,
        TEMPLATE_EDITOR_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_instruct_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Instruct Preset ",
        INSTRUCT_EDITOR_FIELDS,
        values,
        &[],
    )
    .with_boolean_fields(INSTRUCT_EDITOR_BOOLEAN)
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_reasoning_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Reasoning Preset ",
        REASONING_EDITOR_FIELDS,
        values,
        &[],
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_system_prompt_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit System Prompt ",
        SYSTEM_PROMPT_FIELDS,
        values,
        SYSTEM_PROMPT_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_entry_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Entry ",
        ENTRY_EDITOR_FIELDS,
        values,
        ENTRY_EDITOR_MULTILINE,
    )
    .with_placeholder("keyword1, keyword2, ...", ENTRY_EDITOR_PLACEHOLDER_FIELDS)
    .with_validated_fields(vec![
        (
            6,
            FieldValidation::Int {
                min: -999,
                max: 999,
            },
        ),
        (7, FieldValidation::Int { min: 0, max: 24 }),
    ])
}

pub fn open_entry_editor_non_selective(values: Vec<String>) -> FieldDialog<'static> {
    let mut dialog = open_entry_editor(values);
    dialog.hidden_fields = vec![3];
    dialog
}

const THEME_TAB_LABELS: &[&str] = &["Base theme", "", "Reset all colors", "Cancel"];
const THEME_TAB_SELECTOR: &[usize] = &[0];
const THEME_TAB_SEPARATOR: &[usize] = &[1];
const THEME_TAB_ACTIONS: &[usize] = &[2, 3];

pub(crate) const THEME_COLOR_TAB_LAYOUT: &[&[libllm_core::config::ColorLabel]] = &[
    MESSAGES_LABEL_IDS,
    BORDERS_STATUS_LABEL_IDS,
    UI_LABEL_IDS,
    INDICATORS_LABEL_IDS,
    GROUP_CHARACTER_LABEL_IDS,
];

const MESSAGES_LABEL_IDS: &[libllm_core::config::ColorLabel] = &[
    libllm_core::config::ColorLabel::UserCharacterFg,
    libllm_core::config::ColorLabel::UserCharacterBg,
    libllm_core::config::ColorLabel::SideCharacterFg,
    libllm_core::config::ColorLabel::SideCharacterBg,
    libllm_core::config::ColorLabel::AssistantMessageFg,
    libllm_core::config::ColorLabel::AssistantMessageBg,
    libllm_core::config::ColorLabel::SystemMessage,
    libllm_core::config::ColorLabel::Dialogue,
];
const MESSAGES_LABELS: &[&str] = &[
    libllm_core::config::ColorLabel::UserCharacterFg.name(),
    libllm_core::config::ColorLabel::UserCharacterBg.name(),
    libllm_core::config::ColorLabel::SideCharacterFg.name(),
    libllm_core::config::ColorLabel::SideCharacterBg.name(),
    libllm_core::config::ColorLabel::AssistantMessageFg.name(),
    libllm_core::config::ColorLabel::AssistantMessageBg.name(),
    libllm_core::config::ColorLabel::SystemMessage.name(),
    libllm_core::config::ColorLabel::Dialogue.name(),
];
const MESSAGES_COLOR_FIELDS: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7];

const BORDERS_STATUS_LABEL_IDS: &[libllm_core::config::ColorLabel] = &[
    libllm_core::config::ColorLabel::BorderFocused,
    libllm_core::config::ColorLabel::BorderUnfocused,
    libllm_core::config::ColorLabel::StatusBarFg,
    libllm_core::config::ColorLabel::StatusBarBg,
    libllm_core::config::ColorLabel::StatusErrorFg,
    libllm_core::config::ColorLabel::StatusErrorBg,
    libllm_core::config::ColorLabel::StatusInfoFg,
    libllm_core::config::ColorLabel::StatusInfoBg,
    libllm_core::config::ColorLabel::StatusWarningFg,
    libllm_core::config::ColorLabel::StatusWarningBg,
];
const BORDERS_STATUS_LABELS: &[&str] = &[
    libllm_core::config::ColorLabel::BorderFocused.name(),
    libllm_core::config::ColorLabel::BorderUnfocused.name(),
    libllm_core::config::ColorLabel::StatusBarFg.name(),
    libllm_core::config::ColorLabel::StatusBarBg.name(),
    libllm_core::config::ColorLabel::StatusErrorFg.name(),
    libllm_core::config::ColorLabel::StatusErrorBg.name(),
    libllm_core::config::ColorLabel::StatusInfoFg.name(),
    libllm_core::config::ColorLabel::StatusInfoBg.name(),
    libllm_core::config::ColorLabel::StatusWarningFg.name(),
    libllm_core::config::ColorLabel::StatusWarningBg.name(),
];
const BORDERS_STATUS_COLOR_FIELDS: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9];

const UI_LABEL_IDS: &[libllm_core::config::ColorLabel] = &[
    libllm_core::config::ColorLabel::NavCursorFg,
    libllm_core::config::ColorLabel::NavCursorBg,
    libllm_core::config::ColorLabel::HoverBg,
    libllm_core::config::ColorLabel::SidebarHighlightFg,
    libllm_core::config::ColorLabel::SidebarHighlightBg,
    libllm_core::config::ColorLabel::Dimmed,
    libllm_core::config::ColorLabel::CommandPickerFg,
    libllm_core::config::ColorLabel::CommandPickerBg,
    libllm_core::config::ColorLabel::SearchHighlightFg,
    libllm_core::config::ColorLabel::SearchHighlightBg,
];
const UI_LABELS: &[&str] = &[
    libllm_core::config::ColorLabel::NavCursorFg.name(),
    libllm_core::config::ColorLabel::NavCursorBg.name(),
    libllm_core::config::ColorLabel::HoverBg.name(),
    libllm_core::config::ColorLabel::SidebarHighlightFg.name(),
    libllm_core::config::ColorLabel::SidebarHighlightBg.name(),
    libllm_core::config::ColorLabel::Dimmed.name(),
    libllm_core::config::ColorLabel::CommandPickerFg.name(),
    libllm_core::config::ColorLabel::CommandPickerBg.name(),
    libllm_core::config::ColorLabel::SearchHighlightFg.name(),
    libllm_core::config::ColorLabel::SearchHighlightBg.name(),
];
const UI_COLOR_FIELDS: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9];

const INDICATORS_LABEL_IDS: &[libllm_core::config::ColorLabel] = &[
    libllm_core::config::ColorLabel::StreamingIndicator,
    libllm_core::config::ColorLabel::ApiUnavailable,
    libllm_core::config::ColorLabel::SummaryIndicator,
];
const INDICATORS_LABELS: &[&str] = &[
    libllm_core::config::ColorLabel::StreamingIndicator.name(),
    libllm_core::config::ColorLabel::ApiUnavailable.name(),
    libllm_core::config::ColorLabel::SummaryIndicator.name(),
];
const INDICATORS_COLOR_FIELDS: &[usize] = &[0, 1, 2];

const GROUP_CHARACTER_LABEL_IDS: &[libllm_core::config::ColorLabel] = &[
    libllm_core::config::ColorLabel::GroupCharacterFg1,
    libllm_core::config::ColorLabel::GroupCharacterFg2,
    libllm_core::config::ColorLabel::GroupCharacterFg3,
    libllm_core::config::ColorLabel::GroupCharacterFg4,
    libllm_core::config::ColorLabel::GroupCharacterFg5,
    libllm_core::config::ColorLabel::GroupCharacterFg6,
    libllm_core::config::ColorLabel::GroupCharacterFg7,
    libllm_core::config::ColorLabel::GroupCharacterFg8,
    libllm_core::config::ColorLabel::GroupCharacterBg1,
    libllm_core::config::ColorLabel::GroupCharacterBg2,
    libllm_core::config::ColorLabel::GroupCharacterBg3,
    libllm_core::config::ColorLabel::GroupCharacterBg4,
    libllm_core::config::ColorLabel::GroupCharacterBg5,
    libllm_core::config::ColorLabel::GroupCharacterBg6,
    libllm_core::config::ColorLabel::GroupCharacterBg7,
    libllm_core::config::ColorLabel::GroupCharacterBg8,
];
const GROUP_CHARACTER_LABELS: &[&str] = &[
    libllm_core::config::ColorLabel::GroupCharacterFg1.name(),
    libllm_core::config::ColorLabel::GroupCharacterFg2.name(),
    libllm_core::config::ColorLabel::GroupCharacterFg3.name(),
    libllm_core::config::ColorLabel::GroupCharacterFg4.name(),
    libllm_core::config::ColorLabel::GroupCharacterFg5.name(),
    libllm_core::config::ColorLabel::GroupCharacterFg6.name(),
    libllm_core::config::ColorLabel::GroupCharacterFg7.name(),
    libllm_core::config::ColorLabel::GroupCharacterFg8.name(),
    libllm_core::config::ColorLabel::GroupCharacterBg1.name(),
    libllm_core::config::ColorLabel::GroupCharacterBg2.name(),
    libllm_core::config::ColorLabel::GroupCharacterBg3.name(),
    libllm_core::config::ColorLabel::GroupCharacterBg4.name(),
    libllm_core::config::ColorLabel::GroupCharacterBg5.name(),
    libllm_core::config::ColorLabel::GroupCharacterBg6.name(),
    libllm_core::config::ColorLabel::GroupCharacterBg7.name(),
    libllm_core::config::ColorLabel::GroupCharacterBg8.name(),
];
const GROUP_CHARACTER_COLOR_FIELDS: &[usize] =
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

pub fn open_theme_editor(config: &libllm_core::config::Config) -> TabbedFieldDialog<'static> {
    let overrides = config.theme_colors.as_ref().cloned().unwrap_or_default();

    let base_theme = config.theme.clone().unwrap_or_else(|| "dark".to_owned());

    let theme_vals = vec![base_theme, String::new(), String::new(), String::new()];
    let theme_tab = TabSection::new("Theme", THEME_TAB_LABELS, theme_vals)
        .with_selector_fields(THEME_TAB_SELECTOR)
        .with_action_fields(THEME_TAB_ACTIONS)
        .with_separator_fields(THEME_TAB_SEPARATOR);

    let messages_vals: Vec<String> = MESSAGES_LABEL_IDS
        .iter()
        .map(|l| overrides.get(*l).unwrap_or_default().to_owned())
        .collect();
    let messages = TabSection::new("Messages", MESSAGES_LABELS, messages_vals)
        .with_validated_fields(color_validations(MESSAGES_LABELS.len()))
        .with_color_preview_fields(MESSAGES_COLOR_FIELDS);

    let borders_vals: Vec<String> = BORDERS_STATUS_LABEL_IDS
        .iter()
        .map(|l| overrides.get(*l).unwrap_or_default().to_owned())
        .collect();
    let borders_status = TabSection::new("Borders & Status", BORDERS_STATUS_LABELS, borders_vals)
        .with_validated_fields(color_validations(BORDERS_STATUS_LABELS.len()))
        .with_color_preview_fields(BORDERS_STATUS_COLOR_FIELDS);

    let ui_vals: Vec<String> = UI_LABEL_IDS
        .iter()
        .map(|l| overrides.get(*l).unwrap_or_default().to_owned())
        .collect();
    let ui_tab = TabSection::new("UI", UI_LABELS, ui_vals)
        .with_validated_fields(color_validations(UI_LABELS.len()))
        .with_color_preview_fields(UI_COLOR_FIELDS);

    let ind_vals: Vec<String> = INDICATORS_LABEL_IDS
        .iter()
        .map(|l| overrides.get(*l).unwrap_or_default().to_owned())
        .collect();
    let indicators = TabSection::new("Indicators", INDICATORS_LABELS, ind_vals)
        .with_validated_fields(color_validations(INDICATORS_LABELS.len()))
        .with_color_preview_fields(INDICATORS_COLOR_FIELDS);

    let group_chars_vals: Vec<String> = GROUP_CHARACTER_LABEL_IDS
        .iter()
        .map(|l| overrides.get(*l).unwrap_or_default().to_owned())
        .collect();
    let group_chars = TabSection::new("Group Characters", GROUP_CHARACTER_LABELS, group_chars_vals)
        .with_validated_fields(color_validations(GROUP_CHARACTER_LABELS.len()))
        .with_color_preview_fields(GROUP_CHARACTER_COLOR_FIELDS);

    TabbedFieldDialog::new(
        " Theme ",
        vec![
            theme_tab,
            messages,
            borders_status,
            ui_tab,
            indicators,
            group_chars,
        ],
    )
}

fn color_validations(count: usize) -> Vec<(usize, FieldValidation)> {
    (0..count).map(|i| (i, FieldValidation::Color)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_editor_group_character_tab_round_trips_overrides() {
        let overrides = libllm_core::config::ThemeColorOverrides {
            group_character_fg_1: Some("#112233".to_owned()),
            group_character_bg_8: Some("#445566".to_owned()),
            ..Default::default()
        };
        let config = libllm_core::config::Config {
            theme_colors: Some(overrides),
            ..libllm_core::config::Config::default()
        };
        let dialog = open_theme_editor(&config);
        let sections = dialog.sections();
        // sections[0] = Theme, [1] = Messages, [2] = Borders & Status, [3] = UI,
        // [4] = Indicators, [5] = Group Characters
        let group_tab = sections
            .iter()
            .find(|s| s.title == "Group Characters")
            .expect("Group Characters tab must exist in the theme editor");
        assert_eq!(
            group_tab.values[0], "#112233",
            "group_character_fg_1 must appear at index 0 of the Group Characters tab"
        );
        assert_eq!(
            group_tab.values[15], "#445566",
            "group_character_bg_8 must appear at index 15 of the Group Characters tab"
        );
    }
}
