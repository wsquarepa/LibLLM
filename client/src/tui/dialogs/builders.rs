//! Factory functions for constructing field editor dialogs with validation rules.

use super::FieldDialog;
use super::tabbed_field::{TabSection, TabbedFieldDialog};
use super::validation::FieldValidation;

pub(in crate::tui) const DIALOG_WIDTH_RATIO: f32 = 0.7;
pub(in crate::tui) const DIALOG_HEIGHT_RATIO: f32 = 0.6;
pub(in crate::tui) const LIST_DIALOG_WIDTH: u16 = 50;
pub(in crate::tui) const LIST_DIALOG_TALL_PADDING: u16 = 4;
pub(in crate::tui) const FIELD_DIALOG_DEFAULT_WIDTH: u16 = 60;

const GENERAL_LABELS: &[&str] = &[
    "API URL",
    "Template preset",
    "Instruct preset",
    "Reasoning preset",
    "TLS Skip Verify",
    "Debug Logging",
];
const GENERAL_BOOLEAN: &[usize] = &[4, 5];
const GENERAL_SELECTOR: &[usize] = &[1, 2, 3];

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
    "Context Size",
    "Trigger Threshold",
    "Prompt",
];
const SUMMARIZATION_BOOLEAN: &[usize] = &[0];
const SUMMARIZATION_MULTILINE: &[usize] = &[4];
const SUMMARIZATION_PLACEHOLDER: &[usize] = &[1];

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

const CHARACTER_EDITOR_FIELDS: &[&str] = &[
    "Name",
    "Description",
    "Personality",
    "Scenario",
    "First Message",
    "Examples",
    "System Prompt",
    "Post-History",
];
const CHARACTER_EDITOR_MULTILINE: &[usize] = &[1, 2, 3, 4, 5, 6, 7];

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

pub fn open_config_editor(
    sections: Vec<Vec<String>>,
    locked: Vec<Vec<usize>>,
) -> TabbedFieldDialog<'static> {
    let [general_vals, sampling_vals, backup_vals, summarization_vals]: [Vec<String>; 4] =
        sections.try_into().expect("expected 4 section vectors");
    let [general_locked, sampling_locked, backup_locked, summarization_locked]: [Vec<usize>; 4] =
        locked.try_into().expect("expected 4 lock vectors");

    let general = TabSection {
        title: "General",
        labels: GENERAL_LABELS,
        original_values: general_vals.clone(),
        values: general_vals,
        multiline_fields: &[],
        boolean_fields: GENERAL_BOOLEAN,
        selector_fields: GENERAL_SELECTOR,
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: &[],
        placeholder_text: None,
        locked_fields: general_locked,
        validated_fields: Vec::new(),
        color_preview_fields: &[],
        selected: 0,
    };

    let sampling = TabSection {
        title: "Sampling",
        labels: SAMPLING_LABELS,
        original_values: sampling_vals.clone(),
        values: sampling_vals,
        multiline_fields: &[],
        boolean_fields: &[],
        selector_fields: &[],
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: &[],
        placeholder_text: None,
        locked_fields: sampling_locked,
        validated_fields: vec![
            (0, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (1, FieldValidation::Int { min: 1, max: 100 }),
            (2, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (3, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (4, FieldValidation::Int { min: -1, max: 32767 }),
            (5, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (6, FieldValidation::Int { min: -1, max: 32767 }),
        ],
        color_preview_fields: &[],
        selected: 0,
    };

    let backup = TabSection {
        title: "Backup",
        labels: BACKUP_LABELS,
        original_values: backup_vals.clone(),
        values: backup_vals,
        multiline_fields: &[],
        boolean_fields: BACKUP_BOOLEAN,
        selector_fields: &[],
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: &[],
        placeholder_text: None,
        locked_fields: backup_locked,
        validated_fields: vec![
            (1, FieldValidation::Int { min: 0, max: 3650 }),
            (2, FieldValidation::Int { min: 0, max: 3650 }),
            (3, FieldValidation::Int { min: 0, max: 3650 }),
            (4, FieldValidation::Int { min: 0, max: 100 }),
            (5, FieldValidation::Int { min: 0, max: 100 }),
        ],
        color_preview_fields: &[],
        selected: 0,
    };

    let summarization = TabSection {
        title: "Summarization",
        labels: SUMMARIZATION_LABELS,
        original_values: summarization_vals.clone(),
        values: summarization_vals,
        multiline_fields: SUMMARIZATION_MULTILINE,
        boolean_fields: SUMMARIZATION_BOOLEAN,
        selector_fields: &[],
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: SUMMARIZATION_PLACEHOLDER,
        placeholder_text: Some("(inherit main api_url)"),
        locked_fields: summarization_locked,
        validated_fields: vec![
            (2, FieldValidation::Int { min: 512, max: 131072 }),
            (3, FieldValidation::Int { min: 1, max: 100 }),
        ],
        color_preview_fields: &[],
        selected: 0,
    };

    TabbedFieldDialog::new(
        " Configuration ",
        vec![general, sampling, backup, summarization],
    )
}

pub fn open_persona_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(" Edit Persona ", PERSONA_FIELDS, values, PERSONA_MULTILINE)
        .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_character_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Character ",
        CHARACTER_EDITOR_FIELDS,
        values,
        CHARACTER_EDITOR_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
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

const MESSAGES_LABELS: &[&str] = &[
    "user_message",
    "assistant_message_fg",
    "assistant_message_bg",
    "system_message",
    "dialogue",
];
const MESSAGES_COLOR_FIELDS: &[usize] = &[0, 1, 2, 3, 4];

const BORDERS_STATUS_LABELS: &[&str] = &[
    "border_focused",
    "border_unfocused",
    "status_bar_fg",
    "status_bar_bg",
    "status_error_fg",
    "status_error_bg",
    "status_info_fg",
    "status_info_bg",
    "status_warning_fg",
    "status_warning_bg",
];
const BORDERS_STATUS_COLOR_FIELDS: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9];

const UI_LABELS: &[&str] = &[
    "nav_cursor_fg",
    "nav_cursor_bg",
    "hover_bg",
    "sidebar_highlight_fg",
    "sidebar_highlight_bg",
    "dimmed",
    "command_picker_fg",
    "command_picker_bg",
];
const UI_COLOR_FIELDS: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7];

const INDICATORS_LABELS: &[&str] = &[
    "streaming_indicator",
    "api_unavailable",
    "summary_indicator",
];
const INDICATORS_COLOR_FIELDS: &[usize] = &[0, 1, 2];

pub fn open_theme_editor(config: &libllm::config::Config) -> TabbedFieldDialog<'static> {
    let overrides = config
        .theme_colors
        .as_ref()
        .cloned()
        .unwrap_or_default();

    let base_theme = config.theme.clone().unwrap_or_else(|| "dark".to_owned());

    let theme_vals = vec![
        base_theme,
        String::new(),
        String::new(),
        String::new(),
    ];
    let theme_tab = TabSection {
        title: "Theme",
        labels: THEME_TAB_LABELS,
        original_values: theme_vals.clone(),
        values: theme_vals,
        multiline_fields: &[],
        boolean_fields: &[],
        selector_fields: THEME_TAB_SELECTOR,
        action_fields: THEME_TAB_ACTIONS,
        separator_fields: THEME_TAB_SEPARATOR,
        placeholder_fields: &[],
        placeholder_text: None,
        locked_fields: Vec::new(),
        validated_fields: Vec::new(),
        color_preview_fields: &[],
        selected: 0,
    };

    let messages_vals: Vec<String> = MESSAGES_LABELS
        .iter()
        .map(|l| override_value(&overrides, l))
        .collect();
    let messages = TabSection {
        title: "Messages",
        labels: MESSAGES_LABELS,
        original_values: messages_vals.clone(),
        values: messages_vals,
        multiline_fields: &[],
        boolean_fields: &[],
        selector_fields: &[],
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: &[],
        placeholder_text: None,
        locked_fields: Vec::new(),
        validated_fields: color_validations(MESSAGES_LABELS.len()),
        color_preview_fields: MESSAGES_COLOR_FIELDS,
        selected: 0,
    };

    let borders_vals: Vec<String> = BORDERS_STATUS_LABELS
        .iter()
        .map(|l| override_value(&overrides, l))
        .collect();
    let borders_status = TabSection {
        title: "Borders & Status",
        labels: BORDERS_STATUS_LABELS,
        original_values: borders_vals.clone(),
        values: borders_vals,
        multiline_fields: &[],
        boolean_fields: &[],
        selector_fields: &[],
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: &[],
        placeholder_text: None,
        locked_fields: Vec::new(),
        validated_fields: color_validations(BORDERS_STATUS_LABELS.len()),
        color_preview_fields: BORDERS_STATUS_COLOR_FIELDS,
        selected: 0,
    };

    let ui_vals: Vec<String> = UI_LABELS
        .iter()
        .map(|l| override_value(&overrides, l))
        .collect();
    let ui_tab = TabSection {
        title: "UI",
        labels: UI_LABELS,
        original_values: ui_vals.clone(),
        values: ui_vals,
        multiline_fields: &[],
        boolean_fields: &[],
        selector_fields: &[],
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: &[],
        placeholder_text: None,
        locked_fields: Vec::new(),
        validated_fields: color_validations(UI_LABELS.len()),
        color_preview_fields: UI_COLOR_FIELDS,
        selected: 0,
    };

    let ind_vals: Vec<String> = INDICATORS_LABELS
        .iter()
        .map(|l| override_value(&overrides, l))
        .collect();
    let indicators = TabSection {
        title: "Indicators",
        labels: INDICATORS_LABELS,
        original_values: ind_vals.clone(),
        values: ind_vals,
        multiline_fields: &[],
        boolean_fields: &[],
        selector_fields: &[],
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: &[],
        placeholder_text: None,
        locked_fields: Vec::new(),
        validated_fields: color_validations(INDICATORS_LABELS.len()),
        color_preview_fields: INDICATORS_COLOR_FIELDS,
        selected: 0,
    };

    TabbedFieldDialog::new(
        " Theme ",
        vec![theme_tab, messages, borders_status, ui_tab, indicators],
    )
}

fn color_validations(count: usize) -> Vec<(usize, FieldValidation)> {
    (0..count).map(|i| (i, FieldValidation::Color)).collect()
}

fn override_value(overrides: &libllm::config::ThemeColorOverrides, label: &str) -> String {
    match label {
        "user_message" => overrides.user_message.clone().unwrap_or_default(),
        "assistant_message_fg" => overrides.assistant_message_fg.clone().unwrap_or_default(),
        "assistant_message_bg" => overrides.assistant_message_bg.clone().unwrap_or_default(),
        "system_message" => overrides.system_message.clone().unwrap_or_default(),
        "dialogue" => overrides.dialogue.clone().unwrap_or_default(),
        "border_focused" => overrides.border_focused.clone().unwrap_or_default(),
        "border_unfocused" => overrides.border_unfocused.clone().unwrap_or_default(),
        "status_bar_fg" => overrides.status_bar_fg.clone().unwrap_or_default(),
        "status_bar_bg" => overrides.status_bar_bg.clone().unwrap_or_default(),
        "status_error_fg" => overrides.status_error_fg.clone().unwrap_or_default(),
        "status_error_bg" => overrides.status_error_bg.clone().unwrap_or_default(),
        "status_info_fg" => overrides.status_info_fg.clone().unwrap_or_default(),
        "status_info_bg" => overrides.status_info_bg.clone().unwrap_or_default(),
        "status_warning_fg" => overrides.status_warning_fg.clone().unwrap_or_default(),
        "status_warning_bg" => overrides.status_warning_bg.clone().unwrap_or_default(),
        "nav_cursor_fg" => overrides.nav_cursor_fg.clone().unwrap_or_default(),
        "nav_cursor_bg" => overrides.nav_cursor_bg.clone().unwrap_or_default(),
        "hover_bg" => overrides.hover_bg.clone().unwrap_or_default(),
        "sidebar_highlight_fg" => overrides.sidebar_highlight_fg.clone().unwrap_or_default(),
        "sidebar_highlight_bg" => overrides.sidebar_highlight_bg.clone().unwrap_or_default(),
        "dimmed" => overrides.dimmed.clone().unwrap_or_default(),
        "command_picker_fg" => overrides.command_picker_fg.clone().unwrap_or_default(),
        "command_picker_bg" => overrides.command_picker_bg.clone().unwrap_or_default(),
        "streaming_indicator" => overrides.streaming_indicator.clone().unwrap_or_default(),
        "api_unavailable" => overrides.api_unavailable.clone().unwrap_or_default(),
        "summary_indicator" => overrides.summary_indicator.clone().unwrap_or_default(),
        _ => String::new(),
    }
}
